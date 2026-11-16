# Changelog

## 0.1.0

First release.

- Added: GTK 4 calendar with Day, Week, Month, Agenda and Tasks views, a month calendar and the list of
  calendars in the sidebar.
- Added: several calendars with colours; events (timed, all-day, multi-day, repeating) and tasks (due date,
  priority, completion).
- Added: drag in the day and week views to create, move and resize events; double-click to create one.
- Added: the calendar is stored in one iCalendar file (`~/.local/share/opencal/calendar.ics`) that keeps
  deletions and completion times, so two copies can be merged.
- Added: sync with your own pull and push commands, with `$CAL_DB`, `$CAL_DB_PULL`, `$CAL_SYNC_REPORT` and
  other variables, on demand, at start-up or every N minutes. The result lists the changes both ways.
- Added: example sync scripts for rclone, git, scp and WebDAV.
- Added: import and export of `.ics` files, and merging another opencal file.
- Added: command line: `--sync`, `--merge`, `--import`, `--export`, `--agenda`, `--add-task`, `--path`.
- Added: `libopencal.so` with a C header and pkg-config file, and the `opencal` Rust crate.
- Added: Settings window (General, Calendar, Reminders, Sync, Storage).
- Added: running in the background with a tray icon, start at login (`--background`), and a single instance:
  starting opencal again shows the running window.
- Added: reminder notifications before events and when tasks are due.
- Added: automatic sync after changes, besides at start and every N minutes.
- Added: right-click menus on days, hours and entries; the sidebar can be resized.
- Added: menus are painted with the GTK theme's colours.
