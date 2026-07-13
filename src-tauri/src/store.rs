use std::path::PathBuf;
use tauri::{AppHandle, Manager};

fn store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("store.json"))
}

#[tauri::command]
pub fn store_load(app: AppHandle) -> Result<String, String> {
    Ok(std::fs::read_to_string(store_path(&app)?).unwrap_or_else(|_| "{}".into()))
}

#[tauri::command]
pub fn store_save(app: AppHandle, data: String) -> Result<(), String> {
    let path = store_path(&app)?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(path, data).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn passphrase_set(identity_id: String, passphrase: String) -> Result<(), String> {
    keyring::Entry::new("arabel", &identity_id)
        .and_then(|e| e.set_password(&passphrase))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn passphrase_delete(identity_id: String) -> Result<(), String> {
    // absente = déjà supprimée, pas une erreur
    match keyring::Entry::new("arabel", &identity_id).and_then(|e| e.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn passphrase_get(identity_id: &str) -> Option<String> {
    keyring::Entry::new("arabel", identity_id)
        .ok()
        .and_then(|e| e.get_password().ok())
}

#[cfg(test)]
mod tests {
    #[test]
    fn keychain_roundtrip() {
        let e = keyring::Entry::new("arabel-test", "test-id").unwrap();
        e.set_password("s3cret").unwrap();
        assert_eq!(e.get_password().unwrap(), "s3cret");
        e.delete_credential().unwrap();
        assert!(e.get_password().is_err());
    }
}
