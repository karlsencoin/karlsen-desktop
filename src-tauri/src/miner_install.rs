//! Downloads and verifies the KarlsenMiner release binary.
//!
//! KarlsenMiner is not shipped inside the Desktop installer: antivirus products flag GPU
//! miners as "potentially unwanted", and a bundled miner would get the whole node + wallet
//! installer quarantined. The user opts in on the MINER tab instead, and Desktop downloads
//! the official release archive from GitHub.
//!
//! TLS uses the operating system's certificate store (ureq `native-certs`), so the download
//! also works behind antivirus HTTPS inspection or a company proxy.
//!
//! Trust does not rest on the download channel. Both the archive and the executable inside
//! it must match SHA-256 values compiled into this Desktop build, and the executable is
//! re-checked before every start. A new miner release therefore needs a new Desktop build
//! with updated pins; that is intentional.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// KarlsenMiner release pinned by this Desktop build.
pub const MINER_VERSION: &str = "3.2.0";

/// Hard cap on the download so a wrong or hostile response cannot fill the disk.
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
/// Hard cap on the extracted executable (decompression bomb guard).
const MAX_BINARY_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
}

/// One platform's release asset and its pinned hashes.
#[derive(Debug, Clone, Copy)]
pub struct Package {
    pub url: &'static str,
    pub kind: ArchiveKind,
    /// SHA-256 of the release archive (same value as SHA256SUMS.txt on the release).
    pub archive_sha256: &'static str,
    /// File name of the miner executable inside the archive and on disk.
    pub bin_name: &'static str,
    /// SHA-256 of the executable itself.
    pub bin_sha256: &'static str,
}

pub const WINDOWS_X64: Package = Package {
    url: "https://github.com/karlsencoin/karlsen-miner/releases/download/v3.2.0/karlsenminer-v3.2.0-windows.zip",
    kind: ArchiveKind::Zip,
    archive_sha256: "94b07c354eaab3e2da9a9893bcaa0799f4f3a3eaa11f4ba0683e32d39134b8cd",
    bin_name: "karlsenminer.exe",
    bin_sha256: "de7e561dab2c6627333c13e1fff04da61dff536ad1f6911142fd62c4b81c3e73",
};

pub const LINUX_X64: Package = Package {
    url: "https://github.com/karlsencoin/karlsen-miner/releases/download/v3.2.0/karlsenminer-v3.2.0-linux.tar.gz",
    kind: ArchiveKind::TarGz,
    archive_sha256: "cb6dfc8378f5b67598deba54818d6817b0d73cfeef82ca560147eb5295e42ddb",
    bin_name: "karlsenminer",
    bin_sha256: "d5fa519acba89811363e2bd4bc56c164b1d278a7066054447c016855279ee2e8",
};

/// The package for the platform Desktop was built for; `None` where KarlsenMiner has no build.
pub fn package() -> Option<&'static Package> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some(&WINDOWS_X64)
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(&LINUX_X64)
    } else {
        None
    }
}

/// Folder that holds this version's executable: `<miner_root>/karlsenminer-v<version>/`.
pub fn install_dir(miner_root: &Path) -> PathBuf {
    miner_root.join(format!("karlsenminer-v{MINER_VERSION}"))
}

pub fn binary_path(miner_root: &Path, pkg: &Package) -> PathBuf {
    install_dir(miner_root).join(pkg.bin_name)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    /// "downloading" | "verifying" | "extracting"
    pub stage: &'static str,
    pub received_bytes: u64,
    /// From Content-Length; `None` when the server does not send it.
    pub total_bytes: Option<u64>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a file as lowercase hex.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// True when the installed executable exists and matches the pinned hash.
pub fn is_installed(miner_root: &Path, pkg: &Package) -> bool {
    verify_installed(miner_root, pkg).is_ok()
}

/// Check the installed executable against the pinned hash. Called before every start, so a
/// file that was replaced or damaged after installation is never executed.
pub fn verify_installed(miner_root: &Path, pkg: &Package) -> Result<PathBuf, String> {
    let path = binary_path(miner_root, pkg);
    let actual = sha256_file(&path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => "KarlsenMiner is not installed.".to_string(),
        _ => format!("cannot read {}: {e}", path.display()),
    })?;
    if actual != pkg.bin_sha256 {
        return Err(format!(
            "{} does not match the official KarlsenMiner v{MINER_VERSION} release (SHA-256 {actual}). \
             It was changed after installation; remove it and install again.",
            path.display()
        ));
    }
    Ok(path)
}

/// Last path component of an archive entry name. The Windows release zip stores its entries
/// with backslashes ("karlsenminer-v3.2.0-windows\karlsenminer.exe"), so both separators count.
fn entry_file_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// Copy at most `MAX_BINARY_BYTES` from `src` into a new file at `dest`.
fn write_capped(mut src: impl Read, dest: &Path) -> Result<(), String> {
    let mut out = File::create(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let copied = io::copy(&mut src.by_ref().take(MAX_BINARY_BYTES + 1), &mut out)
        .map_err(|e| format!("cannot extract the miner executable: {e}"))?;
    if copied > MAX_BINARY_BYTES {
        return Err("the miner executable in the archive is larger than expected".into());
    }
    out.sync_all().map_err(|e| e.to_string())
}

/// Extract only the miner executable from `archive` into `dest`. Entry paths are never used
/// to build an output path, so a crafted archive cannot write outside `dest`.
fn extract_binary(archive: &Path, pkg: &Package, dest: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    match pkg.kind {
        ArchiveKind::Zip => {
            let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("bad zip archive: {e}"))?;
            for i in 0..zip.len() {
                let entry = zip.by_index(i).map_err(|e| format!("bad zip archive: {e}"))?;
                if entry.is_file() && entry_file_name(entry.name()) == pkg.bin_name {
                    return write_capped(entry, dest);
                }
            }
        }
        ArchiveKind::TarGz => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
            for entry in tar.entries().map_err(|e| format!("bad tar archive: {e}"))? {
                let entry = entry.map_err(|e| format!("bad tar archive: {e}"))?;
                let is_bin = entry.header().entry_type().is_file()
                    && entry
                        .path()
                        .ok()
                        .and_then(|p| p.file_name().map(|n| n == pkg.bin_name))
                        .unwrap_or(false);
                if is_bin {
                    return write_capped(entry, dest);
                }
            }
        }
    }
    Err(format!("{} not found in the release archive", pkg.bin_name))
}

/// Verify a downloaded archive against the pins and install its executable.
/// `archive` is consumed (deleted) whether or not installation succeeds.
pub fn install_from_archive(miner_root: &Path, pkg: &Package, archive: &Path) -> Result<PathBuf, String> {
    let result = (|| {
        let actual = sha256_file(archive).map_err(|e| e.to_string())?;
        if actual != pkg.archive_sha256 {
            return Err(format!(
                "Download rejected: SHA-256 {actual} does not match the official KarlsenMiner v{MINER_VERSION} \
                 release ({}). Nothing was installed.",
                pkg.archive_sha256
            ));
        }
        let dir = install_dir(miner_root);
        let staged = dir.join(format!("{}.part", pkg.bin_name));
        extract_binary(archive, pkg, &staged)?;
        let bin_hash = sha256_file(&staged).map_err(|e| e.to_string())?;
        if bin_hash != pkg.bin_sha256 {
            let _ = fs::remove_file(&staged);
            return Err(format!("Extracted {} has an unexpected SHA-256 ({bin_hash}). Nothing was installed.", pkg.bin_name));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        }
        let dest = dir.join(pkg.bin_name);
        // Only a fully verified file ever appears under the final name.
        fs::rename(&staged, &dest).map_err(|e| format!("cannot install {}: {e}", dest.display()))?;
        Ok(dest)
    })();
    let _ = fs::remove_file(archive);
    result
}

/// Download the pinned release archive, verify it and install the executable.
/// Blocking (network + disk); call from a blocking context.
pub fn download_and_install(
    miner_root: &Path,
    pkg: &Package,
    mut progress: impl FnMut(InstallProgress),
) -> Result<PathBuf, String> {
    let dir = install_dir(miner_root);
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let archive = dir.join("download.part");

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .try_proxy_from_env(true)
        .user_agent(concat!("karlsen-desktop/", env!("CARGO_PKG_VERSION")))
        .build();
    let response = agent.get(pkg.url).call().map_err(|e| format!("download failed: {e}"))?;
    let total_bytes = response.header("Content-Length").and_then(|v| v.parse::<u64>().ok());
    if total_bytes.is_some_and(|t| t > MAX_ARCHIVE_BYTES) {
        return Err("download refused: the release archive is larger than expected".into());
    }

    let download = (|| -> Result<(), String> {
        let mut reader = response.into_reader().take(MAX_ARCHIVE_BYTES + 1);
        let mut out = File::create(&archive).map_err(|e| format!("cannot create {}: {e}", archive.display()))?;
        let mut buf = [0u8; 64 * 1024];
        let mut received_bytes = 0u64;
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| format!("cannot write the download: {e}"))?;
            received_bytes += n as u64;
            if received_bytes > MAX_ARCHIVE_BYTES {
                return Err("download refused: the release archive is larger than expected".into());
            }
            progress(InstallProgress { stage: "downloading", received_bytes, total_bytes });
        }
        out.sync_all().map_err(|e| e.to_string())?;
        progress(InstallProgress { stage: "verifying", received_bytes, total_bytes });
        Ok(())
    })();
    if let Err(e) = download {
        let _ = fs::remove_file(&archive);
        return Err(e);
    }
    install_from_archive(miner_root, pkg, &archive)
}

/// Remove this version's install folder (executable and miner config).
pub fn uninstall(miner_root: &Path) -> Result<(), String> {
    let dir = install_dir(miner_root);
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kd-miner-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const FAKE_BIN: &[u8] = b"#!/bin/sh\necho fake miner\n";

    fn fake_pkg(kind: ArchiveKind, archive: &Path, bin_name: &'static str) -> Package {
        Package {
            url: "",
            kind,
            archive_sha256: Box::leak(sha256_file(archive).unwrap().into_boxed_str()),
            bin_name,
            bin_sha256: Box::leak(hex(&Sha256::digest(FAKE_BIN)).into_boxed_str()),
        }
    }

    fn write_tar_gz(path: &Path, entry: &str) {
        let gz = flate2::write::GzEncoder::new(File::create(path).unwrap(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(FAKE_BIN.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, entry, FAKE_BIN).unwrap();
        tar.into_inner().unwrap().finish().unwrap();
    }

    fn write_zip(path: &Path, entry: &str) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        zip.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(FAKE_BIN).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn entry_file_name_handles_both_separators() {
        assert_eq!(entry_file_name("karlsenminer-v3.2.0-windows\\karlsenminer.exe"), "karlsenminer.exe");
        assert_eq!(entry_file_name("karlsenminer-v3.2.0-linux/karlsenminer"), "karlsenminer");
        assert_eq!(entry_file_name("karlsenminer"), "karlsenminer");
    }

    #[test]
    fn installs_from_tar_gz_and_verifies() {
        let root = temp_root("targz");
        fs::create_dir_all(install_dir(&root)).unwrap();
        let archive = root.join("a.tar.gz");
        write_tar_gz(&archive, "karlsenminer-vX-linux/karlsenminer");
        let pkg = fake_pkg(ArchiveKind::TarGz, &archive, "karlsenminer");
        let bin = install_from_archive(&root, &pkg, &archive).unwrap();
        assert_eq!(fs::read(&bin).unwrap(), FAKE_BIN);
        assert!(!archive.exists(), "archive must be removed after install");
        assert!(is_installed(&root, &pkg));
        // A file modified after installation is refused.
        fs::write(&bin, b"tampered").unwrap();
        assert!(verify_installed(&root, &pkg).unwrap_err().contains("does not match"));
        uninstall(&root).unwrap();
        assert!(!is_installed(&root, &pkg));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn installs_from_zip_with_backslash_entry_names() {
        let root = temp_root("zip");
        fs::create_dir_all(install_dir(&root)).unwrap();
        let archive = root.join("a.zip");
        write_zip(&archive, "karlsenminer-vX-windows\\karlsenminer.exe");
        let pkg = fake_pkg(ArchiveKind::Zip, &archive, "karlsenminer.exe");
        let bin = install_from_archive(&root, &pkg, &archive).unwrap();
        assert_eq!(fs::read(&bin).unwrap(), FAKE_BIN);
        // The executable sits directly in the install folder, never at the entry's own path.
        assert_eq!(bin.parent().unwrap(), install_dir(&root));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_archive_with_wrong_hash() {
        let root = temp_root("badhash");
        fs::create_dir_all(install_dir(&root)).unwrap();
        let archive = root.join("a.tar.gz");
        write_tar_gz(&archive, "x/karlsenminer");
        let mut pkg = fake_pkg(ArchiveKind::TarGz, &archive, "karlsenminer");
        pkg.archive_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
        let err = install_from_archive(&root, &pkg, &archive).unwrap_err();
        assert!(err.contains("Download rejected"), "{err}");
        assert!(!binary_path(&root, &pkg).exists());
        assert!(!archive.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_archive_whose_binary_differs_from_the_pin() {
        let root = temp_root("badbin");
        fs::create_dir_all(install_dir(&root)).unwrap();
        let archive = root.join("a.tar.gz");
        write_tar_gz(&archive, "x/karlsenminer");
        let mut pkg = fake_pkg(ArchiveKind::TarGz, &archive, "karlsenminer");
        pkg.bin_sha256 = "1111111111111111111111111111111111111111111111111111111111111111";
        let err = install_from_archive(&root, &pkg, &archive).unwrap_err();
        assert!(err.contains("unexpected SHA-256"), "{err}");
        assert!(fs::read_dir(install_dir(&root)).unwrap().next().is_none(), "no file may be left behind");
        fs::remove_dir_all(&root).unwrap();
    }

    /// The pins must match the real v3.2.0 release assets. Runs only when the assets are
    /// provided: KD_MINER_ASSETS=<dir with the release zip and tar.gz> cargo test
    #[test]
    fn pins_match_real_release_assets() {
        let Some(assets) = std::env::var_os("KD_MINER_ASSETS").map(PathBuf::from) else {
            return;
        };
        for pkg in [&WINDOWS_X64, &LINUX_X64] {
            let root = temp_root(pkg.bin_name);
            fs::create_dir_all(install_dir(&root)).unwrap();
            let name = pkg.url.rsplit('/').next().unwrap();
            let copy = root.join(name);
            fs::copy(assets.join(name), &copy).unwrap();
            install_from_archive(&root, pkg, &copy).unwrap();
            verify_installed(&root, pkg).unwrap();
            fs::remove_dir_all(&root).unwrap();
        }
    }

    /// Full download path against GitHub. Needs network: KD_MINER_NETWORK_TEST=1 cargo test
    #[test]
    fn downloads_and_installs_from_github() {
        if std::env::var_os("KD_MINER_NETWORK_TEST").is_none() {
            return;
        }
        let pkg = package().expect("no KarlsenMiner build for this platform");
        let root = temp_root("net");
        let mut last = 0;
        let bin = download_and_install(&root, pkg, |p| last = p.received_bytes).unwrap();
        assert!(last > 1_000_000);
        assert_eq!(verify_installed(&root, pkg).unwrap(), bin);
        fs::remove_dir_all(&root).unwrap();
    }
}
