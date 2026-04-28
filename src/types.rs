use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    #[serde(rename = "attrPath")]
    pub attr_path: String,

    pub pname: String,

    #[serde(rename = "appId")]
    pub app_id: String,

    #[serde(rename = "desktopFile")]
    pub desktop_file: String,

    #[serde(rename = "runtimeHint")]
    pub runtime_hint: String,
}