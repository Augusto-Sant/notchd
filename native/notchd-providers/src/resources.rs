//! The `resources` provider: CPU, memory and the disk holding a path
//! (docs/architecture.md, Providers). The rules that read the numbers, and
//! decide when one runs short, are `notchd-core::resources`; a backend only
//! does the I/O of its OS.

use notchd_core::resources::Usage;
use serde::{Deserialize, Serialize};

/// What one reading found. A part that could not be read is `None`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// The busy fraction since the last reading; `None` on the first one.
    pub cpu: Option<f64>,
    /// How many CPUs there are, 0 when unknown.
    pub cores: usize,
    pub memory: Option<Usage>,
    /// Present on the readings that read the disk (it changes slowly).
    pub disk: Option<Usage>,
}

pub trait ResourcesProvider: Send {
    /// A reading; the disk is the filesystem holding `disk_path`.
    fn sample(&mut self, disk_path: &str) -> Sample;
}

/// The backend of this OS; `None` where there is none yet (the module then
/// shows its empty state).
pub fn system() -> Option<Box<dyn ResourcesProvider>> {
    #[cfg(target_os = "linux")]
    return Some(Box::new(crate::linux::procfs::Procfs::new()));
    #[cfg(not(target_os = "linux"))]
    return None;
}
