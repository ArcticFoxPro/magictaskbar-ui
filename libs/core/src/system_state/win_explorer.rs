use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct StartMenuItem {
    pub path: PathBuf,
    pub umid: Option<String>,
    pub toast_activator: Option<String>,
    /// Will be present if the item is a shortcut
    pub target: Option<PathBuf>,
}
