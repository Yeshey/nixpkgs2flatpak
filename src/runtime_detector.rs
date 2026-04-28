/// Pick the best Flatpak runtime based on package name and .desktop filename.
pub fn detect(attr_path: &str, desktop_filename: &str) -> &'static str {
    let attr  = attr_path.to_lowercase();
    let fname = desktop_filename.to_lowercase();

    let any_contains = |signals: &[&str]| {
        signals.iter().any(|s| {
            attr.contains(s) || fname.contains(s)
        })
    };

    // KDE / Qt — check before GNOME because some KDE apps mention GTK
    if any_contains(&["kde", "plasma", "qt", "kf5", "kf6"])
        || attr.starts_with("kdepackages")
    {
        return "org.kde.Platform/6.10";
    }

    // GNOME / GTK / libadwaita
    if any_contains(&["gnome", "gtk", "libadwaita", "adwaita"]) {
        return "org.gnome.Platform/49";
    }

    // Electron bundles: use GNOME runtime (smallest workable choice)
    if any_contains(&["electron"]) {
        return "org.gnome.Platform/49";
    }

    // Everything else (Java, CLI wrappers, X11 apps, …)
    "org.freedesktop.Platform/24.08"
}