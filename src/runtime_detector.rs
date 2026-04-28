pub fn detect(attr_path: &str, desktop_filename: &str) -> &'static str {
    let attr  = attr_path.to_lowercase();
    let fname = desktop_filename.to_lowercase();

    let any_contains = |signals: &[&str]| {
        signals.iter().any(|s| attr.contains(s) || fname.contains(s))
    };

    if any_contains(&["kde", "plasma", "qt", "kf5", "kf6"]) || attr.starts_with("kdepackages") {
        return "org.kde.Platform/6.10";
    }

    // Default everything else to GNOME since nix2flatpak guarantees this index exists
    "org.gnome.Platform/49"
}