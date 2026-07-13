use crate::ssh::{Cmd, SshState};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;

/// Métriques de la machine locale (load, CPU, RAM, disque) — même forme que
/// la ligne "M …" des métriques distantes, mais en JSON direct.
#[tauri::command]
pub fn local_metrics() -> serde_json::Value {
    use sysinfo::{Disks, System};
    let mut sys = System::new();
    sys.refresh_memory();
    let load = System::load_average().one;
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let disks = Disks::new_with_refreshed_list();
    let (disk_total, disk_used) = disks
        .list()
        .iter()
        .find(|d| d.mount_point() == std::path::Path::new("/"))
        .map(|d| (d.total_space(), d.total_space() - d.available_space()))
        .unwrap_or((0, 0));
    serde_json::json!({
        "load": load,
        "cpus": cpus,
        "memTotal": sys.total_memory(),
        "memUsed": sys.used_memory(),
        "diskTotal": disk_total,
        "diskUsed": disk_used,
    })
}

fn size(cols: u32, rows: u32) -> PtySize {
    PtySize {
        rows: rows as u16,
        cols: cols as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Terminal local : shell de l'utilisateur dans un PTY, mêmes events et mêmes
/// commandes write/resize/disconnect que les sessions SSH (via SshState).
#[tauri::command]
pub async fn local_connect(
    app: AppHandle,
    state: State<'_, SshState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let pty = native_pty_system();
    let pair = pty.openpty(size(cols, rows)).map_err(|e| e.to_string())?;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.arg("-l");
    cmd.env("TERM", "xterm-256color");
    if let Ok(home) = std::env::var("HOME") {
        cmd.cwd(home);
    }
    let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let master = pair.master;

    let (tx, mut rx) = mpsc::channel::<Cmd>(64);
    state.0.lock().await.insert(session_id.clone(), tx);

    // lecture bloquante → thread dédié
    let out_event = format!("ssh-output-{session_id}");
    let closed_event = format!("ssh-closed-{session_id}");
    let app_r = app.clone();
    let sid = session_id.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = app_r.emit(&out_event, B64.encode(&buf[..n]));
                }
            }
        }
        let _ = app_r.emit(&closed_event, ());
        tauri::async_runtime::block_on(async {
            app_r.state::<SshState>().0.lock().await.remove(&sid);
        });
    });

    // contrôle write/resize/close
    tauri::async_runtime::spawn(async move {
        while let Some(c) = rx.recv().await {
            match c {
                Cmd::Write(bytes) => {
                    let _ = writer.write_all(&bytes);
                }
                Cmd::Resize(cols, rows) => {
                    let _ = master.resize(size(cols, rows));
                }
                Cmd::Close => break,
            }
        }
        let _ = child.kill(); // le reader voit EOF et émet ssh-closed
    });

    Ok(())
}
