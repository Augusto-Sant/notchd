//! A module's log file: `<log_dir>/<id>.log`, rotated at a size cap, a few
//! rotated files kept (`<id>.log.1` is the newest of them).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct LogFile {
    path: PathBuf,
    max_bytes: u64,
    keep: usize,
    inner: Mutex<Inner>,
}

struct Inner {
    file: Option<File>,
    size: u64,
}

impl LogFile {
    pub fn new(dir: &Path, id: &str, max_bytes: u64, keep: usize) -> LogFile {
        LogFile {
            path: dir.join(format!("{id}.log")),
            max_bytes,
            keep,
            inner: Mutex::new(Inner {
                file: None,
                size: 0,
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one line (a newline is added). Failures are swallowed: a log
    /// that cannot be written must not take the module down.
    pub fn line(&self, text: &str) {
        let mut data = Vec::with_capacity(text.len() + 1);
        data.extend_from_slice(text.as_bytes());
        data.push(b'\n');
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.file.is_none() && !self.open(&mut inner) {
            return;
        }
        if inner.size > 0 && inner.size + data.len() as u64 > self.max_bytes {
            self.rotate(&mut inner);
        }
        if let Some(file) = inner.file.as_mut()
            && file.write_all(&data).is_ok()
        {
            inner.size += data.len() as u64;
        }
    }

    /// A line the host itself writes, so it reads apart from the module's own.
    pub fn note(&self, text: &str) {
        self.line(&format!("[notchd {}] {text}", unix_secs()));
    }

    fn open(&self, inner: &mut Inner) -> bool {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                inner.size = file.metadata().map_or(0, |m| m.len());
                inner.file = Some(file);
                true
            }
            Err(_) => false,
        }
    }

    fn rotate(&self, inner: &mut Inner) {
        inner.file = None;
        let name = |n: usize| PathBuf::from(format!("{}.{n}", self.path.display()));
        if self.keep == 0 {
            let _ = fs::remove_file(&self.path);
        } else {
            let _ = fs::remove_file(name(self.keep));
            for n in (1..self.keep).rev() {
                let _ = fs::rename(name(n), name(n + 1));
            }
            let _ = fs::rename(&self.path, name(1));
        }
        self.open(inner);
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use notchd_testkit::tempdir::TempDir;

    #[test]
    fn rotates_at_the_cap_and_keeps_a_few() {
        let dir = TempDir::new("log");
        let log = LogFile::new(&dir.path().join("logs"), "m", 100, 2);
        for i in 0..40 {
            log.line(&format!("line number {i:02} of the log"));
        }
        let base = dir.path().join("logs/m.log");
        assert!(base.exists());
        assert!(dir.path().join("logs/m.log.1").exists());
        assert!(dir.path().join("logs/m.log.2").exists());
        assert!(!dir.path().join("logs/m.log.3").exists());
        let newest = fs::read_to_string(&base).unwrap();
        assert!(newest.contains("line number 39"));
        assert!(fs::metadata(&base).unwrap().len() <= 100);
    }

    #[test]
    fn appends_across_instances() {
        let dir = TempDir::new("log2");
        LogFile::new(dir.path(), "m", 1 << 20, 5).line("first");
        LogFile::new(dir.path(), "m", 1 << 20, 5).line("second");
        let text = fs::read_to_string(dir.path().join("m.log")).unwrap();
        assert_eq!(text, "first\nsecond\n");
    }
}
