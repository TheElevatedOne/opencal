//! Settings (`~/.config/opencal/config.toml`) and file locations.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub app: AppConfig,
    pub storage: StorageConfig,
    pub sync: SyncConfig,
    pub view: ViewConfig,
    pub reminders: RemindersConfig,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Keep running with a tray icon when the window is closed (reminders and timed sync keep
    /// working). Quit from the tray menu or with Ctrl+Q.
    pub background: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemindersConfig {
    /// Desktop notification before events start.
    pub events: bool,
    /// Minutes before the start (0 = at the start).
    pub minutes_before: u32,
    /// Desktop notification when a task is due (date-only tasks: at `view.day_start_hour`).
    pub tasks: bool,
}

impl Default for RemindersConfig {
    fn default() -> Self {
        RemindersConfig {
            events: true,
            minutes_before: 10,
            tasks: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// The calendar file. Empty = `~/.local/share/opencal/calendar.ics`.
    pub path: String,
    /// Tombstones of deleted entries older than this many days are dropped (0 = keep forever).
    pub tombstone_days: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// Shell command that downloads the remote copy to `$CAL_DB_PULL`. Empty = skip the pull.
    pub pull: String,
    /// Shell command that uploads `$CAL_DB` after the merge. Empty = skip the push.
    pub push: String,
    /// Run the push even when the remote copy already has everything.
    pub always_push: bool,
    pub on_start: bool,
    /// Sync every N minutes while opencal runs (0 = off).
    pub interval_minutes: u32,
    /// Sync this many seconds after the last change (0 = off).
    pub after_change_seconds: u32,
    /// Kill a command that runs longer than this.
    pub timeout_seconds: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            pull: String::new(),
            push: String::new(),
            always_push: false,
            on_start: false,
            interval_minutes: 0,
            after_change_seconds: 0,
            timeout_seconds: 300,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewConfig {
    /// day, week, month, agenda or tasks
    pub view: String,
    pub week_starts_monday: bool,
    /// The hour the day and week views scroll to.
    pub day_start_hour: u32,
    pub show_completed: bool,
    /// UIDs of calendars that are switched off.
    pub hidden_calendars: Vec<String>,
    pub width: i32,
    pub height: i32,
    pub sidebar: bool,
    /// Length of new events, in minutes.
    pub event_minutes: u32,
    /// UID of the calendar new events and tasks go to (empty = the first visible one).
    pub default_calendar: String,
    /// Width of the sidebar in pixels.
    pub sidebar_width: i32,
}

impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig {
            view: "month".into(),
            week_starts_monday: true,
            day_start_hour: 7,
            show_completed: true,
            hidden_calendars: Vec::new(),
            width: 1100,
            height: 720,
            sidebar: true,
            event_minutes: 60,
            default_calendar: String::new(),
            sidebar_width: 260,
        }
    }
}

/// Where opencal keeps its files. `OPENCAL_PROFILE` (or `--profile DIR`) puts everything under
/// one directory, for testing or a second set of calendars.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn new(profile: Option<&Path>) -> Self {
        let profile = profile.map(Path::to_path_buf).or_else(|| {
            std::env::var_os("OPENCAL_PROFILE")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
        });
        match profile {
            Some(p) => Paths {
                config_dir: p.clone(),
                data_dir: p.clone(),
                cache_dir: p.join("cache"),
            },
            None => Paths {
                config_dir: dirs::config_dir()
                    .unwrap_or_else(|| PathBuf::from(".config"))
                    .join("opencal"),
                data_dir: dirs::data_dir()
                    .unwrap_or_else(|| PathBuf::from(".local/share"))
                    .join("opencal"),
                cache_dir: dirs::cache_dir()
                    .unwrap_or_else(|| PathBuf::from(".cache"))
                    .join("opencal"),
            },
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// The calendar file for this config.
    pub fn db(&self, config: &Config) -> PathBuf {
        if config.storage.path.is_empty() {
            self.data_dir.join("calendar.ics")
        } else {
            expand_home(&config.storage.path)
        }
    }
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(path),
    }
}

impl Config {
    /// A missing file gives the defaults.
    pub fn load(paths: &Paths) -> Result<Config> {
        let file = paths.config_file();
        match std::fs::read_to_string(&file) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("reading {}", file.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", file.display())),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let text = toml::to_string_pretty(self)?;
        crate::write_atomic(&paths.config_file(), text.as_bytes())
    }
}
