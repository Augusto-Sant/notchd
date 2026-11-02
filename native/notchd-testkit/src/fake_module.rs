//! Fake user modules: folders under a temp modules directory, made from the
//! tiny shell scripts in `tests/modules/` (good, crash, slow, malformed,
//! badtree, badmanifest, stubborn, chatty, garbage) or written inline.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The repo's `tests/modules/` folder.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/modules")
}

/// Copies fixture `fixture` to `<modules_dir>/<id>/` and returns the folder.
pub fn install(modules_dir: &Path, fixture: &str, id: &str) -> PathBuf {
    let from = fixtures_dir().join(fixture);
    let to = modules_dir.join(id);
    fs::create_dir_all(&to).expect("create module dir");
    for entry in fs::read_dir(&from).unwrap_or_else(|e| panic!("fixture {fixture}: {e}")) {
        let entry = entry.expect("fixture entry");
        // `fs::copy` keeps the executable bit (`run.sh`).
        fs::copy(entry.path(), to.join(entry.file_name())).expect("copy fixture file");
    }
    to
}

/// Writes a module of your own: `manifest` as `module.json` and `script` as
/// an executable `run.sh`.
pub fn write(modules_dir: &Path, id: &str, manifest: &str, script: &str) -> PathBuf {
    let dir = modules_dir.join(id);
    fs::create_dir_all(&dir).expect("create module dir");
    fs::write(dir.join("module.json"), manifest).expect("write manifest");
    write_script(&dir, script);
    dir
}

/// Replaces `run.sh` (a save). The host fingerprints size and mtime, so a
/// rewrite is noticed when either differs: a rewrite of equal length whose
/// mtime did not move (a coarse clock) gets its mtime pushed forward here, so
/// no sleep is needed to make a save visible.
pub fn write_script(dir: &Path, script: &str) {
    let path = dir.join("run.sh");
    let before = fs::metadata(&path)
        .and_then(|m| Ok((m.len(), m.modified()?)))
        .ok();
    fs::write(&path, script).expect("write script");
    if let Some((len, mtime)) = before {
        let file = fs::File::options()
            .write(true)
            .open(&path)
            .expect("open script");
        let now = file.metadata().and_then(|m| m.modified()).unwrap_or(mtime);
        if len == script.len() as u64 && now <= mtime {
            file.set_modified(mtime + Duration::from_secs(1))
                .expect("bump mtime");
        }
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// A script that sends `init` back to stderr, then one compact view with `text`.
pub fn viewer_script(text: &str) -> String {
    format!(
        "#!/bin/sh\nread -r init\nprintf '{{\"t\":\"view\",\"slot\":\"compact\",\"tree\":{{\"t\":\"text\",\"text\":\"{text}\"}}}}\\n'\nwhile read -r line; do\n  case \"$line\" in *'\"t\":\"stop\"'*) exit 0 ;; esac\ndone\n"
    )
}

/// Kept for callers; [`write_script`] now guarantees a distinct fingerprint
/// itself, so this no longer sleeps.
pub fn tick_mtime() {}
