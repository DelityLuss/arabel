mod fonts;
mod local;
mod sftp;
pub mod ssh;
mod store;

use ssh::SshState;
use std::collections::HashMap;
#[cfg(target_os = "macos")]
use tauri::menu::AboutMetadata;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::Emitter;
use tokio::sync::Mutex;

/// Répertoire personnel de l'utilisateur en `String` : `HOME` sur Unix,
/// `%USERPROFILE%` sur Windows (via `dirs`, pas `env::var("HOME")` qui est vide sur Windows).
pub fn home_dir() -> Result<String, String> {
    dirs::home_dir()
        .and_then(|p| p.to_str().map(str::to_string))
        .ok_or_else(|| "home directory not found".to_string())
}

fn build_menu(app: &tauri::App) -> tauri::Result<()> {
    // Windows : aucune barre de menu dans la fenêtre. Le bandeau
    // Fichier/Shell/View/Window s'afficherait sous notre titlebar custom et fait
    // doublon avec la sidebar (Réglages) + les raccourcis clavier.
    if cfg!(target_os = "windows") {
        return Ok(());
    }

    let h = app.handle();

    // Les accélérateurs des actions applicatives (nouveaux panneaux, splits,
    // clear, sidebar, réglages…) sont gérés côté JS et remappables depuis les
    // réglages, donc AUCUN accélérateur natif ici : sinon la touche déclencherait
    // deux fois (menu natif + clavier JS). Les items restent cliquables.
    let none = None::<&str>;

    let shell = Submenu::with_items(
        h,
        "Shell",
        true,
        &[
            &MenuItem::with_id(h, "new-connection", "New Connection…", true, none)?,
            &MenuItem::with_id(h, "close-pane", "Close Pane", true, none)?,
            &PredefinedMenuItem::separator(h)?,
            &MenuItem::with_id(h, "split-h", "Split Right", true, none)?,
            &MenuItem::with_id(h, "split-v", "Split Down", true, none)?,
            &PredefinedMenuItem::separator(h)?,
            &MenuItem::with_id(h, "clear", "Clear Terminal", true, none)?,
            &MenuItem::with_id(h, "sync-config", "Inject Claude Config", true, none)?,
            &MenuItem::with_id(h, "enhance-shell", "Enable Autosuggestions", true, none)?,
        ],
    )?;

    // Plein écran : item macOS-only (PredefinedMenuItem::fullscreen n'existe que sur macOS).
    #[cfg(target_os = "macos")]
    let view = Submenu::with_items(
        h,
        "View",
        true,
        &[
            &MenuItem::with_id(h, "toggle-sidebar", "Sidebar", true, none)?,
            &PredefinedMenuItem::separator(h)?,
            &PredefinedMenuItem::fullscreen(h, None)?,
        ],
    )?;
    #[cfg(not(target_os = "macos"))]
    let view = Submenu::with_items(
        h,
        "View",
        true,
        &[&MenuItem::with_id(h, "toggle-sidebar", "Sidebar", true, none)?],
    )?;

    let window = Submenu::with_items(
        h,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(h, None)?,
            &PredefinedMenuItem::maximize(h, None)?,
        ],
    )?;

    // macOS : menu applicatif à la Apple (about/hide/quit) + Édition natif.
    #[cfg(target_os = "macos")]
    let menu = {
        let app_menu = Submenu::with_items(
            h,
            "Arabel",
            true,
            &[
                &PredefinedMenuItem::about(h, None, Some(AboutMetadata::default()))?,
                &PredefinedMenuItem::separator(h)?,
                &MenuItem::with_id(h, "settings", "Settings…", true, none)?,
                &PredefinedMenuItem::separator(h)?,
                &PredefinedMenuItem::hide(h, None)?,
                &PredefinedMenuItem::hide_others(h, None)?,
                &PredefinedMenuItem::separator(h)?,
                &PredefinedMenuItem::quit(h, None)?,
            ],
        )?;
        let edit = Submenu::with_items(
            h,
            "Edit",
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
        Menu::with_items(h, &[&app_menu, &edit, &shell, &view, &window])?
    };

    // Windows/Linux : pas de menu applicatif Apple, pas d'Édition natif (WebView2
    // gère déjà copier/coller dans les champs, et un accélérateur Ctrl+C volerait
    // le SIGINT du terminal). Réglages/Quitter vont dans un menu « Fichier ».
    #[cfg(not(target_os = "macos"))]
    let menu = {
        let file = Submenu::with_items(
            h,
            "File",
            true,
            &[
                &MenuItem::with_id(h, "settings", "Settings…", true, none)?,
                &PredefinedMenuItem::separator(h)?,
                &PredefinedMenuItem::quit(h, None)?,
            ],
        )?;
        Menu::with_items(h, &[&file, &shell, &view, &window])?
    };

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
            local::ssh_pty_connect,
            local::local_metrics,
            local::mosh_connect,
            local::mosh_available,
            local::wsl_distros,
            ssh::metrics_watch,
            ssh::metrics_unwatch,
            ssh::ssh_connect,
            ssh::ssh_write,
            ssh::ssh_resize,
            ssh::ssh_disconnect,
            ssh::claude_probe,
            ssh::claude_sync,
            ssh::shell_enhance,
            ssh::events_watch,
            ssh::events_unwatch,
            ssh::ssh_config_parse,
            ssh::vscode_terminal,
            ssh::port_forward_start,
            ssh::port_forward_stop,
            sftp::sftp_home,
            sftp::sftp_list,
            sftp::sftp_download,
            sftp::sftp_upload,
            sftp::sftp_paste_image,
            sftp::git_run,
            store::store_load,
            store::store_save,
            store::passphrase_set,
            store::passphrase_delete,
            fonts::list_fonts
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
