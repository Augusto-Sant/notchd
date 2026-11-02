//! One daemon serves one instance: a config path and a socket path.
//! Resolution order, first hit wins (docs/architecture.md, Transport):
//! socket: `--socket`, `NOTCHD_SOCKET`, `$XDG_RUNTIME_DIR/notchd/<instance>.sock`
//! (macOS `$TMPDIR`); config: `--config`, `NOTCHD_CONFIG`,
//! `$XDG_CONFIG_HOME/notchd/config.json`, `~/.config/notchd/config.json`.

use std::path::{Path, PathBuf};

pub const DEFAULT_INSTANCE: &str = "default";

/// Where one daemon listens and what it reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub name: String,
    pub socket: PathBuf,
    pub config: PathBuf,
}

/// Overrides from flags; `None` falls through to the environment.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub instance: Option<String>,
    pub socket: Option<PathBuf>,
    pub config: Option<PathBuf>,
}

fn env(get: &dyn Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    get(key).filter(|v| !v.is_empty())
}

/// Resolves an instance from overrides and an environment lookup (a closure,
/// so tests need not touch the process environment).
pub fn resolve(o: &Overrides, get: &dyn Fn(&str) -> Option<String>) -> Result<Instance, String> {
    let name = o
        .instance
        .clone()
        .or_else(|| env(get, "NOTCHD_INSTANCE"))
        .unwrap_or_else(|| DEFAULT_INSTANCE.to_string());
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("bad instance name {name:?}"));
    }
    let socket = match o
        .socket
        .clone()
        .or_else(|| env(get, "NOTCHD_SOCKET").map(PathBuf::from))
    {
        Some(p) => p,
        None => {
            let base = env(get, "XDG_RUNTIME_DIR")
                .or_else(|| env(get, "TMPDIR"))
                .ok_or("no XDG_RUNTIME_DIR or TMPDIR: set NOTCHD_SOCKET")?;
            Path::new(&base).join("notchd").join(format!("{name}.sock"))
        }
    };
    let config = match o
        .config
        .clone()
        .or_else(|| env(get, "NOTCHD_CONFIG").map(PathBuf::from))
    {
        Some(p) => p,
        None => {
            let base = match env(get, "XDG_CONFIG_HOME") {
                Some(x) => PathBuf::from(x),
                None => Path::new(&env(get, "HOME").ok_or("no HOME: set NOTCHD_CONFIG")?)
                    .join(".config"),
            };
            base.join("notchd").join("config.json")
        }
    };
    Ok(Instance {
        name,
        socket,
        config,
    })
}

/// [`resolve`] against the process environment.
pub fn from_env(o: &Overrides) -> Result<Instance, String> {
    resolve(o, &|k| std::env::var(k).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn defaults_are_instance_scoped_under_the_runtime_dir() {
        let i = resolve(
            &Overrides::default(),
            &lookup(&[("XDG_RUNTIME_DIR", "/run/user/1"), ("HOME", "/h")]),
        )
        .unwrap();
        assert_eq!(i.socket, Path::new("/run/user/1/notchd/default.sock"));
        assert_eq!(i.config, Path::new("/h/.config/notchd/config.json"));
    }

    #[test]
    fn env_and_flags_override_in_order() {
        let get = lookup(&[
            ("XDG_RUNTIME_DIR", "/r"),
            ("NOTCHD_SOCKET", "/s.sock"),
            ("NOTCHD_CONFIG", "/c.json"),
            ("NOTCHD_INSTANCE", "dev"),
        ]);
        let i = resolve(&Overrides::default(), &get).unwrap();
        assert_eq!(
            (i.name.as_str(), i.socket.as_path()),
            ("dev", Path::new("/s.sock"))
        );
        assert_eq!(i.config, Path::new("/c.json"));
        let o = Overrides {
            socket: Some("/f.sock".into()),
            ..Default::default()
        };
        assert_eq!(resolve(&o, &get).unwrap().socket, Path::new("/f.sock"));
    }

    #[test]
    fn a_bad_instance_name_is_refused() {
        let o = Overrides {
            instance: Some("../x".into()),
            ..Default::default()
        };
        assert!(resolve(&o, &lookup(&[("XDG_RUNTIME_DIR", "/r"), ("HOME", "/h")])).is_err());
    }
}
