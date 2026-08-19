use base64::{engine::general_purpose::STANDARD as B64, Engine};
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg};
use russh::{client, ChannelMsg};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{timeout, Duration};

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

pub fn expand_tilde(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .map(|h| h.join(rest).to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        None => path.to_string(),
    }
}

/// Le parseur de clé (russh/ssh-key) ne lit que le format OpenSSH (+ PKCS#8).
/// Une clé PuTTY, une clé PEM « legacy » (RSA/EC/DSA), ou un fichier de clé
/// PUBLIQUE échouent avec des erreurs cryptiques (« Der: trailing data… »). On
/// regarde l'entête du fichier pour rendre un message que l'utilisateur peut agir.
fn explain_key_error(path: &str, e: impl std::fmt::Display) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    classify_key_error(&text, path, &e.to_string())
}
fn classify_key_error(text: &str, path: &str, e: &str) -> String {
    let head = text.trim_start();
    let first = head.lines().next().unwrap_or("");
    if first.starts_with("PuTTY-User-Key-File") {
        format!("This is a PuTTY key. Convert it to OpenSSH: puttygen \"{path}\" -O private-openssh -o id_arabel — then point the identity to id_arabel.")
    } else if first.starts_with("ssh-") || first.starts_with("ecdsa-") || first.starts_with("sk-") {
        "That's a PUBLIC key. Point the identity to the PRIVATE key — the same file WITHOUT the .pub extension.".into()
    } else if head.contains("Proc-Type:")
        || head.contains("BEGIN RSA PRIVATE KEY")
        || head.contains("BEGIN EC PRIVATE KEY")
        || head.contains("BEGIN DSA PRIVATE KEY")
    {
        format!("This key is in the old PEM format, which isn't supported. Convert it in place (keeps the same key & passphrase): ssh-keygen -p -f \"{path}\" — or add it to ssh-agent (ssh-add) and use ssh-agent auth.")
    } else if head.contains("BEGIN ENCRYPTED PRIVATE KEY") {
        "This key needs its passphrase — set it on the identity in arabel, or use ssh-agent.".into()
    } else if !head.contains("BEGIN") {
        format!("Not a valid private key file: {path}")
    } else {
        format!("Couldn't read the key ({path}). If it has a passphrase, set it on the identity — or use ssh-agent. [{e}]")
    }
}

/// Traduit une erreur d'E/S réseau (résolution DNS / connexion TCP) en message
/// clair indiquant l'étape qui a échoué et une piste, plutôt qu'un `tokio io …`.
fn net_error(host: &str, port: u16, e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    let m = e.to_string().to_lowercase();
    if m.contains("resolve")
        || m.contains("nodename")
        || m.contains("not known")
        || m.contains("name or service")
        || m.contains("failed to lookup")
    {
        return format!("Can't resolve host \"{host}\" — check the hostname (DNS).");
    }
    match e.kind() {
        ConnectionRefused => format!("Connection refused by {host}:{port} — nothing is listening there (wrong port, or the SSH server is down)."),
        TimedOut => format!("Timed out reaching {host}:{port} — host down, wrong address, or a firewall is blocking the port."),
        _ => format!("Can't reach {host}:{port} — {e}."),
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
    // Étape 1 — résolution DNS + connexion TCP, avec timeout (12 s) pour ne pas
    // pendre sur un hôte injoignable. Erreurs réseau précises via net_error().
    let stream = match timeout(Duration::from_secs(12), TcpStream::connect((host, port))).await {
        Err(_) => {
            return Err(format!(
                "Timed out connecting to {host}:{port} — host unreachable, wrong address, or a firewall is blocking the port."
            ))
        }
        Ok(Err(e)) => return Err(net_error(host, port, &e)),
        Ok(Ok(s)) => s,
    };
    // Étape 2 — handshake SSH sur la connexion établie (protocole, clé serveur).
    let mut handle = client::connect_stream(config, stream, handler)
        .await
        .map_err(|e| match e {
            russh::Error::UnknownKey => {
                "The server's key changed since last time (possible MITM) — verify the host, then remove its line from ~/.ssh/known_hosts.".to_string()
            }
            e => format!("SSH handshake failed with {host}:{port} — {e}. Is this really an SSH server?"),
        })?;

    match auth.as_deref() {
        Some("agent") => auth_agent(&mut handle, user).await?,
        Some("password") => {
            // mot de passe explicite prioritaire, sinon Keychain via l'id passé
            let pw = passphrase
                .filter(|p| !p.is_empty())
                .or_else(|| identity_id.as_deref().and_then(crate::store::passphrase_get))
                .ok_or("password not found (Keychain)")?;
            let res = handle
                .authenticate_password(user, pw)
                .await
                .map_err(|e| format!("auth: {e}"))?;
            if !res.success() {
                return Err(format!("Wrong password for \"{user}\" (or the server doesn't allow password login)."));
            }
        }
        _ => {
            // passphrase explicite prioritaire, sinon Keychain via l'identité
            let passphrase = passphrase
                .filter(|p| !p.is_empty())
                .or_else(|| identity_id.as_deref().and_then(crate::store::passphrase_get));
            let path = expand_tilde(key_path);
            let key = load_secret_key(&path, passphrase.as_deref())
                .map_err(|e| explain_key_error(&path, e))?;
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
                return Err(format!("Server rejected the key for \"{user}\" — check the username, and that this key's public part is in ~/.ssh/authorized_keys on the server."));
            }
        }
    }
    Ok(handle)
}

/// Essaie chaque clé chargée dans le ssh-agent local.
async fn auth_agent(handle: &mut client::Handle<Handler>, user: &str) -> Result<(), String> {
    use russh::keys::agent::client::AgentClient;
    use russh::keys::agent::AgentIdentity;
    // Unix : socket pointé par SSH_AUTH_SOCK. Windows : l'agent OpenSSH vit sur un
    // named pipe fixe (connect_env n'existe pas côté Windows dans russh).
    // ponytail: chemin de pipe fixe ; ajouter fallback SSH_AUTH_SOCK/Pageant si un user Windows le réclame.
    #[cfg(unix)]
    let mut agent = AgentClient::connect_env()
        .await
        .map_err(|e| format!("ssh-agent: {e}"))?;
    #[cfg(windows)]
    let mut agent = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
        .await
        .map_err(|e| format!("ssh-agent: {e}"))?;
    let ids = agent.request_identities().await.map_err(|e| e.to_string())?;
    if ids.is_empty() {
        return Err("ssh-agent: no keys loaded (ssh-add?)".into());
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
    Err("ssh-agent: no key accepted by the server".into())
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

/// Ce VPS a-t-il déjà été équipé par arabel ? `~/.arabel/hook.sh` est notre
/// empreinte : seul `install_hooks` la pose, donc sa présence prouve qu'un
/// `claude_sync` a déjà réussi ici — depuis n'importe quelle machine.
///
/// Le drapeau `claude` d'un remote vit dans le store LOCAL de l'app, sur un
/// Remote dont l'id est un UUID tiré par machine : depuis un 2e poste, le même
/// VPS repartait « non équipé » alors que tout y était déjà. L'info n'existait
/// nulle part comme fait observable — seulement comme préférence déclarée. On
/// la redemande donc au VPS, seule source de vérité partagée par les postes.
///
/// On ne teste PAS `command -v claude` : `exec` ouvre un shell non-interactif,
/// qui ne lit pas ~/.bashrc — or c'est lui qui met ~/.local/bin (là où claude
/// s'installe) dans le PATH. Le test répondrait « non » sur un VPS pourtant
/// équipé, soit exactement le bug qu'on corrige.
#[tauri::command]
pub async fn claude_probe(
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
) -> Result<bool, String> {
    let handle = connect_auth(&host, port, &user, &key_path, None, identity_id, auth).await?;
    let (_, out) = exec(
        &handle,
        "test -x \"$HOME/.arabel/hook.sh\" && echo yes || echo no",
        None,
    )
    .await?;
    Ok(out.trim().lines().last() == Some("yes"))
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
    // Réglages arabel poussés dans le settings.json distant. Option<> : un appel
    // qui les omet garde le comportement d'avant (teams désactivées).
    agent_teams: Option<bool>,
    agent_team_panes: Option<bool>,
    // Relais de status line (bandeau contexte / quotas). Absent = on n'y touche
    // pas plus qu'avant : `false` retire le nôtre s'il traîne, sans plus.
    context_banner: Option<bool>,
) -> Result<String, String> {
    let home = crate::home_dir()?;
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
        return Err("nothing to sync in ~/.claude".into());
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
        "command -v claude >/dev/null 2>&1 && echo present || \
         (curl -fsSL https://claude.ai/install.sh | bash >/dev/null 2>&1 && echo installed || echo install-failed)",
        None,
    )
    .await?;
    let boot = boot.trim().lines().last().unwrap_or("?").to_string();

    let (status, out) = exec(&handle, "tar xzf - -C \"$HOME\"", Some(&tar.stdout)).await?;
    if status != 0 {
        return Err(format!("remote tar (code {status}): {out}"));
    }

    let teams = agent_teams.unwrap_or(false);
    let panes = agent_team_panes.unwrap_or(true);
    let hooks = install_hooks(&handle, teams, panes).await;
    let verbs = install_verb_cli(&handle).await;
    let plugins = provision_plugins(&handle, &home).await;
    // Après le tar : il vient d'écraser le settings.json distant par le nôtre,
    // qui porte le chemin de NOTRE ctx.sh mais pas la status line que le VPS
    // avait mise de côté. On rejoue donc le branchement ici.
    let banner = install_statusline(&handle, context_banner.unwrap_or(false)).await;

    let mut msg = format!(
        "claude {boot} · {} config item(s) pushed · {hooks} · {verbs} · {plugins} · {banner}",
        existing.len()
    );
    if teams {
        msg.push_str(if panes {
            " · agent teams (own panes)"
        } else {
            " · agent teams (in-process)"
        });
    }
    if let Some(w) = statusline_warning(&handle, &home).await {
        msg.push_str(&format!(" · {w}"));
    }
    Ok(msg)
}

/// Installe zsh-autosuggestions sur le VPS (suggestions grises en fin de frappe)
/// et le source dans ~/.zshrc. Idempotent : ré-exécutable sans effet de bord.
#[tauri::command]
pub async fn shell_enhance(
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
) -> Result<String, String> {
    // sh POSIX (pas zsh : on ne dépend que du shell de login pour installer).
    // git en priorité, sinon tarball via curl. La ligne source n'est ajoutée
    // qu'une fois (grep) et pointe le chemin absolu résolu.
    const SETUP: &str = r#"set -e
command -v zsh >/dev/null 2>&1 || { echo "zsh-absent"; exit 3; }
DIR="$HOME/.zsh/zsh-autosuggestions"
if [ ! -f "$DIR/zsh-autosuggestions.zsh" ]; then
  mkdir -p "$HOME/.zsh"
  if command -v git >/dev/null 2>&1; then
    git clone -q --depth=1 https://github.com/zsh-users/zsh-autosuggestions "$DIR"
  elif command -v curl >/dev/null 2>&1; then
    curl -fsSL https://github.com/zsh-users/zsh-autosuggestions/archive/refs/heads/master.tar.gz \
      | tar xz -C "$HOME/.zsh"
    mv "$HOME/.zsh/zsh-autosuggestions-master" "$DIR"
  else
    echo "no-fetch"; exit 4
  fi
fi
RC="$HOME/.zshrc"
grep -qF "zsh-autosuggestions.zsh" "$RC" 2>/dev/null || \
  printf '\n# arabel: suggestions\nsource %s/zsh-autosuggestions.zsh\n' "$DIR" >> "$RC"
[ "$(basename "${SHELL:-}")" = zsh ] && echo ok || echo "ok-not-default"
"#;
    let handle = connect_auth(&host, port, &user, &key_path, None, identity_id, auth).await?;
    match exec(&handle, "sh", Some(SETUP.as_bytes())).await? {
        (0, out) if out.trim() == "ok-not-default" => {
            Ok("suggestions installed — but your default shell isn't zsh (chsh -s $(which zsh)). Open a new terminal.".into())
        }
        (0, _) => Ok("suggestions enabled — open a new terminal.".into()),
        (3, _) => Err("zsh missing on the VPS (install it: apt/dnf install zsh)".into()),
        (4, _) => Err("neither git nor curl on the VPS to fetch the plugin".into()),
        (c, out) => Err(format!("failed (code {c}): {}", out.trim())),
    }
}

/// Installe le script de hook arabel et le branche dans ~/.claude/settings.json
/// distant ; y applique aussi les réglages « agent teams ».
async fn install_hooks(handle: &client::Handle<Handler>, teams: bool, panes: bool) -> String {
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
changed = False
# PreToolUse/UserPromptSubmit alimentent l'activité live du dashboard ;
# Notification/Stop portent l'attente (permission/question) et la fin.
for ev in ("Notification", "Stop", "PreToolUse", "UserPromptSubmit"):
    entries = h.setdefault(ev, [])
    if not any("arabel" in json.dumps(e) for e in entries):
        entries.append({"hooks": [cmd]}); changed = True

# Agent teams : la var d'env active la fonctionnalité, teammateMode décide si
# chaque coéquipier ouvre son propre panneau tmux (miroir dans la grille arabel
# en mode -CC) ou reste dans le pane du lead. Décoché = on RETIRE ce qu'on avait
# posé, sinon la case ne voudrait rien dire sur un VPS déjà équipé.
env = cfg.setdefault("env", {})
if TEAMS:
    if env.get("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS") != "1":
        env["CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"] = "1"; changed = True
    mode = "tmux" if PANES else "in-process"
    if cfg.get("teammateMode") != mode:
        cfg["teammateMode"] = mode; changed = True
else:
    if env.pop("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", None) is not None: changed = True
    if cfg.pop("teammateMode", None) is not None: changed = True
if not env:
    cfg.pop("env", None)

os.makedirs(os.path.dirname(p), exist_ok=True)
json.dump(cfg, open(p, "w"), indent=2)
print("installed" if changed else "present")
"#;
    let w = exec(
        handle,
        "mkdir -p ~/.arabel && cat > ~/.arabel/hook.sh && chmod +x ~/.arabel/hook.sh",
        Some(HOOK_SH.as_bytes()),
    )
    .await;
    if !matches!(w, Ok((0, _))) {
        return "hooks not installed (script write failed)".into();
    }
    // Les valeurs sont injectées par un prélude : `format!` sur MERGE_PY buterait
    // sur les accolades des dicts python.
    let py = format!(
        "TEAMS = {}\nPANES = {}\n{MERGE_PY}",
        if teams { "True" } else { "False" },
        if panes { "True" } else { "False" },
    );
    match exec(handle, "python3 -", Some(py.as_bytes())).await {
        Ok((0, out)) if out.contains("present") => "hooks already in place".into(),
        Ok((0, _)) => "notification hooks installed".into(),
        _ => "hooks not wired up (python3 missing on the VPS?)".into(),
    }
}

/// Pousse (ou retire) le relais de status line qui alimente le bandeau
/// « contexte & quotas » — voir `ctx.rs` pour le pourquoi du procédé.
///
/// `enable == false` n'est pas un no-op : il DÉSINSTALLE. Décocher le réglage
/// doit défaire ce qu'on a posé sur le VPS, sinon la case ne voudrait rien dire
/// sur une machine déjà équipée (même raison que « agent teams »).
async fn install_statusline(handle: &client::Handle<Handler>, enable: bool) -> String {
    if enable {
        let w = exec(
            handle,
            "mkdir -p ~/.arabel && cat > ~/.arabel/ctx.sh && chmod +x ~/.arabel/ctx.sh",
            Some(crate::ctx::CTX_SH.as_bytes()),
        )
        .await;
        if !matches!(w, Ok((0, _))) {
            return "context banner off (script write failed)".into();
        }
    }
    // Prélude injecté : `format!` sur le script buterait sur les accolades python.
    let py = format!(
        "ENABLE = {}\nOURS = {:?}\n{}",
        if enable { "True" } else { "False" },
        crate::ctx::STATUSLINE_CMD,
        crate::ctx::MERGE_PY,
    );
    match exec(handle, "python3 -", Some(py.as_bytes())).await {
        Ok((0, out)) if out.contains("unchanged") => {
            if enable { "context banner already wired" } else { "no context banner" }.into()
        }
        Ok((0, _)) => {
            if enable { "context banner wired (status line)" } else { "context banner removed" }.into()
        }
        _ => "context banner not wired (python3 missing on the VPS?)".into(),
    }
}

/// Branche/débranche le relais sur un VPS déjà équipé, sans repasser par la
/// grosse sync : c'est ce que fait la case à cocher des réglages.
#[tauri::command]
pub async fn ctx_remote_setup(
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
    enable: bool,
) -> Result<String, String> {
    let handle = connect_auth(&host, port, &user, &key_path, None, identity_id, auth).await?;
    Ok(install_statusline(&handle, enable).await)
}

/// Installe `~/.arabel/arabel` : le CLI que l'agent appelle pour piloter l'app
/// (`~/.arabel/arabel preview 3000`), et documente les verbes dans le CLAUDE.md
/// distant pour qu'il sache qu'ils existent.
///
/// Aucun PATH à câbler : le CLAUDE.md donne le chemin absolu. C'est ce qui le fait
/// marcher aussi dans les panneaux `tmux -CC`, qui n'héritent pas de l'init du pane.
async fn install_verb_cli(handle: &client::Handle<Handler>) -> String {
    // Le script n'exécute RIEN : il imprime une séquence OSC sur son propre
    // terminal. L'app la reçoit par le PTY de CE panneau — c'est ce qui lui dit
    // d'où vient l'ordre, sans aucune variable d'environnement.
    const VERB_SH: &str = r#"#!/bin/sh
[ $# -gt 0 ] || { echo "usage: arabel preview <port>" >&2; exit 2; }
seq="\033]7770;$*\007"
# tmux n'achemine pas les OSC qu'il ne connaît pas : emballage passthrough, ESC
# du contenu doublé (allow-passthrough est déjà posé par arabel).
[ -n "$TMUX" ] && seq="\033Ptmux;\033$seq\033\\\\"
printf '%b' "$seq"
"#;
    // Le tar de claude_sync vient d'écraser le CLAUDE.md distant par la copie
    // locale : on ré-ajoute la section après, et le grep garantit qu'il n'y en a
    // qu'une (même motif que shell_enhance avec .zshrc).
    const DOC_SH: &str = r#"set -e
MD="$HOME/.claude/CLAUDE.md"
mkdir -p "$HOME/.claude"
grep -qF "arabel preview" "$MD" 2>/dev/null && exit 0
cat >> "$MD" <<'EOF'

## arabel terminal
This terminal is arabel, a desktop app on the user's machine. You can drive it:

- `~/.arabel/arabel preview <port>` — open a browser pane on that port of THIS
  server. The tunnel is set up automatically. Run it right after starting a dev
  server, so the user sees the page without leaving the terminal.

Two more things work here that a plain terminal cannot do:
- `printf '\033]0;title\007'` sets this pane's title (use it to say what you are doing).
- OSC 52 copies to the user's local clipboard.
EOF
"#;
    let w = exec(
        handle,
        "mkdir -p ~/.arabel && cat > ~/.arabel/arabel && chmod +x ~/.arabel/arabel",
        Some(VERB_SH.as_bytes()),
    )
    .await;
    if !matches!(w, Ok((0, _))) {
        return "verbs not installed (script write failed)".into();
    }
    match exec(handle, "sh", Some(DOC_SH.as_bytes())).await {
        Ok((0, _)) => "verbs ready (arabel preview)".into(),
        _ => "verbs installed (CLAUDE.md not documented)".into(),
    }
}

/// Plugins activés localement (clés `nom@marketplace` de settings.json).
fn read_enabled_plugins(home: &str) -> Vec<String> {
    let txt = match std::fs::read_to_string(std::path::Path::new(home).join(".claude/settings.json")) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    serde_json::from_str::<serde_json::Value>(&txt)
        .ok()
        .and_then(|v| v.get("enabledPlugins")?.as_object().map(|o| {
            o.iter()
                .filter(|(_, v)| v.as_bool() == Some(true))
                .map(|(k, _)| k.clone())
                .collect()
        }))
        .unwrap_or_default()
}

/// Sources des marketplaces connues localement (repo github ou url), pour
/// `claude plugin marketplace add`.
fn read_marketplaces(home: &str) -> Vec<String> {
    let txt = match std::fs::read_to_string(
        std::path::Path::new(home).join(".claude/plugins/known_marketplaces.json"),
    ) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    serde_json::from_str::<serde_json::Value>(&txt)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.values()
                    .filter_map(|m| {
                        let src = m.get("source")?;
                        match src.get("source").and_then(|s| s.as_str()) {
                            Some("github") => src.get("repo").and_then(|r| r.as_str()),
                            _ => src.get("url").and_then(|u| u.as_str()),
                        }
                        .map(String::from)
                    })
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Réinstalle les plugins activés depuis leurs marketplaces (git) sur le VPS —
/// plutôt que de copier le cache local (binaires/chemins Mac non portables).
/// C'est le duo `marketplace add` + `install` fait automatiquement.
async fn provision_plugins(handle: &client::Handle<Handler>, home: &str) -> String {
    let plugins = read_enabled_plugins(home);
    if plugins.is_empty() {
        return "no enabled plugin".into();
    }
    // valeurs issues de la config locale de l'utilisateur (repos github, noms de
    // plugins) : simples, mais on quote quand même pour éviter toute surprise.
    let mut script = String::from("set +e\n");
    for repo in read_marketplaces(home) {
        script.push_str(&format!("claude plugin marketplace add '{repo}' >/dev/null 2>&1\n"));
    }
    for p in &plugins {
        script.push_str(&format!("claude plugin install '{p}' >/dev/null 2>&1\n"));
    }
    script.push_str("claude plugin list 2>/dev/null\n");
    let out = match exec(handle, "sh", Some(script.as_bytes())).await {
        Ok((_, out)) => out,
        Err(e) => return format!("plugins not installed ({e})"),
    };
    let ok = plugins
        .iter()
        .filter(|p| out.contains(p.split('@').next().unwrap_or(p)))
        .count();
    format!("{ok}/{} plugin(s) installed", plugins.len())
}

/// Détecte si la statusline configurée sera cassée sur le VPS : chemin absolu
/// propre à la machine locale, ou dépendance (bun) absente à distance. On avertit
/// seulement — on ne réécrit pas la config de l'utilisateur.
async fn statusline_warning(handle: &client::Handle<Handler>, home: &str) -> Option<String> {
    let txt = std::fs::read_to_string(std::path::Path::new(home).join(".claude/settings.json")).ok()?;
    let cmd = serde_json::from_str::<serde_json::Value>(&txt)
        .ok()?
        .get("statusLine")?
        .get("command")?
        .as_str()?
        .to_string();
    let mut issues: Vec<String> = vec![];
    if cmd.contains(home) {
        issues.push(format!("local path {home}"));
    }
    if cmd.contains("bun")
        && !matches!(
            exec(handle, "command -v bun >/dev/null 2>&1 && echo y || echo n", None).await,
            Ok((_, o)) if o.trim() == "y"
        )
    {
        issues.push("bun missing on VPS".into());
    }
    (!issues.is_empty()).then(|| format!("⚠ status line needs adjusting ({})", issues.join(", ")))
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

/// Contexte & quotas des panes claude de ce VPS, relus toutes les 3 s.
///
/// Un `tail -F` serait plus fin, mais le relais RÉÉCRIT un fichier par pane au
/// lieu d'empiler des lignes (sinon le fichier grossirait à chaque rendu de
/// status line, plusieurs fois par seconde). On relit donc, comme les métriques.
/// Le ménage d'entrée jette les panes d'une session d'app révolue.
#[tauri::command]
pub async fn ctx_watch(
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
        format!("ctx:{remote_id}"),
        remote_id,
        "arabel-ctx",
        r#"mkdir -p ~/.arabel/ctx; find ~/.arabel/ctx -name '*.json' -mtime +0 -delete 2>/dev/null; \
           while :; do for f in ~/.arabel/ctx/*.json; do [ -f "$f" ] && { cat "$f"; echo; }; done; sleep 3; done"#,
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
pub async fn ctx_unwatch(state: State<'_, WatchState>, remote_id: String) -> Result<(), String> {
    watch_stop(&state, &format!("ctx:{remote_id}")).await;
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

/// Étend `~/`, chemins absolus, ou relatifs à `~/.ssh` (sémantique `Include` d'OpenSSH).
fn ssh_expand(home: &str, p: &str) -> std::path::PathBuf {
    if let Some(r) = p.strip_prefix("~/") {
        std::path::Path::new(home).join(r)
    } else if p.starts_with('/') {
        std::path::PathBuf::from(p)
    } else {
        std::path::Path::new(home).join(".ssh").join(p)
    }
}

/// Extrait une valeur string d'un JSON(C) par recherche textuelle (settings.json de VS Code
/// contient des commentaires, donc serde_json ne suffit pas).
fn jsonc_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let after = &text[text.find(&needle)? + needle.len()..];
    let a2 = &after[after.find(':')? + 1..];
    let q1 = a2.find('"')?;
    let q2 = a2[q1 + 1..].find('"')?;
    Some(a2[q1 + 1..q1 + 1 + q2].to_string())
}

/// Idem pour une valeur numérique (ex. `"terminal.integrated.fontSize": 13`).
fn jsonc_number(text: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{key}\"");
    let after = &text[text.find(&needle)? + needle.len()..];
    let a2 = after[after.find(':')? + 1..].trim_start();
    let end = a2.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-').unwrap_or(a2.len());
    a2[..end].parse().ok()
}

/// Emplacements possibles du settings.json de VS Code (stable/Insiders/VSCodium).
/// `dirs::config_dir()` pointe le bon dossier par OS : `~/Library/Application Support`
/// (macOS), `%APPDATA%` (Windows), `~/.config` (Linux) — là où VS Code range sa config.
fn vscode_settings_files() -> Vec<std::path::PathBuf> {
    let Some(base) = dirs::config_dir() else { return Vec::new() };
    ["Code", "Code - Insiders", "VSCodium"]
        .iter()
        .map(|b| base.join(b).join("User").join("settings.json"))
        .collect()
}

/// Lit un réglage de VS Code (stable ou Insiders).
fn vscode_setting(key: &str) -> Option<String> {
    for p in vscode_settings_files() {
        if let Ok(text) = std::fs::read_to_string(&p) {
            if let Some(v) = jsonc_string(&text, key) {
                return Some(v);
            }
        }
    }
    None
}

fn push_host(
    alias: &Option<String>, hostname: &Option<String>, user: &Option<String>,
    port: &Option<u16>, identity: &Option<String>, out: &mut Vec<serde_json::Value>,
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

/// Parse un fichier de config SSH (gère `Include`, récursif, protégé contre les boucles).
fn parse_ssh_file(
    path: &std::path::Path, home: &str,
    out: &mut Vec<serde_json::Value>, seen: &mut std::collections::HashSet<std::path::PathBuf>,
) {
    if !seen.insert(path.to_path_buf()) {
        return; // déjà lu (évite les Include circulaires)
    }
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let (mut alias, mut hostname, mut user, mut port, mut identity): (
        Option<String>, Option<String>, Option<String>, Option<u16>, Option<String>,
    ) = (None, None, None, None, None);
    let sep = |c: char| c.is_whitespace() || c == '=';
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(sep) else { continue };
        let key = k.to_ascii_lowercase();
        let val = v.trim_start_matches(sep).trim().to_string();
        match key.as_str() {
            "include" => {
                push_host(&alias, &hostname, &user, &port, &identity, out);
                (alias, hostname, user, port, identity) = (None, None, None, None, None);
                for token in val.split_whitespace() {
                    let ip = ssh_expand(home, token);
                    if token.contains('*') {
                        // glob simple : lit tous les fichiers du dossier
                        if let Some(dir) = ip.parent() {
                            if let Ok(rd) = std::fs::read_dir(dir) {
                                let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
                                files.sort();
                                for f in files {
                                    parse_ssh_file(&f, home, out, seen);
                                }
                            }
                        }
                    } else {
                        parse_ssh_file(&ip, home, out, seen);
                    }
                }
            }
            "host" => {
                push_host(&alias, &hostname, &user, &port, &identity, out);
                (hostname, user, port, identity) = (None, None, None, None);
                alias = val.split_whitespace().find(|p| !p.contains('*') && !p.contains('?')).map(str::to_string);
            }
            "hostname" => hostname = Some(val),
            "user" => user = Some(val),
            "port" => port = val.parse().ok(),
            "identityfile" => identity = Some(val),
            _ => {}
        }
    }
    push_host(&alias, &hostname, &user, &port, &identity, out);
}

/// Parse ~/.ssh/config (+ Include) et le fichier SSH configuré dans VS Code.
#[tauri::command]
pub fn ssh_config_parse() -> Result<Vec<serde_json::Value>, String> {
    let home = crate::home_dir()?;
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    parse_ssh_file(&std::path::Path::new(&home).join(".ssh/config"), &home, &mut out, &mut seen);
    // VS Code Remote-SSH peut pointer un autre fichier
    if let Some(cfg) = vscode_setting("remote.SSH.configFile") {
        parse_ssh_file(&ssh_expand(&home, &cfg), &home, &mut out, &mut seen);
    }
    // dédoublonne par (host)
    let mut names = std::collections::HashSet::new();
    out.retain(|h| names.insert(h["host"].as_str().unwrap_or("").to_string()));
    Ok(out)
}

/// Police et taille du terminal configurées dans VS Code (pour import).
#[tauri::command]
pub fn vscode_terminal() -> serde_json::Value {
    let (mut family, mut size) = (None, None);
    for p in vscode_settings_files() {
        if let Ok(text) = std::fs::read_to_string(&p) {
            if family.is_none() {
                family = jsonc_string(&text, "terminal.integrated.fontFamily");
            }
            if size.is_none() {
                size = jsonc_number(&text, "terminal.integrated.fontSize");
            }
        }
    }
    serde_json::json!({ "fontFamily": family, "fontSize": size })
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
        return Err("tunnel already active".into());
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

#[cfg(test)]
mod tests {
    use super::{jsonc_number, jsonc_string};
    #[test]
    fn jsonc_extract() {
        let s = r#"{
          // commentaire style VS Code
          "terminal.integrated.fontFamily": "JetBrains Mono, monospace",
          "terminal.integrated.fontSize": 13,
          "remote.SSH.configFile": "~/.ssh/vscode_config"
        }"#;
        assert_eq!(jsonc_string(s, "terminal.integrated.fontFamily").as_deref(), Some("JetBrains Mono, monospace"));
        assert_eq!(jsonc_string(s, "remote.SSH.configFile").as_deref(), Some("~/.ssh/vscode_config"));
        assert_eq!(jsonc_number(s, "terminal.integrated.fontSize"), Some(13.0));
        assert_eq!(jsonc_string(s, "absent"), None);
        assert_eq!(jsonc_number(s, "absent"), None);
    }
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

#[cfg(test)]
mod key_tests {
    use super::{classify_key_error, net_error};
    fn msg(text: &str) -> String {
        classify_key_error(text, "/k", "Der: trailing data")
    }
    #[test]
    fn classifies_network_errors() {
        use std::io::{Error, ErrorKind};
        assert!(net_error("h", 22, &Error::new(ErrorKind::ConnectionRefused, "x")).contains("refused"));
        assert!(net_error("h", 22, &Error::new(ErrorKind::TimedOut, "x")).contains("Timed out"));
        assert!(net_error("bad", 22, &Error::new(ErrorKind::Other, "failed to lookup address information")).contains("resolve"));
        assert!(net_error("h", 2222, &Error::new(ErrorKind::Other, "weird")).contains("2222"));
    }
    #[test]
    fn classifies_key_formats() {
        assert!(msg("ssh-ed25519 AAAAC3Nz... user@host").contains("PUBLIC key"));
        assert!(msg("PuTTY-User-Key-File-3: ssh-ed25519").contains("PuTTY"));
        assert!(msg("-----BEGIN RSA PRIVATE KEY-----\n...").contains("old PEM"));
        assert!(msg("-----BEGIN EC PRIVATE KEY-----\n...").contains("old PEM"));
        assert!(msg("Proc-Type: 4,ENCRYPTED\n...").contains("old PEM"));
        assert!(msg("-----BEGIN ENCRYPTED PRIVATE KEY-----").contains("passphrase"));
        assert!(msg("garbage not a key").contains("Not a valid private key"));
        // format OpenSSH (chiffré, passphrase manquante) → repli, on ne mésclassifie pas
        assert!(msg("-----BEGIN OPENSSH PRIVATE KEY-----\n...").contains("passphrase"));
    }
}
