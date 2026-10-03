//! Persistent user settings stored as JSON in the app config directory.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Node data directory (passed as --appdir). Empty = app data dir default.
    pub data_dir: String,
    /// Start the node automatically when the app launches.
    pub auto_start_node: bool,
    /// Restart the node automatically if it exits unexpectedly.
    pub auto_restart_node: bool,
    /// gRPC listen address.
    pub rpc_listen: String,
    /// wRPC Borsh listen address (used by the dashboard and the wallet).
    pub rpc_listen_borsh: String,
    /// P2P listen address.
    pub p2p_listen: String,
    pub upnp_enabled: bool,
    pub utxo_index: bool,
    /// Keep running in the system tray when the window is closed.
    pub run_in_background: bool,
    /// Extra raw arguments appended to the karlsend command line.
    pub extra_args: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data_dir: String::new(),
            auto_start_node: true,
            auto_restart_node: true,
            rpc_listen: "127.0.0.1:42110".into(),
            rpc_listen_borsh: "127.0.0.1:43110".into(),
            p2p_listen: "0.0.0.0:42111".into(),
            upnp_enabled: false,
            utxo_index: true,
            run_in_background: false,
            extra_args: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load(config_dir: &Path) -> Self {
        let path = config_dir.join(FILE_NAME);
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, config_dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(config_dir)?;
        let tmp = config_dir.join(format!("{FILE_NAME}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        // Atomic replace so a crash never leaves a half-written file.
        std::fs::rename(tmp, config_dir.join(FILE_NAME))?;
        Ok(())
    }

    /// Resolve the effective node data directory.
    pub fn resolved_data_dir(&self, default_dir: &Path) -> PathBuf {
        if self.data_dir.trim().is_empty() {
            default_dir.join("karlsend")
        } else {
            PathBuf::from(self.data_dir.trim())
        }
    }

    /// Build the karlsend command line from the settings.
    pub fn node_args(&self, data_dir: &Path) -> Vec<String> {
        let mut args = vec![
            "--yes".to_string(),
            format!("--appdir={}", data_dir.display()),
            format!("--rpclisten={}", self.rpc_listen),
            format!("--rpclisten-borsh={}", self.rpc_listen_borsh),
            format!("--listen={}", self.p2p_listen),
        ];
        if self.utxo_index {
            args.push("--utxoindex".into());
        }
        if !self.upnp_enabled {
            args.push("--disable-upnp".into());
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }
}
