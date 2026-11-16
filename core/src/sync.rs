//! Command-based sync, in the spirit of KeePassXC's "merge from file":
//!
//! 1. the **pull** command downloads the remote copy of the calendar file to `$CAL_DB_PULL`;
//! 2. opencal merges it into the local file `$CAL_DB` and works out what changed both ways;
//! 3. the **push** command uploads `$CAL_DB` (skipped when the remote already has everything).
//!
//! Commands run with `sh -c` in the calendar file's directory. They get these variables:
//!
//! | variable                | value                                                         |
//! | ----------------------- | ------------------------------------------------------------- |
//! | `CAL_DB`                | the local calendar file (merged when the push runs)           |
//! | `CAL_DB_PULL`           | where the pull command must save the downloaded copy          |
//! | `CAL_DIR`               | the directory of `CAL_DB`                                      |
//! | `CAL_SYNC_DIR`          | a scratch directory for this sync, deleted afterwards          |
//! | `CAL_SYNC_STAGE`        | `pull` or `push`                                               |
//! | `CAL_SYNC_REPORT`       | push only: text file listing the changes (a commit message)    |
//! | `CAL_SYNC_INCOMING`     | push only: number of changes the remote brought                |
//! | `CAL_SYNC_OUTGOING`     | push only: number of changes the push sends                    |
//! | `CAL_REMOTE_EXISTS`     | push only: `1` if the pull produced a file, else `0`           |
//! | `CAL_VERSION`           | opencal's version                                              |
//!
//! A pull that succeeds without creating `$CAL_DB_PULL` means "there is no remote copy yet"
//! (the first sync); the push then uploads the local file.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::config::{Config, Paths};
use crate::merge::{self, Report};

/// Output of one command.
#[derive(Clone, Debug, Default, Serialize)]
pub struct CommandLog {
    pub stage: String,
    pub command: String,
    pub status: Option<i32>,
    pub output: String,
}

/// Result of a sync.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SyncResult {
    pub report: Report,
    pub remote_existed: bool,
    pub pushed: bool,
    pub logs: Vec<CommandLog>,
    /// Set when the sync stopped; the local file is unchanged if it failed before the merge.
    pub error: Option<String>,
}

impl SyncResult {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }

    /// One line for a status bar.
    pub fn status_line(&self) -> String {
        match &self.error {
            Some(e) => format!("Sync failed: {e}"),
            None if !self.remote_existed => format!(
                "Synced (no remote copy yet, uploaded): {}",
                self.report.summary()
            ),
            None => format!("Synced: {}", self.report.summary()),
        }
    }

    pub fn log_text(&self) -> String {
        let mut s = String::new();
        for l in &self.logs {
            let status = l
                .status
                .map_or("killed".to_string(), |c| format!("exit {c}"));
            s.push_str(&format!(
                "$ {}  [{}, {status}]\n{}",
                l.command, l.stage, l.output
            ));
            if !l.output.ends_with('\n') && !l.output.is_empty() {
                s.push('\n');
            }
        }
        s
    }
}

/// The sync's environment variables as (name, description), for settings dialogs.
pub const VARIABLES: &[(&str, &str)] = &[
    (
        "CAL_DB",
        "the local calendar file (already merged when the push runs)",
    ),
    (
        "CAL_DB_PULL",
        "where the pull command must save the downloaded copy",
    ),
    ("CAL_DIR", "the directory of CAL_DB (commands run there)"),
    ("CAL_SYNC_DIR", "scratch directory, deleted after the sync"),
    ("CAL_SYNC_STAGE", "pull or push"),
    (
        "CAL_SYNC_REPORT",
        "push: text file listing the changes, usable as a commit message",
    ),
    (
        "CAL_SYNC_INCOMING",
        "push: number of changes the remote brought",
    ),
    ("CAL_SYNC_OUTGOING", "push: number of changes being pushed"),
    (
        "CAL_REMOTE_EXISTS",
        "push: 1 if the pull produced a file, else 0",
    ),
    ("CAL_VERSION", "opencal's version"),
];

fn run_command(
    stage: &str,
    command: &str,
    cwd: &Path,
    env: &[(&str, String)],
    timeout: Duration,
) -> Result<CommandLog> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.env("CAL_SYNC_STAGE", stage);
    // Its own process group, so a timeout can stop everything the command started.
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting the {stage} command"))?;
    // Read both pipes on threads so a chatty command can't block on a full pipe.
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if started.elapsed() > timeout {
            // Kill the whole process group: programs the script started (ssh, rclone, …) would
            // otherwise keep the output pipes open and the reads below would never finish.
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut output = t_out.join().unwrap_or_default();
    output.push_str(&t_err.join().unwrap_or_default());
    Ok(CommandLog {
        stage: stage.into(),
        command: command.into(),
        status: status.and_then(|s| s.code()),
        output,
    })
}

/// Load the calendar file; a missing file is a new calendar.
pub fn load_store(path: &Path) -> Result<crate::Store> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let mut store =
                crate::ics::parse(&text).with_context(|| format!("reading {}", path.display()))?;
            store.ensure_default_calendar();
            Ok(store)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(crate::Store::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run a full sync with the configured commands. Blocks until the commands finish, so call it
/// off the UI thread.
pub fn sync(paths: &Paths, config: &Config) -> SyncResult {
    let mut result = SyncResult::default();
    if let Err(e) = sync_inner(paths, config, &mut result) {
        result.error = Some(format!("{e:#}"));
    }
    result
}

fn sync_inner(paths: &Paths, config: &Config, result: &mut SyncResult) -> Result<()> {
    let sc = &config.sync;
    if sc.pull.trim().is_empty() && sc.push.trim().is_empty() {
        bail!("no sync commands are set up");
    }
    let db = paths.db(config);
    let dir = db
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir)?;
    std::fs::create_dir_all(&paths.cache_dir)?;
    let scratch = paths.cache_dir.join(format!("sync-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;
    let scratch = ScratchDir(scratch);
    let pull_file = scratch.0.join("pulled.ics");
    let timeout = Duration::from_secs(sc.timeout_seconds.max(1));

    // The local file must exist for the commands (and a first push).
    let local = load_store(&db)?;
    if !db.exists() {
        crate::save_store(&db, &local)?;
    }

    let mut env: Vec<(&str, String)> = vec![
        ("CAL_DB", db.display().to_string()),
        ("CAL_DB_PULL", pull_file.display().to_string()),
        ("CAL_DIR", dir.display().to_string()),
        ("CAL_SYNC_DIR", scratch.0.display().to_string()),
        ("CAL_VERSION", env!("CARGO_PKG_VERSION").to_string()),
    ];

    if !sc.pull.trim().is_empty() {
        let log = run_command("pull", &sc.pull, &dir, &env, timeout)?;
        let status = log.status;
        result.logs.push(log);
        match status {
            Some(0) => {}
            Some(c) => bail!("the pull command failed (exit {c})"),
            None => bail!("the pull command timed out after {} s", timeout.as_secs()),
        }
    }

    let remote = match std::fs::read_to_string(&pull_file) {
        Ok(text) if !text.trim().is_empty() => {
            result.remote_existed = true;
            crate::ics::parse(&text).context("reading the pulled file")?
        }
        _ => crate::Store::default(),
    };
    // Safety net: once this pull command has found a remote copy, a pull that "succeeds"
    // without a file is far more likely a broken script or connection than a deleted remote.
    // Pushing now would replace the remote copy and lose the other devices' changes.
    let state_file = paths.data_dir.join("sync-state");
    let known_remote = std::fs::read_to_string(&state_file)
        .ok()
        .is_some_and(|s| s == sc.pull);
    if !sc.pull.trim().is_empty() && !result.remote_existed && known_remote {
        bail!(
            "the pull command produced no file, but earlier syncs with it found one; not pushing, \
             so the remote copy isn't overwritten. If the remote copy is really gone, delete {} \
             and sync again",
            state_file.display()
        );
    }
    if result.remote_existed && !known_remote {
        let _ = crate::write_atomic(&state_file, sc.pull.as_bytes());
    }

    // Re-read the local file: it may have been edited while the pull ran.
    let local = load_store(&db).unwrap_or(local);
    let (mut merged, report) = merge::merge(&local, &remote);
    merged.prune_tombstones(config.storage.tombstone_days);
    if merged != local {
        crate::save_store(&db, &merged)?;
    }
    result.report = report;

    let need_push = sc.always_push || !result.report.outgoing.is_empty() || !result.remote_existed;
    if !sc.push.trim().is_empty() && need_push {
        let report_file = scratch.0.join("report.txt");
        let text = format!(
            "opencal sync: {}\n\n{}",
            result.report.summary(),
            result.report.to_text()
        );
        std::fs::write(&report_file, text)?;
        env.push(("CAL_SYNC_REPORT", report_file.display().to_string()));
        env.push((
            "CAL_SYNC_INCOMING",
            result.report.incoming.len().to_string(),
        ));
        env.push((
            "CAL_SYNC_OUTGOING",
            result.report.outgoing.len().to_string(),
        ));
        env.push((
            "CAL_REMOTE_EXISTS",
            if result.remote_existed { "1" } else { "0" }.to_string(),
        ));
        let log = run_command("push", &sc.push, &dir, &env, timeout)?;
        let status = log.status;
        result.logs.push(log);
        match status {
            Some(0) => result.pushed = true,
            Some(c) => bail!("the push command failed (exit {c}); the local file is merged"),
            None => bail!(
                "the push command timed out after {} s; the local file is merged",
                timeout.as_secs()
            ),
        }
    }
    Ok(())
}

/// Merge another calendar file into the local one (no commands).
pub fn merge_file(db: &Path, other: &Path) -> Result<Report> {
    let local = load_store(db)?;
    let text =
        std::fs::read_to_string(other).with_context(|| format!("reading {}", other.display()))?;
    let remote = crate::ics::parse(&text)?;
    let (merged, report) = merge::merge(&local, &remote);
    if merged != local {
        crate::save_store(db, &merged)?;
    }
    Ok(report)
}

/// Ready-made command pairs, shown in the settings dialog. `(name, pull, push)`.
pub const PRESETS: &[(&str, &str, &str)] = &[
    (
        "rclone (any cloud storage rclone supports)",
        // Only "not found" means "no remote copy yet"; any other failure stops the sync.
        r#"out=$(rclone lsf remote:opencal/); rc=$?; [ $rc -eq 3 ] && exit 0; [ $rc -eq 0 ] || exit $rc; printf '%s\n' "$out" | grep -qx calendar.ics || exit 0; rclone copyto remote:opencal/calendar.ics "$CAL_DB_PULL""#,
        r#"rclone copyto "$CAL_DB" remote:opencal/calendar.ics"#,
    ),
    (
        "scp / ssh server",
        // `test -e` exits 1 for a missing file and 255 when ssh fails.
        r#"ssh user@host test -e opencal/calendar.ics; rc=$?; [ $rc -eq 1 ] && exit 0; [ $rc -eq 0 ] || exit $rc; scp -q user@host:opencal/calendar.ics "$CAL_DB_PULL""#,
        r#"ssh user@host mkdir -p opencal && scp -q "$CAL_DB" user@host:opencal/calendar.ics.tmp && ssh user@host mv opencal/calendar.ics.tmp opencal/calendar.ics"#,
    ),
    (
        "WebDAV with curl (Nextcloud, …)",
        r#"code=$(curl -s -n -o "$CAL_DB_PULL" -w '%{http_code}' https://dav.example.com/opencal/calendar.ics); [ "$code" = 200 ] || { rm -f "$CAL_DB_PULL"; [ "$code" = 404 ]; }"#,
        r#"curl -sf -n -T "$CAL_DB" https://dav.example.com/opencal/calendar.ics"#,
    ),
    (
        "git repository (~/calendar-repo)",
        r#"git -C ~/calendar-repo pull -q --ff-only && { [ ! -e ~/calendar-repo/calendar.ics ] || cp ~/calendar-repo/calendar.ics "$CAL_DB_PULL"; }"#,
        r#"cp "$CAL_DB" ~/calendar-repo/calendar.ics && git -C ~/calendar-repo add calendar.ics && git -C ~/calendar-repo commit -q -F "$CAL_SYNC_REPORT" && git -C ~/calendar-repo push -q"#,
    ),
    (
        "Shared folder (Syncthing, NFS, USB stick)",
        r#"[ ! -e ~/Sync/calendar.ics ] || cp ~/Sync/calendar.ics "$CAL_DB_PULL""#,
        r#"cp "$CAL_DB" ~/Sync/calendar.ics.tmp && mv ~/Sync/calendar.ics.tmp ~/Sync/calendar.ics"#,
    ),
    (
        "Your own scripts",
        r#"~/.config/opencal/pull.sh"#,
        r#"~/.config/opencal/push.sh"#,
    ),
];
