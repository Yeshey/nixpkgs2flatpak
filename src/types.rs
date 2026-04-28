use serde::{Deserialize, Serialize};

/// One entry in discovered.json — everything Nix needs to build the Flatpak
/// without evaluating the package itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    /// Full nixpkgs attribute path, e.g. "gnome-calculator" or "kdePackages.kcalc"
    #[serde(rename = "attrPath")]
    pub attr_path: String,

    /// Simple name used as the flake output key, e.g. "gnome-calculator"
    pub pname: String,

    /// Flatpak reverse-DNS application ID, e.g. "org.gnome.Calculator"
    #[serde(rename = "appId")]
    pub app_id: String,

    /// Filename of the .desktop file, e.g. "org.gnome.Calculator.desktop"
    #[serde(rename = "desktopFile")]
    pub desktop_file: String,

    /// Suggested Flatpak runtime, e.g. "org.gnome.Platform/49"
    #[serde(rename = "runtimeHint")]
    pub runtime_hint: String,

    /// XDG categories from the .desktop file, e.g. ["GNOME","GTK","Utility"]
    pub categories: Vec<String>,
}