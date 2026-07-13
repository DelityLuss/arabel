mod fonts;
mod local;
mod sftp;
pub mod ssh;
mod store;

use ssh::SshState;
use std::collections::HashMap;
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::Emitter;
use tokio::sync::Mutex;

fn build_menu(app: &tauri::App) -> tauri::Result<()> {
    let h = app.handle();
    let app_menu = Submenu::with_items(
        h,
        "Arabel",
        true,
        &[
            &PredefinedMenuItem::about(h, None, Some(AboutMetadata::default()))?,
            &PredefinedMenuItem::separator(h)?,
            &MenuItem::with_id(h, "settings", "Réglages…", true, Some("Cmd+,"))?,
            &PredefinedMenuItem::separator(h)?,
            &PredefinedMenuItem::hide(h, None)?,
            &PredefinedMenuItem::hide_others(h, None)?,
            &PredefinedMenuItem::separator(h)?,
            &PredefinedMenuItem::quit(h, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        h,
        "Édition",
        true,
        &[
            &PredefinedMenuItem::undo(h, None)?,
            &PredefinedMenuItem::redo(h, None)?,
            &PredefinedMenuItem::separator(h)?,
            &PredefinedMenuItem::cut(h, None)?,
            &PredefinedMenuItem::copy(h, None)?,
            &PredefinedMenuItem::paste(h, None)?,
            &PredefinedMenuItem::select_all(h, None)?,
        ],
    )?;
    let shell = Submenu::with_items(
        h,
        "Shell",
        true,
        &[
            &MenuItem::with_id(h, "new-connection", "Nouvelle connexion…", true, Some("Cmd+N"))?,
            &MenuItem::with_id(h, "close-pane", "Fermer le panneau", true, Some("Cmd+W"))?,
            &PredefinedMenuItem::separator(h)?,
            &MenuItem::with_id(h, "split-h", "Diviser à droite", true, Some("Cmd+D"))?,
            &MenuItem::with_id(h, "split-v", "Diviser en dessous", true, Some("Cmd+Shift+D"))?,
            &PredefinedMenuItem::separator(h)?,
            &MenuItem::with_id(h, "clear", "Effacer le terminal", true, Some("Cmd+K"))?,
            &MenuItem::with_id(h, "sync-config", "Injecter la config Claude", true, None::<&str>)?,
        ],
    )?;
    let view = Submenu::with_items(
        h,
        "Présentation",
        true,
        &[
            &MenuItem::with_id(h, "toggle-sidebar", "Barre latérale", true, Some("Cmd+B"))?,
            &PredefinedMenuItem::separator(h)?,
            &PredefinedMenuItem::fullscreen(h, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        h,
        "Fenêtre",
        true,
        &[
            &PredefinedMenuItem::minimize(h, None)?,
            &PredefinedMenuItem::maximize(h, None)?,
        ],
    )?;
    let menu = Menu::with_items(h, &[&app_menu, &edit, &shell, &view, &window])?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, ev| {
        let _ = app.emit("menu", ev.id().0.clone());
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .manage(SshState(Mutex::new(HashMap::new())))
        .manage(ssh::WatchState(Mutex::new(HashMap::new())))
        .manage(ssh::ForwardState(Mutex::new(HashMap::new())))
        .manage(sftp::SftpState(Mutex::new(HashMap::new())))
        .setup(|app| {
            build_menu(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            local::local_connect,
            local::local_metrics,
            ssh::metrics_watch,
            ssh::metrics_unwatch,
            ssh::ssh_connect,
            ssh::ssh_write,
            ssh::ssh_resize,
            ssh::ssh_disconnect,
            ssh::claude_sync,
            ssh::events_watch,
            ssh::events_unwatch,
            ssh::ssh_config_parse,
            ssh::port_forward_start,
            ssh::port_forward_stop,
            sftp::sftp_home,
            sftp::sftp_list,
            sftp::sftp_download,
            sftp::sftp_upload,
            store::store_load,
            store::store_save,
            store::passphrase_set,
            store::passphrase_delete,
            fonts::list_fonts
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
