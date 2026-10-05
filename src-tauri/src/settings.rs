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
    /// Peers passed as --addpeer so a fresh node finds the network immediately,
    /// even when DNS seeders are slow or unreachable. Discovery still adds others.
    /// Entries may be "host:port" (resolved by the app at node start, because
    /// karlsend --addpeer only accepts IP addresses) or "ip:port".
    pub bootstrap_peers: Vec<String>,
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
            // On, like karlsend itself: a node reachable from the internet strengthens the
            // network. Settings shows it clearly and the dashboard reports the result.
            upnp_enabled: true,
            utxo_index: true,
            bootstrap_peers: vec![
                // DNS name first so a VPS move only needs a DNS update; the raw IP is a
                // fallback for when the name does not resolve (DNS outage, record missing).
                "p2p.karlsencoin.org:42111".into(),
                "72.62.146.74:42111".into(),
            ],
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

    /// Resolve the effective node data directory (passed as --appdir).
    /// Empty setting = `default_dir` itself, karlsend's own default folder, so chain data of a
    /// node run on its own is reused (layout: <appdir>/karlsen-mainnet/{datadir,logs}).
    pub fn resolved_data_dir(&self, default_dir: &Path) -> PathBuf {
        if self.data_dir.trim().is_empty() {
            default_dir.to_path_buf()
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
        for peer in resolve_peers(&self.bootstrap_peers) {
            args.push(format!("--addpeer={peer}"));
        }
        if !self.upnp_enabled {
            args.push("--disable-upnp".into());
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }
}

/// Turn "host:port" / "ip:port" entries into unique "ip:port" strings for --addpeer.
/// Unresolvable entries are skipped: bootstrap peers only speed up the first connection,
/// the node still finds the network through its built-in DNS seeders without them.
/// Blocking (DNS lookup); call from a blocking context.
pub fn resolve_peers(entries: &[String]) -> Vec<String> {
    use std::net::{SocketAddr, ToSocketAddrs};
    let mut out: Vec<String> = Vec::new();
    for entry in entries.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        let addrs: Vec<SocketAddr> = match entry.parse::<SocketAddr>() {
            Ok(a) => vec![a],
            Err(_) => entry.to_socket_addrs().map(|it| it.collect()).unwrap_or_default(),
        };
        for a in addrs {
            let s = a.to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_data_dir_is_karlsend_default_folder() {
        let def = Path::new("/home/karl/.rusty-karlsen");
        assert_eq!(Settings::default().resolved_data_dir(def), def);
        let custom = Settings { data_dir: " /data/kls ".into(), ..Settings::default() };
        assert_eq!(custom.resolved_data_dir(def), Path::new("/data/kls"));
    }

    #[test]
    fn resolve_peers_keeps_ips_dedups_and_skips_bad_names() {
        let peers = vec![
            "72.62.146.74:42111".to_string(),
            " 72.62.146.74:42111 ".to_string(),
            "no-such-host.invalid:42111".to_string(),
            "".to_string(),
        ];
        assert_eq!(resolve_peers(&peers), vec!["72.62.146.74:42111".to_string()]);
    }
}
