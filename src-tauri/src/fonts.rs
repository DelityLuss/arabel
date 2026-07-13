// Liste les familles de fontes installées (pour le sélecteur des réglages).
#[tauri::command]
pub fn list_fonts() -> Vec<String> {
    let mut names: Vec<String> = core_text::font_manager::copy_available_font_family_names()
        .iter()
        .map(|s| s.to_string())
        .filter(|n| !n.starts_with('.')) // fontes système cachées
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}
