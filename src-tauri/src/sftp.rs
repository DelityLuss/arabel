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
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let dl = std::path::Path::new(&home).join("Downloads");
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
