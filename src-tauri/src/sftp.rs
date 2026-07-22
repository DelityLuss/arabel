use crate::ssh::{connect_auth, Handler};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use russh_sftp::client::SftpSession;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

/// Une session poolée par remote : (Handle SSH partagé, session SFTP). Le Handle est
/// en Arc pour être sorti du pool sous verrou bref (voir git_run). La connexion SSH
/// reste vivante tant que la session est dans le pool.
type Session = (Arc<russh::client::Handle<Handler>>, SftpSession);
pub struct SftpState(pub Mutex<HashMap<String, Session>>);

type Pool<'a> = tokio::sync::MutexGuard<'a, HashMap<String, Session>>;

#[allow(clippy::too_many_arguments)]
async fn ensure<'a>(
    state: &'a SftpState,
    remote_id: &str,
    host: &str,
    port: u16,
    user: &str,
    key_path: &str,
    identity_id: Option<String>,
    auth: Option<String>,
) -> Result<Pool<'a>, String> {
    let mut pool = state.0.lock().await;
    if !pool.contains_key(remote_id) {
        let handle = connect_auth(host, port, user, key_path, None, identity_id, auth).await?;
        let ch = handle
            .channel_open_session()
            .await
            .map_err(|e| e.to_string())?;
        ch.request_subsystem(true, "sftp")
            .await
            .map_err(|e| e.to_string())?;
        let sftp = SftpSession::new(ch.into_stream())
            .await
            .map_err(|e| format!("sftp: {e}"))?;
        pool.insert(remote_id.to_string(), (Arc::new(handle), sftp));
    }
    Ok(pool)
}

/// Ne jette la session poolée (pour la recréer) QUE si l'erreur signale une connexion
/// morte (I/O, timeout, désync protocolaire). Une erreur bénigne renvoyée par le
/// serveur (`Status` : permission refusée, fichier absent…) laisse la session SFTP et
/// la connexion SSH intactes — naviguer dans un dossier interdit ou ouvrir un fichier
/// absent ne doit pas tuer le remote et forcer une reconnexion.
fn drop_on_dead<T>(
    pool: &mut Pool<'_>,
    remote_id: &str,
    res: Result<T, russh_sftp::client::error::Error>,
) -> Result<T, String> {
    use russh_sftp::client::error::Error;
    res.map_err(|e| {
        if !matches!(e, Error::Status(_) | Error::Limited(_)) {
            pool.remove(remote_id);
        }
        e.to_string()
    })
}

#[tauri::command]
pub async fn sftp_home(
    state: State<'_, SftpState>,
    remote_id: String,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
) -> Result<String, String> {
    let mut pool = ensure(&state, &remote_id, &host, port, &user, &key_path, identity_id, auth).await?;
    let res = pool.get(&remote_id).unwrap().1.canonicalize(".").await;
    drop_on_dead(&mut pool, &remote_id, res)
}

#[tauri::command]
pub async fn sftp_list(
    state: State<'_, SftpState>,
    remote_id: String,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
    path: String,
) -> Result<Vec<serde_json::Value>, String> {
    let mut pool = ensure(&state, &remote_id, &host, port, &user, &key_path, identity_id, auth).await?;
    let res = pool.get(&remote_id).unwrap().1.read_dir(&path).await;
    let dir = drop_on_dead(&mut pool, &remote_id, res)?;
    Ok(dir
        .map(|e| {
            serde_json::json!({
                "name": e.file_name(),
                "isDir": e.file_type().is_dir(),
                "size": e.metadata().size.unwrap_or(0),
            })
        })
        .collect())
}

#[tauri::command]
pub async fn sftp_download(
    state: State<'_, SftpState>,
    remote_id: String,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
    path: String,
    name: String,
) -> Result<String, String> {
    let mut pool = ensure(&state, &remote_id, &host, port, &user, &key_path, identity_id, auth).await?;
    let sftp = &pool.get(&remote_id).unwrap().1;
    let res: Result<Vec<u8>, russh_sftp::client::error::Error> = async {
        let mut f = sftp.open(&path).await?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)
            .await
            .map_err(|e| russh_sftp::client::error::Error::IO(e.to_string()))?;
        Ok(buf)
    }
    .await;
    let buf = drop_on_dead(&mut pool, &remote_id, res)?;

    // `name` vient du serveur distant : on ne garde que le dernier segment,
    // sinon un nom « ../../.ssh/authorized_keys » écrirait hors de Downloads.
    let name = std::path::Path::new(&name)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != "..")
        .ok_or("nom de fichier invalide")?
        .to_string();
    // dossier Téléchargements par OS (~/Downloads, %USERPROFILE%\Downloads, XDG…)
    let dl = dirs::download_dir().ok_or("Downloads folder not found")?;
    let mut target = dl.join(&name);
    let mut n = 1;
    while target.exists() {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (name.clone(), String::new()),
        };
        target = dl.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    std::fs::write(&target, &buf).map_err(|e| e.to_string())?;
    Ok(target.display().to_string())
}

/// Colle une image du presse-papier local vers le VPS : écrit dans
/// `~/.arabel/paste/<name>` et renvoie le chemin ABSOLU distant (à insérer dans
/// le terminal pour que l'agent distant le lise). Borne le dossier à 30 fichiers.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn sftp_paste_image(
    state: State<'_, SftpState>,
    remote_id: String,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
    name: String,
    data_b64: String,
) -> Result<String, String> {
    let bytes = B64.decode(data_b64).map_err(|e| e.to_string())?;
    // nom sûr : dernier segment uniquement (le nom est généré côté app mais on
    // ne fait pas confiance aveugle à une entrée qui construit un chemin distant)
    let name = std::path::Path::new(&name)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != "..")
        .ok_or("nom de fichier invalide")?
        .to_string();
    let mut pool = ensure(&state, &remote_id, &host, port, &user, &key_path, identity_id, auth).await?;
    let sftp = &pool.get(&remote_id).unwrap().1;
    let res: Result<String, russh_sftp::client::error::Error> = async {
        let home = sftp.canonicalize(".").await?;
        let dir = format!("{home}/.arabel/paste");
        // mkdir -p best-effort : create_dir échoue si le dossier existe déjà, on ignore
        let _ = sftp.create_dir(format!("{home}/.arabel")).await;
        let _ = sftp.create_dir(&dir).await;
        let path = format!("{dir}/{name}");
        let mut f = sftp.create(&path).await?;
        f.write_all(&bytes)
            .await
            .map_err(|e| russh_sftp::client::error::Error::IO(e.to_string()))?;
        f.shutdown()
            .await
            .map_err(|e| russh_sftp::client::error::Error::IO(e.to_string()))?;
        // borne le dossier : noms préfixés par un timestamp → tri = ordre chrono,
        // on supprime les plus anciens au-delà de 30
        if let Ok(rd) = sftp.read_dir(&dir).await {
            let mut names: Vec<String> = rd.map(|e| e.file_name()).collect();
            if names.len() > 30 {
                names.sort();
                for old in &names[..names.len() - 30] {
                    let _ = sftp.remove_file(format!("{dir}/{old}")).await;
                }
            }
        }
        Ok(path)
    }
    .await;
    drop_on_dead(&mut pool, &remote_id, res)
}

#[tauri::command]
pub async fn sftp_upload(
    state: State<'_, SftpState>,
    remote_id: String,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
    path: String,
    data_b64: String,
) -> Result<String, String> {
    // ponytail: octets en base64 via l'IPC — OK pour des fichiers usuels,
    // passer en flux si besoin de très gros transferts
    let bytes = B64.decode(data_b64).map_err(|e| e.to_string())?;
    let mut pool = ensure(&state, &remote_id, &host, port, &user, &key_path, identity_id, auth).await?;
    let sftp = &pool.get(&remote_id).unwrap().1;
    let res: Result<String, russh_sftp::client::error::Error> = async {
        // pas d'écrasement silencieux : si le nom existe déjà, on suffixe « (n) » sur
        // le basename, comme le download côté local. ponytail: check-then-create =
        // course TOCTOU théorique, sans conséquence pour un usage mono-utilisateur.
        let (dir, base) = match path.rsplit_once('/') {
            Some((d, b)) => (d.to_string(), b.to_string()),
            None => (String::new(), path.clone()),
        };
        let (stem, ext) = match base.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (base.clone(), String::new()),
        };
        let mut final_path = path.clone();
        let mut n = 1;
        while sftp.metadata(&final_path).await.is_ok() {
            let nm = format!("{stem} ({n}){ext}");
            final_path = if dir.is_empty() { nm } else { format!("{dir}/{nm}") };
            n += 1;
        }
        let mut f = sftp.create(&final_path).await?;
        f.write_all(&bytes)
            .await
            .map_err(|e| russh_sftp::client::error::Error::IO(e.to_string()))?;
        f.shutdown()
            .await
            .map_err(|e| russh_sftp::client::error::Error::IO(e.to_string()))?;
        Ok(final_path.rsplit_once('/').map(|(_, b)| b).unwrap_or(&final_path).to_string())
    }
    .await;
    drop_on_dead(&mut pool, &remote_id, res)
}

/// Quote pour le shell distant : entoure de simples quotes, échappe les quotes.
fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Lance `git` dans `cwd` sur le remote (via le canal SSH poolé du SFTP).
/// Renvoie (code de sortie, stdout+stderr). Chaque argument est quoté.
// ponytail: suppose `git` dans le PATH de l'exec non-login ; si asdf/nvm masquent
// git, passer par `bash -lc`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn git_run(
    state: State<'_, SftpState>,
    remote_id: String,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
    cwd: String,
    args: Vec<String>,
) -> Result<(u32, String), String> {
    // On clone le Handle SSH sous verrou bref, puis on exécute SANS tenir le pool :
    // git ouvre son propre canal exec (multiplexage SSH), indépendant du canal SFTP.
    // Sinon un `git fetch` lent gelait le navigateur de fichiers ET tous les remotes.
    let handle = {
        let pool = ensure(&state, &remote_id, &host, port, &user, &key_path, identity_id, auth).await?;
        pool.get(&remote_id).unwrap().0.clone()
    };
    let cmd = format!(
        "git -C {} {}",
        shq(&cwd),
        args.iter().map(|a| shq(a)).collect::<Vec<_>>().join(" ")
    );
    let res = crate::ssh::exec(&handle, &cmd, None).await; // &Arc<Handle> → &Handle (deref)
    if res.is_err() {
        state.0.lock().await.remove(&remote_id); // canal mort : on invalide la session poolée
    }
    res
}

/// Cwd du pane tmux d'une session arabel (`#{pane_current_path}`) — pour ouvrir le
/// panneau git là où le terminal se trouve, sous tmux (le défaut). Passe par le
/// canal exec poolé, comme `git_run`. Chaîne vide si la session/tmux est absente →
/// l'appelant retombe sur le dossier configuré du remote.
// ponytail: renvoie le pane ACTIF de la session ; un split tmux manuel peut viser
// un autre pane que celui affiché. Suffisant pour le cas courant (1 pane/session).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn session_cwd(
    state: State<'_, SftpState>,
    remote_id: String,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    identity_id: Option<String>,
    auth: Option<String>,
    session: String,
) -> Result<String, String> {
    let handle = {
        let pool = ensure(&state, &remote_id, &host, port, &user, &key_path, identity_id, auth).await?;
        pool.get(&remote_id).unwrap().0.clone()
    };
    // exec non-login = PATH minimal ; tmux vit souvent dans /usr/local|/opt/homebrew.
    let cmd = format!(
        "PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin\" tmux display-message -p -t {} '#{{pane_current_path}}'",
        shq(&session)
    );
    let res = crate::ssh::exec(&handle, &cmd, None).await;
    if res.is_err() {
        state.0.lock().await.remove(&remote_id);
    }
    // tmux imprime le chemin sur stdout ; si la session manque, une erreur (ignorée
    // par l'appelant qui vérifie « commence par / »).
    Ok(res?.1.trim().to_string())
}
