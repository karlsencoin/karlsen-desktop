//! Mining settings (stored as `miner.json` in the app config directory) and their
//! translation into the KarlsenMiner and karlsen-stratum-bridge command lines.
//!
//! Every value that ends up on a command line is validated here, in the backend, so the
//! webview cannot pass arbitrary options to either process.

use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE_NAME: &str = "miner.json";

/// Default loopback port of the bundled stratum bridge (the bridge's own default).
pub const DEFAULT_STRATUM_PORT: u16 = 5555;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct MinerSettings {
    /// Karlsen address that receives the block rewards. Empty until the user enters one;
    /// mining never starts without it.
    pub mining_address: String,
    /// Rig (worker) name; appears in the bridge log and in the coinbase tag of found blocks.
    pub worker_name: String,
    /// Loopback TCP port the bundled stratum bridge listens on.
    pub stratum_port: u16,
    /// KarlsenMiner `-d` value: all | amd | nvidia | intel | amd:0,1 | nvidia:0 | 0,1,3
    pub devices: String,
    /// Pause a GPU at this core temperature in °C (`--gpu-off-temp`). `None` = miner default.
    pub gpu_off_temp: Option<u32>,
    /// Start mining automatically when Karlsen Desktop opens.
    pub auto_start: bool,
}

impl Default for MinerSettings {
    fn default() -> Self {
        Self {
            mining_address: String::new(),
            worker_name: "desktop".into(),
            stratum_port: DEFAULT_STRATUM_PORT,
            devices: "all".into(),
            gpu_off_temp: None,
            auto_start: false,
        }
    }
}

impl MinerSettings {
    pub fn load(config_dir: &Path) -> Self {
        std::fs::read_to_string(config_dir.join(FILE_NAME))
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

    /// Check every field; the error text is shown to the user as is.
    pub fn validate(&self) -> Result<(), String> {
        validate_address(self.mining_address.trim())?;
        validate_worker_name(self.worker_name.trim())?;
        if self.stratum_port < 1024 {
            return Err("The stratum port must be between 1024 and 65535.".into());
        }
        validate_devices(self.devices.trim())?;
        if let Some(t) = self.gpu_off_temp {
            if !(50..=100).contains(&t) {
                return Err("GPU pause temperature must be between 50 and 100 °C.".into());
            }
        }
        Ok(())
    }

    /// Build the KarlsenMiner command line: mine on the bundled bridge at 127.0.0.1.
    ///
    /// `--config` points at a file inside Desktop's own miner folder so a
    /// `karlsenminer_config.json` left somewhere else is never picked up, and `--log off`
    /// because Desktop captures the output itself. NVIDIA tuning options are deliberately not
    /// offered: they make the miner ask for administrator rights and restart itself, which
    /// detaches it from Desktop, and a miner stopped by Desktop on Windows cannot restore
    /// the previous clocks.
    pub fn miner_args(&self, work_dir: &Path) -> Result<Vec<String>, String> {
        self.validate()?;
        let mut args = vec![
            "--config".to_string(),
            work_dir.join("karlsenminer_config.json").display().to_string(),
            "--log".to_string(),
            "off".to_string(),
            "-o".to_string(),
            format!("stratum+tcp://127.0.0.1:{}", self.stratum_port),
            "-u".to_string(),
            format!("{}.{}", self.mining_address.trim(), self.worker_name.trim()),
            "-d".to_string(),
            self.devices.trim().to_string(),
        ];
        if let Some(t) = self.gpu_off_temp {
            args.push("--gpu-off-temp".into());
            args.push(t.to_string());
        }
        Ok(args)
    }

    /// Build the karlsen-stratum-bridge command line.
    ///
    /// `node_rpc_listen` is the node's gRPC listen address from the node settings; the bridge
    /// always dials it on 127.0.0.1. The stratum listener is bound to 127.0.0.1 as well, so
    /// nothing on the network can connect to it, and every optional HTTP server of the bridge
    /// (Prometheus, dashboard, health check) is switched off.
    ///
    /// `--solo-mining false` keeps the bridge's variable share difficulty: the miner then
    /// submits frequent low-difficulty shares, which only serve to measure the hashrate and
    /// show that the GPU works. Rewards are the same either way; every block goes to this
    /// node and pays the mining address.
    ///
    /// `--config` names a file in Desktop's miner folder that does not exist, so the bridge
    /// runs on its built-in defaults plus these flags and never loads a stray `config.yaml`.
    pub fn bridge_args(&self, node_rpc_listen: &str, work_dir: &Path) -> Result<Vec<String>, String> {
        self.validate()?;
        let node_port = node_rpc_listen
            .rsplit(':')
            .next()
            .and_then(|p| p.trim().parse::<u16>().ok())
            .filter(|p| *p != 0)
            .ok_or_else(|| format!("The node's gRPC address \"{node_rpc_listen}\" has no valid port (see Settings)."))?;
        let flag = |name: &str, value: &str| [name.to_string(), value.to_string()];
        Ok([
            flag("--config", &work_dir.join("bridge-config.yaml").display().to_string()),
            flag("--karlsend-address", &format!("127.0.0.1:{node_port}")),
            flag("--stratum-port", &format!("127.0.0.1:{}", self.stratum_port)),
            flag("--solo-mining", "false"),
            flag("--var-diff", "true"),
            flag("--prom-port", ""),
            flag("--web-dashboard-port", ""),
            flag("--health-check-port", ""),
            flag("--log-to-file", "false"),
            flag("--print-stats", "false"),
            flag("--coinbase-tag-suffix", "KarlsenDesktop"),
        ]
        .concat())
    }
}

const BECH32_CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const ADDRESS_PREFIX: &str = "karlsen";

/// Karlsen (Kaspa-style bech32) checksum over prefix and 5-bit payload; 0 for a valid address.
fn polymod(values: impl Iterator<Item = u8>) -> u64 {
    let mut c: u64 = 1;
    for d in values {
        let c0 = (c >> 35) as u8;
        c = ((c & 0x07_ffff_ffff) << 5) ^ d as u64;
        for (bit, generator) in
            [0x98_f2bc_8e61u64, 0x79_b76d_99e2, 0xf3_3e5f_b3c4, 0xae_2eab_e2a8, 0x1e_4f43_e470].into_iter().enumerate()
        {
            if c0 & (1 << bit) != 0 {
                c ^= generator;
            }
        }
    }
    c ^ 1
}

/// Validate a mainnet Karlsen address including its checksum, so a typo is caught here
/// instead of mining to an address nobody owns.
pub fn validate_address(address: &str) -> Result<(), String> {
    if address.is_empty() {
        return Err("Enter the Karlsen address that should receive the mining rewards.".into());
    }
    let Some(payload) = address.strip_prefix("karlsen:") else {
        return Err("The mining address must start with \"karlsen:\".".into());
    };
    // 61 characters for Schnorr public key and script hash addresses, 63 for ECDSA.
    if payload.len() != 61 && payload.len() != 63 {
        return Err("The mining address has the wrong length. Copy it again from your wallet.".into());
    }
    let mut data = Vec::with_capacity(payload.len());
    for ch in payload.bytes() {
        match BECH32_CHARSET.iter().position(|&c| c == ch) {
            Some(v) => data.push(v as u8),
            None => return Err("The mining address contains characters that are not valid in a Karlsen address.".into()),
        }
    }
    let prefix = ADDRESS_PREFIX.bytes().map(|b| b & 0x1f);
    if polymod(prefix.chain(std::iter::once(0)).chain(data)) != 0 {
        return Err("The mining address is mistyped (checksum mismatch). Copy it again from your wallet.".into());
    }
    Ok(())
}

fn validate_worker_name(name: &str) -> Result<(), String> {
    let ok = (1..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok {
        Ok(())
    } else {
        Err("The rig name may use 1 to 32 letters, digits, \"-\" and \"_\".".into())
    }
}

/// Accepts exactly the forms KarlsenMiner documents for `-d`.
fn validate_devices(devices: &str) -> Result<(), String> {
    let is_index_list =
        |s: &str| !s.is_empty() && s.split(',').all(|i| !i.is_empty() && i.len() <= 2 && i.bytes().all(|b| b.is_ascii_digit()));
    let ok = match devices.split_once(':') {
        None => matches!(devices, "all" | "amd" | "nvidia" | "intel") || is_index_list(devices),
        Some((vendor, list)) => matches!(vendor, "all" | "amd" | "nvidia" | "intel") && is_index_list(list),
    };
    if ok {
        Ok(())
    } else {
        Err("GPUs must be one of: all, amd, nvidia, intel, amd:0,1, nvidia:0 or a list like 0,1,3.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Karlsen treasury address (public; used in the miner READMEs).
    const TREASURY: &str = "karlsen:qzecljlvsd8h3x32ynuttzyya6s26p4p5p0gyztk7p984r25jp0y2s5fpdqhc";

    fn valid() -> MinerSettings {
        MinerSettings { mining_address: TREASURY.into(), ..MinerSettings::default() }
    }

    #[test]
    fn accepts_real_address_and_rejects_typos() {
        validate_address(TREASURY).unwrap();
        // One character changed: checksum must fail.
        let typo = TREASURY.replace("qzecljlv", "qzecljlw");
        assert!(validate_address(&typo).unwrap_err().contains("checksum"));
        assert!(validate_address("").is_err());
        assert!(validate_address("kaspa:qzecljlvsd8h3x32ynuttzyya6s26p4p5p0gyztk7p984r25jp0y2s5fpdqhc").is_err());
        assert!(validate_address("karlsen:qzec").is_err());
        assert!(validate_address(&TREASURY.to_uppercase()).is_err());
    }

    #[test]
    fn default_settings_need_an_address() {
        let s = MinerSettings::default();
        assert!(s.miner_args(Path::new("/x")).is_err());
        assert!(s.bridge_args("127.0.0.1:42110", Path::new("/x")).is_err());
        assert!(!s.auto_start);
        assert_eq!(s.stratum_port, 5555);
    }

    #[test]
    fn miner_mines_on_the_local_bridge() {
        let mut s = valid();
        s.worker_name = "rig-1".into();
        s.devices = "amd:0,1".into();
        s.gpu_off_temp = Some(80);
        s.stratum_port = 5600;
        let args = s.miner_args(Path::new("/x")).unwrap();
        assert_eq!(
            args,
            [
                "--config", "/x/karlsenminer_config.json", "--log", "off",
                "-o", "stratum+tcp://127.0.0.1:5600",
                "-u", &format!("{TREASURY}.rig-1"),
                "-d", "amd:0,1",
                "--gpu-off-temp", "80",
            ]
        );
    }

    #[test]
    fn bridge_is_loopback_only_and_dials_the_local_node() {
        let s = valid();
        // Even when the node's RPC is bound to all interfaces, the bridge dials loopback.
        let args = s.bridge_args("0.0.0.0:42999", Path::new("/x")).unwrap();
        let value = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].as_str();
        assert_eq!(value("--karlsend-address"), "127.0.0.1:42999");
        assert_eq!(value("--stratum-port"), "127.0.0.1:5555");
        assert_eq!(value("--config"), "/x/bridge-config.yaml");
        for off in ["--prom-port", "--web-dashboard-port", "--health-check-port"] {
            assert_eq!(value(off), "", "{off} must be disabled");
        }
        assert_eq!(value("--log-to-file"), "false");
        assert!(s.bridge_args("no-port", Path::new("/x")).is_err());
        assert!(s.bridge_args("127.0.0.1:0", Path::new("/x")).is_err());
    }

    #[test]
    fn rejects_values_that_could_become_options() {
        for (field, value) in [
            ("worker", "--pl 300"),
            ("worker", "a b"),
            ("worker", ""),
            ("devices", "--benchmark"),
            ("devices", "amd:"),
            ("devices", "gpu0"),
            ("devices", "0,,1"),
        ] {
            let mut s = valid();
            match field {
                "worker" => s.worker_name = value.into(),
                _ => s.devices = value.into(),
            }
            assert!(s.validate().is_err(), "{field} = {value:?} must be rejected");
        }
        let mut s = valid();
        s.gpu_off_temp = Some(20);
        assert!(s.validate().is_err());
        let mut s = valid();
        s.stratum_port = 80;
        assert!(s.validate().is_err());
    }

    #[test]
    fn accepts_documented_device_forms() {
        for d in ["all", "amd", "nvidia", "intel", "amd:0,1", "nvidia:0", "all:0,1", "0,1,3"] {
            let mut s = valid();
            s.devices = d.into();
            s.validate().unwrap_or_else(|e| panic!("{d}: {e}"));
        }
    }

    #[test]
    fn settings_round_trip_and_tolerate_old_or_missing_fields() {
        let dir = std::env::temp_dir().join(format!("kd-miner-settings-{}", std::process::id()));
        let s = MinerSettings { gpu_off_temp: Some(75), auto_start: true, ..valid() };
        s.save(&dir).unwrap();
        assert_eq!(MinerSettings::load(&dir), s);
        // "poolUrl" is from an earlier development build; unknown fields are ignored.
        std::fs::write(dir.join(FILE_NAME), r#"{"workerName":"x","poolUrl":"stratum+ssl://p:1"}"#).unwrap();
        let partial = MinerSettings::load(&dir);
        assert_eq!(partial.worker_name, "x");
        assert_eq!(partial.stratum_port, DEFAULT_STRATUM_PORT);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
