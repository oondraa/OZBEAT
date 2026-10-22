//! Which yt-dlp to run: `MV_YTDLP`, one on the PATH, or else the official
//! Windows build kept in the clip cache. That copy is checked against the
//! release's published checksums and refreshed weekly, since YouTube changes
//! often and old versions stop working.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

const RELEASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/";
const EXE: &str = "yt-dlp.exe";
const SUMS: &str = "SHA2-256SUMS";
const REFRESH: Duration = Duration::from_secs(7 * 86_400);

pub(crate) async fn resolve(dir: &Path) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("MV_YTDLP") {
        return Ok(path.into());
    }
    let on_path = command(Path::new("yt-dlp"))
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success());
    if on_path || !cfg!(windows) {
        return Ok("yt-dlp".into());
    }
    let exe = dir.join(EXE);
    let age = tokio::fs::metadata(&exe)
        .await
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok());
    if age.is_some_and(|age| age < REFRESH) {
        return Ok(exe);
    }
    match download(&exe).await {
        Ok(()) => Ok(exe),
        // An older copy still beats none.
        Err(e) if age.is_some() => {
            eprintln!("yt-dlp: keeping the old copy, update failed: {e}");
            Ok(exe)
        }
        Err(e) => Err(format!("cannot get yt-dlp: {e}")),
    }
}

/// A command that doesn't flash a console window from the windowed app.
pub(crate) fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

async fn download(exe: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("OZBEAT/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let get = async |name: &str| -> Result<Vec<u8>, String> {
        let response = client
            .get(format!("{RELEASE}{name}"))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?;
        Ok(response.bytes().await.map_err(|e| e.to_string())?.to_vec())
    };
    let sums = String::from_utf8(get(SUMS).await?).map_err(|e| e.to_string())?;
    let expected = checksum_for(&sums, EXE).ok_or_else(|| format!("{SUMS} doesn't list {EXE}"))?;
    let bytes = get(EXE).await?;
    if sha256_hex(&bytes) != expected || !bytes.starts_with(b"MZ") {
        return Err("the download doesn't match its checksum".into());
    }
    let part = exe.with_extension("part");
    tokio::fs::write(&part, &bytes)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&part, exe)
        .await
        .map_err(|e| e.to_string())
}

/// The hash for `file` in `sha256sum` output (`<hex>  <file>` or `<hex> *<file>`).
fn checksum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == file).then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes);
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_exe_checksum() {
        let sums = "aa11  yt-dlp\nBB22  yt-dlp.exe\ncc33  yt-dlp_x86.exe\n";
        assert_eq!(checksum_for(sums, "yt-dlp.exe").as_deref(), Some("bb22"));
        assert_eq!(checksum_for(sums, "nope"), None);
    }
}
