//! Keeps downloaded artwork from growing without limit: the least recently
//! used files go first. Lookups only ever re-download what is missing, so
//! deleting a file costs at most one download.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Covers, artist photos and backgrounds from the lookups.
const IMAGES_MAX: u64 = 200 * 1024 * 1024;
/// The players' own covers (small, a few hundred KB each).
const PLAYER_ART_MAX: u64 = 50 * 1024 * 1024;
/// A half-written download older than this was interrupted.
const STALE_TMP: Duration = Duration::from_secs(3600);

/// Trims the caches on a background thread, so startup doesn't wait.
pub fn trim_in_background() {
    let mut dirs = vec![(
        mv_enrich::Config::from_env().cache_dir.join("files"),
        IMAGES_MAX,
    )];
    #[cfg(windows)]
    dirs.push((mv_sources::smtc::thumbnail_dir(), PLAYER_ART_MAX));
    #[cfg(not(windows))]
    let _ = PLAYER_ART_MAX;
    let spawned = std::thread::Builder::new()
        .name("cache".into())
        .spawn(move || {
            for (dir, max) in dirs {
                match trim(&dir, max) {
                    Ok(0) => {}
                    Ok(freed) => eprintln!("cache: freed {} MB in {}", freed >> 20, dir.display()),
                    Err(e) => eprintln!("cache: cannot trim {}: {e}", dir.display()),
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("cache: {e}");
    }
}

/// Deletes the oldest files in `dir` (by last use) until it holds at most
/// `max` bytes. Returns the bytes freed.
fn trim(dir: &Path, max: u64) -> std::io::Result<u64> {
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    let mut freed = 0;
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let path = entry.path();
        let tmp = path.extension().is_some_and(|e| e == "tmp");
        if tmp && used.elapsed().is_ok_and(|age| age > STALE_TMP) {
            if std::fs::remove_file(&path).is_ok() {
                freed += meta.len();
            }
            continue;
        }
        if !tmp {
            files.push((used, meta.len(), path));
        }
    }

    // Newest first; everything past the budget goes.
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut kept = 0;
    for (_, len, path) in files {
        if kept + len <= max {
            kept += len;
        } else if std::fs::remove_file(&path).is_ok() {
            freed += len;
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletes_least_recently_used_first() {
        let dir = std::env::temp_dir().join(format!("ozbeat-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        for (name, age_secs) in [("old", 300), ("mid", 200), ("new", 100)] {
            let path = dir.join(name);
            std::fs::write(&path, [0u8; 1000]).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(now - Duration::from_secs(age_secs))
                .unwrap();
        }
        std::fs::write(dir.join("partial.tmp"), [0u8; 10]).unwrap();

        let freed = trim(&dir, 2500).unwrap();

        assert_eq!(freed, 1000);
        assert!(!dir.join("old").exists());
        assert!(dir.join("mid").exists() && dir.join("new").exists());
        // A fresh .tmp may still be in the middle of a download.
        assert!(dir.join("partial.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_dir_is_fine() {
        let dir = std::env::temp_dir().join("ozbeat-cache-test-missing");
        assert_eq!(trim(&dir, 0).unwrap(), 0);
    }
}
