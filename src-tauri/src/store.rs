use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit, Nonce};

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
    write_atomic(&path, data.as_bytes())
}

/// Écriture atomique : fichier .tmp voisin puis rename. Un crash en plein write
/// laisse au pire le .tmp, jamais un fichier de config tronqué/illisible.
fn write_atomic(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

// ─── coffre local des secrets (passphrases de clés, mots de passe) ────────────
// On ne passe PLUS par le trousseau OS : sur un macOS non signé (pas de certificat
// Apple à 100 €/an), le trousseau redemandait le mot de passe à CHAQUE lancement,
// « Toujours autoriser » ne tenant jamais. À la place : un fichier chiffré.
//
// Sécurité, honnêtement : chaque installation génère une clé aléatoire (vault.key,
// 0600) posée à côté du fichier chiffré. Partager le binaire Arabel ne fuit donc
// aucun secret, et rien n'est en clair sur le disque (sauvegardes/sync/partage
// d'écran). Plafond : qui a un accès complet à CETTE machine (les deux fichiers)
// peut déchiffrer — même niveau qu'un « retenir mon mot de passe » d'app.

/// Identifiant de l'app (= `identifier` de tauri.conf.json). Sert à retrouver le
/// dossier de config sans AppHandle, depuis les fonctions non-commande de ssh.rs.
/// ponytail: doit rester synchro avec tauri.conf.json ; un seul champ, changé rarement.
const APP_ID: &str = "com.luss.arabel";

/// Dossier de config, calculé comme Tauri (`app_config_dir`) mais sans AppHandle :
/// `dirs::config_dir()` donne la même base par OS (Application Support / %APPDATA% / .config).
fn config_dir() -> Result<PathBuf, String> {
    dirs::config_dir()
        .map(|d| d.join(APP_ID))
        .ok_or_else(|| "no config dir".into())
}

/// Charge la clé du coffre, ou la crée (32 octets aléatoires) au premier appel.
fn vault_key() -> Result<[u8; 32], String> {
    let dir = config_dir()?;
    let path = dir.join("vault.key");
    match std::fs::read(&path) {
        Ok(b) if b.len() == 32 => Ok(b.try_into().unwrap()),
        // Fichier présent mais illisible ou tronqué : NE PAS régénérer — une clé neuve
        // rendrait tous les secrets déjà chiffrés indéchiffrables, sans rien dire. On
        // remonte l'erreur (l'utilisateur peut supprimer vault.key pour repartir à zéro).
        Ok(_) => Err("vault.key corrompu (taille inattendue) — supprime-le pour réinitialiser le coffre".into()),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("vault.key illisible: {e}")),
        // Vraiment absent (premier lancement) : on crée la clé.
        Err(_) => {
            let mut raw = [0u8; 32];
            getrandom::getrandom(&mut raw).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            write_atomic(&path, &raw)?; // atomique : pas de vault.key à moitié écrit qui bricquerait le coffre
            restrict(&path);
            Ok(raw)
        }
    }
}

/// Permissions 0600 sur Unix (clé lisible par le seul propriétaire). No-op ailleurs.
fn restrict(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn secrets_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("secrets.json"))
}

fn load_secrets() -> HashMap<String, String> {
    secrets_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_secrets(map: &HashMap<String, String>) -> Result<(), String> {
    let path = secrets_path()?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string(map).map_err(|e| e.to_string())?;
    write_atomic(&path, json.as_bytes())?;
    restrict(&path);
    Ok(())
}

/// nonce(12) aléatoire préfixé au texte chiffré, le tout en base64.
fn encrypt(key: &[u8; 32], plain: &str) -> Result<String, String> {
    let mut nonce = [0u8; 12];
    getrandom::getrandom(&mut nonce).map_err(|e| e.to_string())?;
    let ct = ChaCha20Poly1305::new_from_slice(key)
        .unwrap()
        .encrypt(&Nonce::from(nonce), plain.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    Ok(B64.encode(out))
}

fn decrypt(key: &[u8; 32], blob: &str) -> Option<String> {
    let raw = B64.decode(blob).ok()?;
    let (nonce, ct) = raw.split_at_checked(12)?;
    let nonce: [u8; 12] = nonce.try_into().ok()?;
    let pt = ChaCha20Poly1305::new_from_slice(key)
        .unwrap()
        .decrypt(&Nonce::from(nonce), ct)
        .ok()?;
    String::from_utf8(pt).ok()
}

#[tauri::command]
pub fn passphrase_set(identity_id: String, passphrase: String) -> Result<(), String> {
    let key = vault_key()?;
    let mut map = load_secrets();
    map.insert(identity_id, encrypt(&key, &passphrase)?);
    save_secrets(&map)
}

#[tauri::command]
pub fn passphrase_delete(identity_id: String) -> Result<(), String> {
    let mut map = load_secrets();
    if map.remove(&identity_id).is_some() {
        save_secrets(&map)?;
    }
    Ok(())
}

pub fn passphrase_get(identity_id: &str) -> Option<String> {
    let key = vault_key().ok()?;
    decrypt(&key, load_secrets().get(identity_id)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let mut key = [0u8; 32];
        getrandom::getrandom(&mut key).unwrap();
        let blob = encrypt(&key, "s3cret").unwrap();
        assert_ne!(blob, "s3cret"); // bien chiffré, pas du clair encodé
        assert_eq!(decrypt(&key, &blob).as_deref(), Some("s3cret"));
        // mauvaise clé → refus (AEAD authentifié), pas de déchiffrement silencieux
        let mut other = [0u8; 32];
        getrandom::getrandom(&mut other).unwrap();
        assert_eq!(decrypt(&other, &blob), None);
        // deux chiffrements du même texte diffèrent (nonce aléatoire)
        assert_ne!(encrypt(&key, "s3cret").unwrap(), blob);
    }
}
