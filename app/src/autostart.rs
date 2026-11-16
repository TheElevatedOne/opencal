//! "Start opencal when I log in": a desktop entry in `$XDG_CONFIG_HOME/autostart`.

use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs_home().map(|h| h.join(".config")))?;
    Some(
        base.join("autostart")
            .join(format!("{}.desktop", crate::APP_ID)),
    )
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// `Some(hidden)` when autostart is on.
pub fn state() -> Option<bool> {
    let text = std::fs::read_to_string(path()?).ok()?;
    let exec = text.lines().find_map(|l| l.strip_prefix("Exec="))?;
    Some(exec.split_whitespace().any(|a| a == "--background"))
}

pub fn enable(hidden: bool) -> anyhow::Result<()> {
    let path = path().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let exe = std::env::current_exe()?;
    // Quoted per the desktop entry spec, so paths with spaces work.
    let exe = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=opencal\nComment=Start opencal at login\nIcon={}\n\
         Exec=\"{exe}\"{}\nStartupNotify=false\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        crate::APP_ID,
        if hidden { " --background" } else { "" },
    );
    opencal::write_atomic(&path, entry.as_bytes())
}

pub fn disable() -> anyhow::Result<()> {
    let Some(path) = path() else { return Ok(()) };
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}
