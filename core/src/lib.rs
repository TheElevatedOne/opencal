//! opencal's calendar store, usable from Rust (`opencal` crate) and from C/C++ (`libopencal.so`,
//! see `include/opencal.h`).
//!
//! * [`model`]: calendars, events and tasks;
//! * [`ics`]: the calendar file (iCalendar with tombstones);
//! * [`merge`]: merging two copies and listing the changes;
//! * [`recur`]: occurrences in a date range (recurring events expanded);
//! * [`sync`]: the pull → merge → push sync with user commands;
//! * [`config`]: settings and file locations shared with the opencal app.
//!
//! ```no_run
//! use opencal::{Item, When, ics, sync};
//! let paths = opencal::Paths::new(None);
//! let config = opencal::Config::load(&paths)?;
//! let db = paths.db(&config);
//! let mut store = sync::load_store(&db)?;
//! let cal = store.live_calendars()[0].uid.clone();
//! store.add_item(Item::new_task(&cal, "Water the plants", None));
//! opencal::save_store(&db, &store)?;
//! # anyhow::Ok(())
//! ```

pub mod config;
pub mod ffi;
pub mod ics;
pub mod merge;
pub mod model;
pub mod recur;
pub mod sync;

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

pub use config::{Config, Paths};
pub use merge::{Change, ChangeKind, Report};
pub use model::{Calendar, Item, Kind, Store, When};
pub use recur::Occurrence;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Write a file through a temporary file and a rename, so readers (and a crash) never see half
/// a file.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.with_context(|| format!("writing {}", path.display()))
}

/// Save the store as the calendar file.
pub fn save_store(path: &Path, store: &Store) -> Result<()> {
    write_atomic(path, ics::write(store).as_bytes())
}

#[cfg(test)]
mod tests;
