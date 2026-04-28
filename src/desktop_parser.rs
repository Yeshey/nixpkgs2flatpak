use std::collections::HashMap;

/// Parse the `[Desktop Entry]` section of a .desktop file into a flat key→value map.
/// Keys from other sections are ignored.
pub fn parse(content: &str) -> HashMap<String, String> {
    let mut in_section = false;
    let mut map = HashMap::new();

    for line in content.lines() {
        let line = line.trim();

        if line == "[Desktop Entry]" {
            in_section = true;
            continue;
        }
        // Any other section header closes [Desktop Entry]
        if line.starts_with('[') {
            in_section = false;
            continue;
        }
        if !in_section || line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }

    map
}

/// Derive a Flatpak-style application ID from the .desktop filename or fields.
///
/// Priority:
///   1. `X-Flatpak-AppId` key in the file
///   2. The stem of the filename when it is already a reverse-DNS name (contains '.')
///   3. Fabricated fallback: `org.nixpkgs.<Name-without-spaces>`
pub fn extract_app_id(desktop_filename: &str, fields: &HashMap<String, String>) -> String {
    if let Some(id) = fields.get("X-Flatpak-AppId") {
        return id.clone();
    }

    let stem = desktop_filename
        .strip_suffix(".desktop")
        .unwrap_or(desktop_filename);

    if stem.contains('.') {
        return stem.to_string();
    }

    // Fallback: synthesise from the app's Name field
    let name = fields
        .get("Name")
        .map(|s| s.as_str())
        .unwrap_or(stem)
        .replace(' ', "");

    format!("org.nixpkgs.{}", name)
}

/// Split a `Categories=GNOME;GTK;Utility;` string into individual tokens.
pub fn parse_categories(raw: &str) -> Vec<String> {
    raw.split(';')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}