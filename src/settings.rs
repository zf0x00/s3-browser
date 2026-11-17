//! User preferences, saved next to the connections.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    Grid,
    List,
    Media,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub view: ViewMode,
    /// Grid tile size step: 0 small, 1 medium, 2 large.
    pub zoom: u8,
    pub inspector: bool,
    pub sidebar: bool,
    /// Fetch images to show thumbnails.
    pub previews: bool,
    /// Largest image (MB) fetched just for a thumbnail.
    pub preview_max_mb: u64,
    /// Local cache budget (GB).
    pub cache_gb: u64,
    /// `None` follows the system.
    pub dark: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            view: ViewMode::Grid,
            zoom: 1,
            inspector: true,
            sidebar: true,
            previews: true,
            preview_max_mb: 15,
            cache_gb: 5,
            dark: None,
        }
    }
}

impl Settings {
    fn path() -> std::path::PathBuf {
        crate::store::config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(crate::store::config_dir());
        let _ = std::fs::write(Self::path(), serde_json::to_vec_pretty(self).unwrap_or_default());
    }

    pub fn preview_max_bytes(&self) -> u64 {
        self.preview_max_mb * 1024 * 1024
    }

    pub fn cache_bytes(&self) -> u64 {
        self.cache_gb * 1024 * 1024 * 1024
    }
}
