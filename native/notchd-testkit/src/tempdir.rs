//! Temp directories and the hermetic environment of a test daemon. Nothing
//! here reads or writes a real user path.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A directory removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> TempDir {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        // Unix socket paths are short (108 bytes): keep the name short too.
        let path =
            std::env::temp_dir().join(format!("nd-{label}-{}-{n}-{nanos:x}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A temp `XDG_RUNTIME_DIR` and `NOTCHD_CONFIG` (and `HOME`, so nothing falls
/// back to the real one).
pub struct TestEnv {
    pub dir: TempDir,
    pub runtime_dir: PathBuf,
    pub config: PathBuf,
}

impl TestEnv {
    pub fn new(label: &str) -> TestEnv {
        let dir = TempDir::new(label);
        let runtime_dir = dir.path().join("run");
        std::fs::create_dir_all(&runtime_dir).expect("create runtime dir");
        let config = dir.path().join("config.json");
        TestEnv {
            dir,
            runtime_dir,
            config,
        }
    }

    /// The socket the daemon binds for `instance` under this environment.
    pub fn socket(&self, instance: &str) -> PathBuf {
        self.runtime_dir
            .join("notchd")
            .join(format!("{instance}.sock"))
    }

    pub fn write_config(&self, text: &str) {
        std::fs::write(&self.config, text).expect("write config");
    }

    /// Points a command at this environment and away from the real one.
    pub fn apply(&self, cmd: &mut Command) {
        cmd.env_clear()
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("NOTCHD_CONFIG", &self.config)
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("xdg-config"));
    }
}
