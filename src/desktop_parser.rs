/// Derive a Flatpak-style application ID from the .desktop filename.
///
/// Priority:
///   1. The stem of the filename when it is already a reverse-DNS name (contains '.')
///   2. Fabricated fallback: `org.nixpkgs.<CleanedName>`
pub fn extract_app_id(desktop_filename: &str, pname: &str) -> String {
    let stem = desktop_filename
        .strip_suffix(".desktop")
        .unwrap_or(desktop_filename);

    if stem.contains('.') {
        return stem.to_string();
    }

    // Fallback: synthesise from the app's pname. 
    // Flatpak IDs must not contain hyphens in their segments.
    let clean_pname: String = pname.chars().filter(|c| c.is_alphanumeric()).collect();
    
    format!("org.nixpkgs.{}", clean_pname)
}