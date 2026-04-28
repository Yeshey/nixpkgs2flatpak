/// Derive a Flatpak-style application ID from the .desktop filename.
pub fn extract_app_id(desktop_filename: &str, pname: &str) -> String {
    let stem = desktop_filename
        .strip_suffix(".desktop")
        .unwrap_or(desktop_filename);

    let raw_id = if stem.contains('.') {
        stem.to_string()
    } else {
        // Fallback: synthesise from the app's pname.
        let clean_pname: String = pname.chars().filter(|c| c.is_alphanumeric()).collect();
        format!("org.nixpkgs.{}", clean_pname)
    };

    // DBus/Flatpak Rule: Segments cannot start with digits.
    // We iterate through segments and prefix any digit-starting parts with 'a'.
    let sanitized: Vec<String> = raw_id
        .split('.')
        .map(|segment| {
            if segment.chars().next().map_or(false, |c| c.is_ascii_digit()) {
                format!("a{}", segment)
            } else {
                segment.to_string()
            }
        })
        .collect();

    sanitized.join(".")
}