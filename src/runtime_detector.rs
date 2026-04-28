use std::collections::HashMap;

/// Pick the best Flatpak runtime based on package name, .desktop Categories,
/// and the app's human-readable Name — no `nix eval` required.
///
/// Returns a static string slice because all possible values are known at compile time.
pub fn detect(
    attr_path: &str,
    categories: &[String],
    fields: &HashMap<String, String>,
) -> &'static str {
    let attr  = attr_path.to_lowercase();
    let name  = fields.get("Name").map(|s| s.to_lowercase()).unwrap_or_default();
    let cats: Vec<String> = categories.iter().map(|c| c.to_lowercase()).collect();

    let any_contains = |signals: &[&str]| {
        signals.iter().any(|s| {
            cats.iter().any(|c| c.contains(s)) || attr.contains(s) || name.contains(s)
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
    if any_contains(&["electron"]) || attr.contains("electron") {
        return "org.gnome.Platform/49";
    }

    // Everything else (Java, CLI wrappers, X11 apps, …)
    "org.freedesktop.Platform/24.08"
}