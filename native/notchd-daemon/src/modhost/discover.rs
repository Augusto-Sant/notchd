//! Finding modules on disk: the manifest, and a fingerprint of a module's
//! folder so a save can be noticed by polling (std only, no watcher crate).

use notchd_proto::Manifest;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Folders a runtime writes into on its own; watching them would restart a
/// Python module every time it imports something.
const SKIP_DIRS: &[&str] = &["__pycache__", "node_modules", "target"];
const MAX_ENTRIES: usize = 5000;
const MAX_DEPTH: usize = 8;

/// A hash of every file's relative path, size and mtime under `dir` (hidden
/// entries and runtime caches left out). Equal fingerprints mean no save.
pub fn fingerprint(dir: &Path) -> u64 {
    let mut entries: Vec<(String, u64, u128)> = Vec::new();
    walk(dir, "", 0, &mut entries);
    entries.sort();
    let mut h = DefaultHasher::new();
    entries.hash(&mut h);
    h.finish()
}

fn walk(dir: &Path, rel: &str, depth: usize, out: &mut Vec<(String, u64, u128)>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.ends_with('~') || name.ends_with(".swp") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let path = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if meta.is_dir() {
            if depth < MAX_DEPTH && !SKIP_DIRS.contains(&name.as_str()) {
                walk(&entry.path(), &path, depth + 1, out);
            }
        } else {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            out.push((path, meta.len(), mtime));
        }
    }
}

/// Reads and validates `<dir>/module.json`. The error says what is wrong,
/// with the line and column when it is a syntax error.
pub fn load_manifest(dir: &Path) -> Result<Manifest, String> {
    let path = dir.join("module.json");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("module.json is missing".into());
        }
        Err(e) => return Err(format!("module.json: {e}")),
    };
    notchd_proto::parse::<Manifest>(&text).map_err(|e| format!("module.json: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use notchd_testkit::tempdir::TempDir;

    #[test]
    fn fingerprint_changes_on_edit_add_and_delete_but_not_on_caches() {
        let dir = TempDir::new("fp");
        let p = dir.path();
        fs::write(p.join("a.txt"), "one").unwrap();
        let first = fingerprint(p);
        assert_eq!(first, fingerprint(p));

        fs::create_dir_all(p.join("__pycache__")).unwrap();
        fs::write(p.join("__pycache__/x.pyc"), "bytes").unwrap();
        fs::write(p.join(".hidden"), "x").unwrap();
        assert_eq!(first, fingerprint(p), "caches and dotfiles are ignored");

        fs::write(p.join("a.txt"), "three").unwrap();
        let edited = fingerprint(p);
        assert_ne!(first, edited);
        fs::write(p.join("b.txt"), "").unwrap();
        let added = fingerprint(p);
        assert_ne!(edited, added);
        fs::remove_file(p.join("b.txt")).unwrap();
        assert_ne!(added, fingerprint(p));
    }

    #[test]
    fn manifest_errors_say_what_is_wrong() {
        let dir = TempDir::new("mf");
        let p = dir.path();
        assert!(load_manifest(p).unwrap_err().contains("missing"));

        fs::write(p.join("module.json"), "{\n  \"command\": [\n").unwrap();
        let e = load_manifest(p).unwrap_err();
        assert!(e.contains("line 3"), "{e}");

        fs::write(p.join("module.json"), "{\"title\":\"x\"}").unwrap();
        assert!(load_manifest(p).unwrap_err().contains("command"));

        fs::write(p.join("module.json"), "{\"command\":[]}").unwrap();
        assert!(load_manifest(p).unwrap_err().contains("argv"));

        fs::write(
            p.join("module.json"),
            "{\"command\":[\"x\"],\"subscribe\":[\"nope\"]}",
        )
        .unwrap();
        assert!(load_manifest(p).unwrap_err().contains("nope"));

        fs::write(p.join("module.json"), "{\"command\":[\"x\"]}").unwrap();
        assert!(load_manifest(p).is_ok());
    }
}
