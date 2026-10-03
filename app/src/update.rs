//! Checks GitHub for a newer release and replaces this exe with it.
//!
//! Windows lets a running exe be renamed but not overwritten, so the update
//! renames this one to `<name>.old`, puts the new one in its place and starts
//! it. The new process deletes the `.old` file once the old one has exited.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use serde::Deserialize;

const LATEST: &str = "https://api.github.com/repos/oondraa/OZBEAT/releases/latest";
const EXE_ASSET: &str = "OZBEAT.exe";
const SUMS_ASSET: &str = "SHA256SUMS.txt";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    exe_url: String,
    sums_url: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Status {
    #[default]
    Checking,
    UpToDate,
    Available(Release),
    Installing,
    /// Installed; the new version is starting and this one closes.
    Restarting,
    Failed(String, Release),
}

pub type SharedUpdate = Arc<Mutex<Status>>;

/// Asks GitHub once, in the background; offline just means no offer.
pub fn check_in_background(update: SharedUpdate) {
    spawn("update-check", async move {
        let status = match latest().await {
            Ok(release) if is_newer(&release.version, CURRENT) => Status::Available(release),
            Ok(_) => Status::UpToDate,
            Err(e) => {
                eprintln!("update check failed: {e}");
                Status::UpToDate
            }
        };
        *update.lock().unwrap() = status;
    });
}

/// Downloads and verifies `release`, swaps it in and restarts into it.
pub fn install_in_background(update: SharedUpdate, release: Release, ctx: egui::Context) {
    *update.lock().unwrap() = Status::Installing;
    spawn("update-install", async move {
        let installed = match std::env::current_exe() {
            Ok(exe) => install(&release, &exe).await.map(|()| exe),
            Err(e) => Err(format!("nenašel jsem soubor programu ({e})")),
        };
        let status = match installed.and_then(|exe| restart(&exe)) {
            Ok(()) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                Status::Restarting
            }
            Err(e) => {
                eprintln!("update failed: {e}");
                Status::Failed(e, release)
            }
        };
        *update.lock().unwrap() = status;
        ctx.request_repaint();
    });
}

/// Deletes what a previous update left behind. The old process may still be
/// closing, so this retries for a few seconds.
pub fn remove_previous() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let old = sibling(&exe, "old");
    if !old.exists() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("update-cleanup".into())
        .spawn(move || {
            for _ in 0..20 {
                if std::fs::remove_file(&old).is_ok() || !old.exists() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
}

fn spawn(name: &str, task: impl Future<Output = ()> + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(task),
                Err(e) => eprintln!("update: cannot start runtime: {e}"),
            }
        });
    if let Err(e) = spawned {
        eprintln!("update: {e}");
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("OZBEAT/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())
}

async fn get(url: &str) -> Result<Vec<u8>, String> {
    let response = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("nepodařilo se stáhnout ({e})"))?;
    if !response.status().is_success() {
        return Err(format!("GitHub odpověděl {}", response.status()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("stahování se přerušilo ({e})"))?;
    Ok(bytes.to_vec())
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// The newest published release.
async fn latest() -> Result<Release, String> {
    let body = get(LATEST).await?;
    let release: GithubRelease = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let asset = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
            .ok_or_else(|| format!("vydání {} nemá soubor {name}", release.tag_name))
    };
    Ok(Release {
        version: release.tag_name.trim_start_matches('v').to_owned(),
        exe_url: asset(EXE_ASSET)?,
        sums_url: asset(SUMS_ASSET)?,
    })
}

/// Downloads `release`, checks it against the published checksum and puts it
/// in place of `exe`.
async fn install(release: &Release, exe: &Path) -> Result<(), String> {
    let sums = String::from_utf8(get(&release.sums_url).await?).map_err(|e| e.to_string())?;
    let expected = checksum_for(&sums, EXE_ASSET)
        .ok_or_else(|| format!("{SUMS_ASSET} neobsahuje {EXE_ASSET}"))?;
    let bytes = get(&release.exe_url).await?;
    let actual = sha256_hex(&bytes);
    if actual != expected {
        return Err(format!(
            "stažený soubor je poškozený (kontrolní součet {actual}, má být {expected})"
        ));
    }
    if !bytes.starts_with(b"MZ") {
        return Err("stažený soubor není program".into());
    }
    replace(exe, &bytes).map_err(|e| {
        format!(
            "nemůžu přepsat {} ({e}). Přesuň OZBEAT do složky, kam můžeš zapisovat \
             (třeba Dokumenty), nebo stáhni novou verzi z webu",
            exe.display()
        )
    })
}

/// Writes `bytes` next to `exe`, then swaps the two by renaming.
fn replace(exe: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let new = sibling(exe, "new");
    let old = sibling(exe, "old");
    std::fs::write(&new, bytes)?;
    // A leftover from an earlier update would block the rename.
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&new, exe) {
        // Put the working version back.
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    Ok(())
}

fn restart(exe: &Path) -> Result<(), String> {
    std::process::Command::new(exe)
        .spawn()
        .map(drop)
        .map_err(|e| {
            format!("nová verze je nainstalovaná, ale nejde spustit ({e}); spusť OZBEAT znovu")
        })
}

/// `OZBEAT.exe` -> `OZBEAT.exe.<suffix>`, in the same folder.
fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    exe.with_file_name(name)
}

/// The hash for `file` in `sha256sum` output (`<hex> *<file>` or `<hex>  <file>`).
fn checksum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(' ')?;
        let name = name.trim_start().trim_start_matches('*');
        (name == file).then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes);
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// "0.1.10" > "0.1.9"; anything unparsable is never newer.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|p| p.parse().ok()).collect() };
    match (parse(candidate), parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
        assert!(!is_newer("latest", "0.1.2"));
    }

    #[test]
    fn finds_checksum_in_sha256sum_output() {
        let sums = "AB12 *OZBEAT.exe\ncd34  OZBEAT.scr\n";
        assert_eq!(checksum_for(sums, "OZBEAT.exe").as_deref(), Some("ab12"));
        assert_eq!(checksum_for(sums, "OZBEAT.scr").as_deref(), Some("cd34"));
        assert_eq!(checksum_for(sums, "other.zip"), None);
    }

    #[test]
    fn hashes_like_sha256sum() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn replaces_file_and_keeps_old_one_aside() {
        let dir = std::env::temp_dir().join(format!("ozbeat-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("OZBEAT.exe");
        std::fs::write(&exe, b"old").unwrap();

        replace(&exe, b"new").unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(sibling(&exe, "old")).unwrap(), b"old");
        assert!(!sibling(&exe, "new").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Downloads the real latest release; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn installs_latest_release() {
        let dir = std::env::temp_dir().join(format!("ozbeat-update-net-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("OZBEAT.exe");
        std::fs::write(&exe, b"MZ old").unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let release = runtime.block_on(latest()).unwrap();
        runtime.block_on(install(&release, &exe)).unwrap();

        let installed = std::fs::read(&exe).unwrap();
        assert!(installed.len() > 1_000_000 && installed.starts_with(b"MZ"));
        assert_eq!(std::fs::read(sibling(&exe, "old")).unwrap(), b"MZ old");
        println!("installed {} ({} bytes)", release.version, installed.len());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
