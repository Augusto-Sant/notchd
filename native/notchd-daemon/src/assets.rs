//! The sidecars the daemon runs (the voice bridges, the components'
//! installer) are Python files. They are embedded in the binary and written
//! out on first use to `<data home>/notchd/sidecars/<version>/`, so the
//! release is one executable and no UI has to ship them. The version is a
//! hash of the files: a new build writes its own folder, an unchanged one
//! reuses the last, and a build can never run another build's files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The files, by their path under the sidecars' folder. `install.py` reads
/// `components.json` next to it and `local-live.py` imports `localai.py`
/// from `../components`, so the layout is the repo's.
pub const FILES: &[(&str, &[u8])] = &[
    (
        "live/codex-live.py",
        include_bytes!("../../../core/live/codex-live.py"),
    ),
    (
        "live/local-live.py",
        include_bytes!("../../../core/live/local-live.py"),
    ),
    (
        "components/install.py",
        include_bytes!("../../../core/components/install.py"),
    ),
    (
        "components/localai.py",
        include_bytes!("../../../core/components/localai.py"),
    ),
    (
        "components/components.json",
        include_bytes!("../../../core/components/components.json"),
    ),
];

/// A folder of another version this old is deleted when a new one is written.
const STALE: Duration = Duration::from_secs(7 * 24 * 3600);

/// FNV-1a over every file's name and bytes, in hex.
pub fn version() -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for (name, bytes) in FILES {
        feed(name.as_bytes());
        feed(&[0]);
        feed(bytes);
        feed(&[0]);
    }
    format!("{h:016x}")
}

/// Writes the files under `<data_home>/notchd/sidecars/<version>/` unless
/// that folder is complete already, and returns it. Written to a folder of
/// its own and renamed, so a daemon starting at the same time never sees a
/// half-written one.
pub fn extract(data_home: &Path) -> io::Result<PathBuf> {
    let base = data_home.join("notchd").join("sidecars");
    let dir = base.join(version());
    if dir.join(".complete").exists() {
        return Ok(dir);
    }
    fs::create_dir_all(&base)?;
    let tmp = base.join(format!(".tmp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    if let Err(e) = write_all(&tmp) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if fs::rename(&tmp, &dir).is_err() {
        // Another daemon won the race (its folder is complete), or an
        // unfinished folder is in the way: ours replaces it.
        if dir.join(".complete").exists() {
            let _ = fs::remove_dir_all(&tmp);
        } else {
            let _ = fs::remove_dir_all(&dir);
            fs::rename(&tmp, &dir)?;
        }
    }
    prune(&base, &dir);
    Ok(dir)
}

fn write_all(dir: &Path) -> io::Result<()> {
    for (name, bytes) in FILES {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, bytes)?;
        #[cfg(unix)]
        if name.ends_with(".py") {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
    }
    fs::write(dir.join(".complete"), version())
}

/// Removes the other versions' folders nobody has touched for a week.
fn prune(base: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path == keep {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > STALE);
        if old {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

/// The data home: `XDG_DATA_HOME`, else `~/.local/share`.
pub fn data_home(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    env("XDG_DATA_HOME")
        .filter(|d| d.starts_with('/'))
        .or_else(|| env("HOME").map(|h| format!("{}/.local/share", h.trim_end_matches('/'))))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("notchd-assets-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn extract_writes_every_file_once_and_reuses_the_folder() {
        let home = scratch("once");
        let dir = extract(&home).unwrap();
        for (name, bytes) in FILES {
            assert_eq!(&fs::read(dir.join(name)).unwrap(), bytes, "{name}");
        }
        assert!(dir.ends_with(version()));
        // A folder marked complete is not written again.
        fs::write(dir.join("components/components.json"), b"mine").unwrap();
        let again = extract(&home).unwrap();
        assert_eq!(again, dir);
        assert_eq!(
            fs::read(dir.join("components/components.json")).unwrap(),
            b"mine"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unfinished_folder_is_written_again() {
        let home = scratch("broken");
        let dir = home.join("notchd/sidecars").join(version());
        fs::create_dir_all(dir.join("live")).unwrap();
        fs::write(dir.join("live/codex-live.py"), b"half").unwrap();
        let out = extract(&home).unwrap();
        assert_eq!(
            fs::read(out.join("live/codex-live.py")).unwrap(),
            FILES[0].1
        );
        assert!(out.join(".complete").exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn data_home_follows_xdg_then_home() {
        let env = |k: &str| match k {
            "XDG_DATA_HOME" => Some("/x/data".to_string()),
            "HOME" => Some("/h".to_string()),
            _ => None,
        };
        assert_eq!(data_home(&env), Some(PathBuf::from("/x/data")));
        let env = |k: &str| (k == "HOME").then(|| "/h".to_string());
        assert_eq!(data_home(&env), Some(PathBuf::from("/h/.local/share")));
        assert_eq!(data_home(&|_| None), None);
    }
}
