// Liste les familles de fontes installées (pour le sélecteur des réglages).
// font-kit énumère via Core Text (macOS), DirectWrite (Windows) ou fontconfig (Linux).
#[tauri::command]
pub fn list_fonts() -> Vec<String> {
    let mut names = font_kit::source::SystemSource::new()
        .all_families()
        .unwrap_or_default();
    names.retain(|n| !n.starts_with('.')); // fontes système cachées (macOS)
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}
