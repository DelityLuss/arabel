use base64::{engine::general_purpose::STANDARD as B64, Engine};
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg};
use russh::{client, ChannelMsg};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex};

pub struct SshState(pub Mutex<HashMap<String, mpsc::Sender<Cmd>>>);

pub enum Cmd {
    Write(Vec<u8>),
    Resize(u32, u32),
    Close,
}

pub struct Handler {
    host: String,
    port: u16,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    // TOFU via ~/.ssh/known_hosts (partagé avec ssh/OpenSSH) :
    // connu+identique → ok ; inconnu → on apprend ; clé changée → refus (MITM possible)
    async fn check_server_key(
        &mut self,
        key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        use russh::keys::known_hosts::{check_known_hosts, learn_known_hosts};
        match check_known_hosts(&self.host, self.port, key) {
            Ok(true) => Ok(true),
            Ok(false) => {
                learn_known_hosts(&self.host, self.port, key)?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}

fn expand_tilde(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => format!("{}/{}", std::env::var("HOME").unwrap_or_default(), rest),
        None => path.to_string(),
    }
}

/// `auth` : "agent" (ssh-agent), "password" (mot de passe dans le Keychain via
/// `identity_id`), sinon clé privée (comportement historique).
#[allow(clippy::too_many_arguments)]
pub async fn connect_auth(
    host: &str,
    port: u16,
    user: &str,
    key_path: &str,
    passphrase: Option<String>,
    identity_id: Option<String>,
    auth: Option<String>,
) -> Result<client::Handle<Handler>, String> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(std::time::Duration::from_secs(15)),
        keepalive_max: 3,
        ..Default::default()
    });
    let handler = Handler {
        host: host.to_string(),
        port,
    };
    let mut handle = client::connect(config, (host, port), handler)
        .await
        .map_err(|e| match e {
            russh::Error::UnknownKey => {
                "la clé du serveur a changé (risque de MITM) — vérifie ~/.ssh/known_hosts".to_string()
            }
            e => format!("connexion: {e}"),
        })?;

    match auth.as_deref() {
        Some("agent") => auth_agent(&mut handle, user).await?,
        Some("password") => {
            // mot de passe explicite prioritaire, sinon Keychain via l'id passé
            let pw = passphrase
                .filter(|p| !p.is_empty())
                .or_else(|| identity_id.as_deref().and_then(crate::store::passphrase_get))
                .ok_or("mot de passe introuvable (Keychain)")?;
            let res = handle
                .authenticate_password(user, pw)
                .await
                .map_err(|e| format!("auth: {e}"))?;
            if !res.success() {
                return Err("mot de passe refusé".into());
            }
        }
        _ => {
            // passphrase explicite prioritaire, sinon Keychain via l'identité
            let passphrase = passphrase
                .filter(|p| !p.is_empty())
                .or_else(|| identity_id.as_deref().and_then(crate::store::passphrase_get));
            let key = load_secret_key(expand_tilde(key_path), passphrase.as_deref())
                .map_err(|e| format!("clé: {e}"))?;
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(|e| e.to_string())?
                .flatten();
            let res = handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(|e| format!("auth: {e}"))?;
            if !res.success() {
                return Err("authentification refusée".into());
            }
        }
    }
    Ok(handle)
}

/// Essaie chaque clé chargée dans le ssh-agent local.
async fn auth_agent(handle: &mut client::Handle<Handler>, user: &str) -> Result<(), String> {
    use russh::keys::agent::client::AgentClient;
    use russh::keys::agent::AgentIdentity;
    let mut agent = AgentClient::connect_env()
        .await
        .map_err(|e| format!("ssh-agent: {e}"))?;
    let ids = agent.request_identities().await.map_err(|e| e.to_string())?;
    if ids.is_empty() {
        return Err("ssh-agent : aucune clé chargée (ssh-add ?)".into());
    }
    let hash = handle.best_supported_rsa_hash().await.ok().flatten().flatten();
    for id in ids {
        let key = match id {
            AgentIdentity::PublicKey { key, .. } => key,
            _ => continue, // certificats non gérés
        };
        let res = handle
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
            .map_err(|e| e.to_string())?;
        if res.success() {
            return Ok(());
        }
    }
    Err("ssh-agent : aucune clé acceptée par le serveur".into())
}

#[tauri::command]
pub async fn ssh_connect(
    app: AppHandle,
    state: State<'_, SshState>,
    session_id: String,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    passphrase: Option<String>,
    identity_id: Option<String>,
    auth: Option<String>,
    cols: u32,
    rows: u32,
    exec_cmd: Option<String>,
) -> Result<(), String> {
    let handle = connect_auth(&host, port, &user, &key_path, passphrase, identity_id, auth).await?;

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| e.to_string())?;
    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .map_err(|e| e.to_string())?;
    // commande de démarrage (ex. tmux new -A) sinon shell de connexion
    match exec_cmd {
        Some(cmd) => channel
            .exec(true, cmd.as_str())
            .await
            .map_err(|e| e.to_string())?,
        None => channel
            .request_shell(true)
            .await
            .map_err(|e| e.to_string())?,
    }

    let (tx, mut rx) = mpsc::channel::<Cmd>(64);
    let tx_id = tx.clone();
    state.0.lock().await.insert(session_id.clone(), tx);

    let out_event = format!("ssh-output-{session_id}");
    let closed_event = format!("ssh-closed-{session_id}");
    let sid = session_id.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                msg = channel.wait() => match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        let _ = app.emit(&out_event, B64.encode(data));
                    }
                    Some(ChannelMsg::ExitStatus { .. }) | Some(ChannelMsg::Close) | None => break,
                    _ => {}
                },
                cmd = rx.recv() => match cmd {
                    Some(Cmd::Write(bytes)) => { let _ = channel.data(&bytes[..]).await; }
                    Some(Cmd::Resize(c, r)) => { let _ = channel.window_change(c, r, 0, 0).await; }
                    Some(Cmd::Close) | None => break,
                },
            }
        }
        let _ = app.emit(&closed_event, ());
        // ne retire que si l'entrée est toujours la nôtre : une reconnexion sur
        // le même session_id a pu réinsérer un autre canal entre-temps
        let st = app.state::<SshState>();
        let mut map = st.0.lock().await;
        if map.get(&sid).is_some_and(|cur| cur.same_channel(&tx_id)) {
            map.remove(&sid);
        }
        drop(map);
        // handle est déplacé ici pour garder la connexion vivante jusqu'à la fin
        drop(handle);
    });

    Ok(())
}

/// Exécute une commande distante, renvoie (exit status, sortie stdout+stderr).
pub async fn exec(
    handle: &client::Handle<Handler>,
    cmd: &str,
    stdin: Option<&[u8]>,
) -> Result<(u32, String), String> {
    let mut ch = handle
        .channel_open_session()
        .await
        .map_err(|e| e.to_string())?;
    ch.exec(true, cmd).await.map_err(|e| e.to_string())?;
    if let Some(data) = stdin {
        ch.data(data).await.map_err(|e| e.to_string())?;
    }
    ch.eof().await.map_err(|e| e.to_string())?;
    let (mut out, mut status) = (String::new(), 0u32);
    while let Some(msg) = ch.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => out.push_str(&String::from_utf8_lossy(data)),
            ChannelMsg::ExtendedData { ref data, .. } => {
                out.push_str(&String::from_utf8_lossy(data))
            }
            ChannelMsg::ExitStatus { exit_status } => status = exit_status,
            _ => {}
        }
    }
    Ok((status, out))
}

/// Pousse la config locale ~/.claude vers le VPS et installe claude si absent.
#[tauri::command]
pub async fn claude_sync(
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
) -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    // ponytail: whitelist explicite — ~/.claude contient aussi l'historique de
    // sessions et des caches qu'il ne faut surtout pas pousser
    const ITEMS: [&str; 5] = [
        ".claude/settings.json",
        ".claude/CLAUDE.md",
        ".claude/commands",
        ".claude/agents",
        ".claude/skills",
    ];
    let existing: Vec<&str> = ITEMS
        .iter()
        .copied()
        .filter(|p| std::path::Path::new(&home).join(p).exists())
        .collect();
    if existing.is_empty() {
        return Err("aucun élément à synchroniser dans ~/.claude".into());
    }
    let tar = tokio::process::Command::new("tar")
        .arg("czf")
        .arg("-")
        .arg("-C")
        .arg(&home)
        .args(&existing)
        .output()
        .await
        .map_err(|e| format!("tar local: {e}"))?;
    if !tar.status.success() {
        return Err(format!(
            "tar local: {}",
            String::from_utf8_lossy(&tar.stderr)
        ));
    }

    let handle = connect_auth(&host, port, &user, &key_path, None, identity_id, auth).await?;

    let (_, boot) = exec(
        &handle,
        "command -v claude >/dev/null 2>&1 && echo présent || \
         (curl -fsSL https://claude.ai/install.sh | bash >/dev/null 2>&1 && echo installé || echo échec-installation)",
        None,
    )
    .await?;
    let boot = boot.trim().lines().last().unwrap_or("?").to_string();

    let (status, out) = exec(&handle, "tar xzf - -C \"$HOME\"", Some(&tar.stdout)).await?;
    if status != 0 {
        return Err(format!("tar distant (code {status}): {out}"));
    }

    let hooks = install_hooks(&handle).await;

    Ok(format!(
        "claude {boot} · {} élément(s) de config poussés · {hooks}",
        existing.len()
    ))
}

/// Installe le script de hook arabel et le branche dans ~/.claude/settings.json distant.
async fn install_hooks(handle: &client::Handle<Handler>) -> String {
    const HOOK_SH: &str = r#"#!/bin/sh
mkdir -p "$HOME/.arabel"
printf '{"pane":"%s","event":%s}\n' "$ARABEL_PANE" "$(cat | tr -d '\n')" >> "$HOME/.arabel/events.jsonl"
"#;
    const MERGE_PY: &str = r#"
import json, os
p = os.path.expanduser("~/.claude/settings.json")
try:
    cfg = json.load(open(p))
except Exception:
    cfg = {}
cmd = {"type": "command", "command": "$HOME/.arabel/hook.sh"}
h = cfg.setdefault("hooks", {})
for ev in ("Notification", "Stop"):
    entries = h.setdefault(ev, [])
    if not any("arabel" in json.dumps(e) for e in entries):
        entries.append({"hooks": [cmd]})
os.makedirs(os.path.dirname(p), exist_ok=True)
json.dump(cfg, open(p, "w"), indent=2)
print("ok")
"#;
    let w = exec(
        handle,
        "mkdir -p ~/.arabel && cat > ~/.arabel/hook.sh && chmod +x ~/.arabel/hook.sh",
        Some(HOOK_SH.as_bytes()),
    )
    .await;
    if !matches!(w, Ok((0, _))) {
        return "hooks non installés (écriture script échouée)".into();
    }
    match exec(handle, "python3 -", Some(MERGE_PY.as_bytes())).await {
        Ok((0, _)) => "hooks de notification installés".into(),
        _ => "hooks non branchés (python3 absent sur le VPS ?)".into(),
    }
}

/// Flux distants (hooks Claude, métriques) : une commande streamée par clé,
/// chaque ligne devient un event Tauri { remoteId, line }.
pub struct WatchState(pub Mutex<HashMap<String, mpsc::Sender<()>>>);

#[allow(clippy::too_many_arguments)]
async fn watch_stream(
    app: AppHandle,
    key: String,
    remote_id: String,
    event: &'static str,
    remote_cmd: &str,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
) -> Result<(), String> {
    // réservation atomique : check + insert sous le même verrou. Sans ça, deux
    // panneaux vers le même remote passent tous deux le test avant d'insérer et
    // ouvrent chacun un flux distant — l'un devient orphelin (tail -F / boucle
    // métriques qui tourne à jamais sur le VPS, impossible à arrêter).
    let (tx, mut rx) = mpsc::channel::<()>(1);
    {
        let state = app.state::<WatchState>();
        let mut guard = state.0.lock().await;
        if guard.contains_key(&key) {
            return Ok(()); // déjà en veille
        }
        guard.insert(key.clone(), tx.clone());
    }
    let setup = async {
        let handle = connect_auth(&host, port, &user, &key_path, None, identity_id, auth).await?;
        let ch = handle
            .channel_open_session()
            .await
            .map_err(|e| e.to_string())?;
        ch.exec(true, remote_cmd).await.map_err(|e| e.to_string())?;
        Ok::<_, String>((handle, ch))
    };
    let (handle, mut ch) = match setup.await {
        Ok(v) => v,
        Err(e) => {
            // la connexion a échoué : on libère la réservation
            app.state::<WatchState>().0.lock().await.remove(&key);
            return Err(e);
        }
    };
    tauri::async_runtime::spawn(async move {
        let mut buf = String::new();
        loop {
            tokio::select! {
                msg = ch.wait() => match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        buf.push_str(&String::from_utf8_lossy(data));
                        while let Some(i) = buf.find('\n') {
                            let line = buf[..i].to_string();
                            buf.drain(..=i);
                            if !line.trim().is_empty() {
                                let _ = app.emit(
                                    event,
                                    serde_json::json!({ "remoteId": remote_id, "line": line }),
                                );
                            }
                        }
                    }
                    Some(ChannelMsg::Close) | None => break,
                    _ => {}
                },
                _ = rx.recv() => break,
            }
        }
        // ne retire que notre propre entrée (idem ssh_connect)
        let st = app.state::<WatchState>();
        let mut map = st.0.lock().await;
        if map.get(&key).is_some_and(|cur| cur.same_channel(&tx)) {
            map.remove(&key);
        }
        drop(map);
        drop(handle);
    });
    Ok(())
}

async fn watch_stop(state: &WatchState, key: &str) {
    if let Some(tx) = state.0.lock().await.get(key).cloned() {
        let _ = tx.send(()).await;
    }
}

#[tauri::command]
pub async fn events_watch(
    app: AppHandle,
    remote_id: String,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
) -> Result<(), String> {
    watch_stream(
        app,
        format!("hook:{remote_id}"),
        remote_id,
        "arabel-hook",
        "mkdir -p ~/.arabel && touch ~/.arabel/events.jsonl && exec tail -F -n0 ~/.arabel/events.jsonl",
        host,
        port,
        user,
        key_path,
        identity_id,
        auth,
    )
    .await
}

#[tauri::command]
pub async fn events_unwatch(state: State<'_, WatchState>, remote_id: String) -> Result<(), String> {
    watch_stop(&state, &format!("hook:{remote_id}")).await;
    Ok(())
}

/// Échantillonne load/CPU/RAM/disque du VPS toutes les 3 s (Linux).
#[tauri::command]
pub async fn metrics_watch(
    app: AppHandle,
    remote_id: String,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
) -> Result<(), String> {
    watch_stream(
        app,
        format!("metrics:{remote_id}"),
        remote_id,
        "arabel-metrics",
        r#"while :; do echo "M $(cut -d' ' -f1 /proc/loadavg) $(nproc) $(free -m | awk '/^Mem:/{print $2" "$3}') $(df -Pm / | awk 'NR==2{print $2" "$3}')"; sleep 3; done"#,
        host,
        port,
        user,
        key_path,
        identity_id,
        auth,
    )
    .await
}

#[tauri::command]
pub async fn metrics_unwatch(
    state: State<'_, WatchState>,
    remote_id: String,
) -> Result<(), String> {
    watch_stop(&state, &format!("metrics:{remote_id}")).await;
    Ok(())
}

/// Parse ~/.ssh/config et renvoie les hôtes déclarés (motifs génériques ignorés).
#[tauri::command]
pub fn ssh_config_parse() -> Result<Vec<serde_json::Value>, String> {
    fn push(
        alias: &Option<String>,
        hostname: &Option<String>,
        user: &Option<String>,
        port: &Option<u16>,
        identity: &Option<String>,
        out: &mut Vec<serde_json::Value>,
    ) {
        if let Some(a) = alias {
            out.push(serde_json::json!({
                "host": a,
                "hostName": hostname.clone().unwrap_or_else(|| a.clone()),
                "user": user.clone().unwrap_or_default(),
                "port": port.unwrap_or(22),
                "identityFile": identity.clone().unwrap_or_default(),
            }));
        }
    }
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(std::path::Path::new(&home).join(".ssh/config")).unwrap_or_default();
    let mut out = Vec::new();
    let (mut alias, mut hostname, mut user, mut port, mut identity): (
        Option<String>, Option<String>, Option<String>, Option<u16>, Option<String>,
    ) = (None, None, None, None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let sep = |c: char| c.is_whitespace() || c == '=';
        let (key, val) = match line.split_once(sep) {
            Some((k, v)) => (k.to_ascii_lowercase(), v.trim_start_matches(sep).trim().to_string()),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                push(&alias, &hostname, &user, &port, &identity, &mut out);
                (hostname, user, port, identity) = (None, None, None, None);
                // premier alias non générique du bloc
                alias = val
                    .split_whitespace()
                    .find(|p| !p.contains('*') && !p.contains('?'))
                    .map(str::to_string);
            }
            "hostname" => hostname = Some(val),
            "user" => user = Some(val),
            "port" => port = val.parse().ok(),
            "identityfile" => identity = Some(val),
            _ => {}
        }
    }
    push(&alias, &hostname, &user, &port, &identity, &mut out);
    Ok(out)
}

/// Redirections de ports (tunnels type `ssh -L`) : un listener local par tunnel,
/// chaque connexion entrante ouvre un canal direct-tcpip vers l'hôte distant.
pub struct ForwardState(pub Mutex<HashMap<String, mpsc::Sender<()>>>);

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn port_forward_start(
    state: State<'_, ForwardState>,
    id: String,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
) -> Result<u16, String> {
    if state.0.lock().await.contains_key(&id) {
        return Err("tunnel déjà actif".into());
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", local_port))
        .await
        .map_err(|e| format!("port local {local_port} : {e}"))?;
    let actual = listener.local_addr().map_err(|e| e.to_string())?.port();
    let handle = Arc::new(connect_auth(&host, port, &user, &key_path, None, identity_id, auth).await?);
    let (tx, mut rx) = mpsc::channel::<()>(1);
    state.0.lock().await.insert(id.clone(), tx);
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    let Ok((mut sock, _)) = accept else { break };
                    let handle = handle.clone();
                    let rhost = remote_host.clone();
                    tauri::async_runtime::spawn(async move {
                        let ch = match handle
                            .channel_open_direct_tcpip(rhost, remote_port as u32, "127.0.0.1", 0)
                            .await
                        {
                            Ok(c) => c,
                            Err(_) => return, // service distant injoignable : on abandonne la socket
                        };
                        let mut stream = ch.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut sock, &mut stream).await;
                    });
                }
                _ = rx.recv() => break,
            }
        }
        // Arc<handle> lâché ici → la connexion SSH dédiée au tunnel se ferme
    });
    Ok(actual)
}

#[tauri::command]
pub async fn port_forward_stop(state: State<'_, ForwardState>, id: String) -> Result<(), String> {
    if let Some(tx) = state.0.lock().await.remove(&id) {
        let _ = tx.send(()).await;
    }
    Ok(())
}

async fn send(state: &SshState, session_id: &str, cmd: Cmd) -> Result<(), String> {
    let tx = state
        .0
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or("session inconnue")?;
    tx.send(cmd).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ssh_write(
    state: State<'_, SshState>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    send(&state, &session_id, Cmd::Write(data.into_bytes())).await
}

#[tauri::command]
pub async fn ssh_resize(
    state: State<'_, SshState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    send(&state, &session_id, Cmd::Resize(cols, rows)).await
}

#[tauri::command]
pub async fn ssh_disconnect(
    state: State<'_, SshState>,
    session_id: String,
) -> Result<(), String> {
    send(&state, &session_id, Cmd::Close).await
}
