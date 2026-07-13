use crate::ssh::{connect_auth, Handler};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use russh_sftp::client::SftpSession;
use std::collections::HashMap;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

/// Une session SFTP poolée par remote (la connexion SSH reste vivante avec).
pub struct SftpState(pub Mutex<HashMap<String, (russh::client::Handle<Handler>, SftpSession)>>);

type Pool<'a> = tokio::sync::MutexGuard<'a, HashMap<String, (russh::client::Handle<Handler>, SftpSession)>>;

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
        pool.insert(remote_id.to_string(), (handle, sftp));
    }
    Ok(pool)
}

/// Sur erreur d'opération on jette la session poolée (elle sera recréée).
fn drop_on_err<T, E: std::fmt::Display>(
    pool: &mut Pool<'_>,
    remote_id: &str,
    res: Result<T, E>,
) -> Result<T, String> {
    res.map_err(|e| {
        pool.remove(remote_id);
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
    drop_on_err(&mut pool, &remote_id, res)
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
    let dir = drop_on_err(&mut pool, &remote_id, res)?;
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
    let res: Result<Vec<u8>, String> = async {
        let mut f = sftp.open(&path).await.map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
        Ok(buf)
    }
    .await;
    let buf = drop_on_err(&mut pool, &remote_id, res)?;

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
    let res: Result<String, String> = async {
        let home = sftp.canonicalize(".").await.map_err(|e| e.to_string())?;
        let dir = format!("{home}/.arabel/paste");
        // mkdir -p best-effort : create_dir échoue si le dossier existe déjà, on ignore
        let _ = sftp.create_dir(format!("{home}/.arabel")).await;
        let _ = sftp.create_dir(&dir).await;
        let path = format!("{dir}/{name}");
        let mut f = sftp.create(&path).await.map_err(|e| e.to_string())?;
        f.write_all(&bytes).await.map_err(|e| e.to_string())?;
        f.shutdown().await.map_err(|e| e.to_string())?;
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
    drop_on_err(&mut pool, &remote_id, res)
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
) -> Result<(), String> {
    // ponytail: octets en base64 via l'IPC — OK pour des fichiers usuels,
    // passer en flux si besoin de très gros transferts
    let bytes = B64.decode(data_b64).map_err(|e| e.to_string())?;
    let mut pool = ensure(&state, &remote_id, &host, port, &user, &key_path, identity_id, auth).await?;
    let sftp = &pool.get(&remote_id).unwrap().1;
    let res: Result<(), String> = async {
        let mut f = sftp.create(&path).await.map_err(|e| e.to_string())?;
        f.write_all(&bytes).await.map_err(|e| e.to_string())?;
        f.shutdown().await.map_err(|e| e.to_string())?;
        Ok(())
    }
    .await;
    drop_on_err(&mut pool, &remote_id, res)
}
