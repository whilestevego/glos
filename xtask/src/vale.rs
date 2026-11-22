//! Vale, the prose linter, at a pinned version. xtask downloads it into `target/vale/` and checks
//! the download against a pinned SHA-256, so every machine runs the same Vale.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// The Vale release glos uses. To move to a new one, change it here with the checksums from the
/// release's `vale_<version>_checksums.txt`.
const VERSION: &str = "3.27.0";

/// The release asset and its SHA-256 for each (OS, CPU) that glos builds on.
const ASSETS: &[(&str, &str, &str, &str)] = &[
    (
        "macos",
        "aarch64",
        "macOS_arm64",
        "91bcd6352a832e5995a61dde73ee6367c5006669c6efa4091e0404ec8d7dd7e3",
    ),
    (
        "macos",
        "x86_64",
        "macOS_64-bit",
        "0288ab603ee6d67240ac5b97e67b0915efcfc28bb33f212a885110a2a5a6e2bc",
    ),
    (
        "linux",
        "x86_64",
        "Linux_64-bit",
        "c80d62f6bce87b81843ac5b512a26e63213927b957adba0a64fc594570da2000",
    ),
    (
        "linux",
        "aarch64",
        "Linux_arm64",
        "dcaee812fd892c5750702d16ed348a1f7113be57c73cf36fe9e016c789cae6e2",
    ),
];

/// A `vale` command that runs in the workspace root, after the binary and the Rust grammar are
/// in place.
pub fn command() -> Result<Command> {
    let root = crate::root();
    let vale = binary(&root)?;
    if !root.join(".vale/styles/config/grammars/rust.wasm").exists() {
        let status = Command::new(&vale)
            .arg("sync")
            .current_dir(&root)
            .status()?;
        if !status.success() {
            bail!("vale sync failed");
        }
    }
    let mut cmd = Command::new(vale);
    cmd.current_dir(root);
    Ok(cmd)
}

fn binary(root: &Path) -> Result<PathBuf> {
    let dir = root.join("target/vale").join(VERSION);
    let path = dir.join("vale");
    if path.exists() {
        return Ok(path);
    }
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let Some(&(.., asset, sha)) = ASSETS.iter().find(|a| a.0 == os && a.1 == arch) else {
        bail!("no pinned Vale {VERSION} release for {os} on {arch}");
    };
    let url = format!(
        "https://github.com/errata-ai/vale/releases/download/v{VERSION}/vale_{VERSION}_{asset}.tar.gz"
    );
    let bytes = crate::fetch(&url)?;
    let got = hex(&Sha256::digest(&bytes));
    if got != sha {
        bail!("{url} has SHA-256 {got}, but {sha} is pinned");
    }
    fs::create_dir_all(&dir)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes.as_slice()));
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_os_str() == "vale" {
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            let tmp = dir.join("vale.tmp");
            fs::write(&tmp, data)?;
            make_executable(&tmp)?;
            fs::rename(&tmp, &path)?;
            return Ok(path);
        }
    }
    bail!("{url} has no `vale` binary")
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("chmod {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_writes_two_lowercase_digits_per_byte() {
        assert_eq!(hex(&[0x00, 0x0f, 0xab]), "000fab");
    }

    #[test]
    fn every_asset_has_a_full_sha256() {
        for (os, arch, _, sha) in ASSETS {
            assert_eq!(sha.len(), 64, "{os} {arch}");
            assert!(sha.bytes().all(|b| b.is_ascii_hexdigit()), "{os} {arch}");
        }
    }
}
