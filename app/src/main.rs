#![allow(clippy::type_complexity, clippy::explicit_counter_loop)]

mod ui;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use chrono::Duration as CDuration;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use opencal::{Config, Item, Paths, When, ics, sync};

const APP_ID: &str = "org.opencal.OpenCal";

const HELP: &str = "\
opencal - local calendar and task manager

Usage: opencal [--profile DIR] [COMMAND]

Without a command, opens the calendar window.

Commands:
  --sync                 Run the configured sync (pull, merge, push) and print the changes
  --merge FILE           Merge another opencal calendar file into yours
  --import FILE          Import an .ics file (into a new calendar named after the file)
  --export FILE          Export your events and tasks as plain .ics (\"-\" = standard output)
  --agenda [DAYS]        Print the next DAYS days (default 7)
  --add-task TEXT        Add a task to the first calendar
  --path                 Print the calendar file's path
  --self-test            Check the store and build every view without showing them
  --screenshot FILE [VIEW]
                         Save a PNG of the window with example data
                         (VIEW: day, week, month, agenda, tasks)
  -h, --help             Show this help
  -V, --version          Show the version

Options:
  --profile DIR          Keep settings and the calendar in DIR (also: OPENCAL_PROFILE)

Files:
  ~/.config/opencal/config.toml     settings, including the sync commands
  ~/.local/share/opencal/calendar.ics   the calendar (iCalendar)
";

fn main() -> glib::ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut profile = None;
    if let Some(i) = args.iter().position(|a| a == "--profile") {
        if i + 1 >= args.len() {
            eprintln!("--profile needs a directory");
            return glib::ExitCode::FAILURE;
        }
        profile = Some(PathBuf::from(args.remove(i + 1)));
        args.remove(i);
    }
    let paths = Paths::new(profile.as_deref());
    let arg = |i: usize| args.get(i).map(String::as_str);
    let result = match arg(0) {
        None => {
            return run_gui(
                paths,
                profile.is_some()
                    || std::env::var_os("OPENCAL_PROFILE").is_some_and(|p| !p.is_empty()),
            );
        }
        Some("-h" | "--help") => {
            print!("{HELP}");
            Ok(())
        }
        Some("-V" | "--version") => {
            println!("opencal {}", opencal::VERSION);
            Ok(())
        }
        Some("--path") => Config::load(&paths).map(|c| println!("{}", paths.db(&c).display())),
        Some("--sync") => cli_sync(&paths),
        Some("--merge") => match arg(1) {
            Some(f) => cli_merge(&paths, Path::new(f)),
            None => Err(anyhow::anyhow!("--merge needs a file")),
        },
        Some("--import") => match arg(1) {
            Some(f) => cli_import(&paths, Path::new(f)),
            None => Err(anyhow::anyhow!("--import needs a file")),
        },
        Some("--export") => match arg(1) {
            Some(f) => cli_export(&paths, f),
            None => Err(anyhow::anyhow!("--export needs a file")),
        },
        Some("--agenda") => cli_agenda(&paths, arg(1).and_then(|d| d.parse().ok()).unwrap_or(7)),
        Some("--add-task") => match arg(1) {
            Some(t) => cli_add_task(&paths, t),
            None => Err(anyhow::anyhow!("--add-task needs a text")),
        },
        Some("--self-test") => return self_test(),
        Some("--screenshot") => match arg(1) {
            Some(f) => return screenshot(Path::new(f), arg(2).unwrap_or("month")),
            None => Err(anyhow::anyhow!("--screenshot needs a file")),
        },
        Some(other) => Err(anyhow::anyhow!("unknown argument {other:?} (see --help)")),
    };
    match result {
        Ok(()) => glib::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("opencal: {e:#}");
            glib::ExitCode::FAILURE
        }
    }
}

fn load(paths: &Paths) -> anyhow::Result<(Config, PathBuf, opencal::Store)> {
    let config = Config::load(paths)?;
    let db = paths.db(&config);
    let store = sync::load_store(&db)?;
    Ok((config, db, store))
}

fn cli_sync(paths: &Paths) -> anyhow::Result<()> {
    let config = Config::load(paths)?;
    let result = sync::sync(paths, &config);
    eprint!("{}", result.log_text());
    print!("{}", result.report.to_text());
    match result.error {
        Some(e) => Err(anyhow::anyhow!(e)),
        None => {
            println!("{}", result.status_line());
            Ok(())
        }
    }
}

fn cli_merge(paths: &Paths, file: &Path) -> anyhow::Result<()> {
    let (_, db, _) = load(paths)?;
    let report = sync::merge_file(&db, file)?;
    print!("{}", report.to_text());
    Ok(())
}

fn cli_import(paths: &Paths, file: &Path) -> anyhow::Result<()> {
    let (_, db, mut store) = load(paths)?;
    let text = std::fs::read_to_string(file)?;
    let parsed = ics::parse(&text)?;
    let target = if parsed.calendars.is_empty() {
        let name = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Imported".into());
        let n = store.calendars.len();
        let cal = opencal::Calendar::new(
            &name,
            opencal::model::CALENDAR_COLORS[n % opencal::model::CALENDAR_COLORS.len()],
        );
        let uid = cal.uid.clone();
        store.add_calendar(cal);
        uid
    } else {
        store.live_calendars()[0].uid.clone()
    };
    let n = ics::import(&mut store, &text, &target)?;
    opencal::save_store(&db, &store)?;
    println!("Imported {n} entries");
    Ok(())
}

fn cli_export(paths: &Paths, file: &str) -> anyhow::Result<()> {
    let (_, _, store) = load(paths)?;
    let text = ics::write_export(&store);
    if file == "-" {
        print!("{text}");
        Ok(())
    } else {
        opencal::write_atomic(Path::new(file), text.as_bytes())
    }
}

fn cli_agenda(paths: &Paths, days: i64) -> anyhow::Result<()> {
    let (config, _, store) = load(paths)?;
    let from = ui::today();
    let hidden = config.view.hidden_calendars.iter().cloned().collect();
    let occ =
        opencal::recur::occurrences(&store, from, from + CDuration::days(days.max(1)), &hidden);
    for i in 0..days.max(1) {
        let day = from + CDuration::days(i);
        let todays: Vec<_> = occ.iter().filter(|o| o.on_day(day)).collect();
        if todays.is_empty() {
            continue;
        }
        println!("{}", day.format("%a %-d %b %Y"));
        for o in todays {
            let when = if o.all_day || o.end - o.start >= CDuration::hours(24) {
                "all day    ".to_string()
            } else if o.kind == opencal::Kind::Task {
                format!("due {}  ", o.start.format("%H:%M"))
            } else {
                format!("{}-{}", o.start.format("%H:%M"), o.end.format("%H:%M"))
            };
            let mark = match (o.kind, o.completed) {
                (opencal::Kind::Task, true) => "[x] ",
                (opencal::Kind::Task, false) => "[ ] ",
                _ => "",
            };
            println!("  {when}  {mark}{}", o.summary);
        }
    }
    Ok(())
}

fn cli_add_task(paths: &Paths, text: &str) -> anyhow::Result<()> {
    let (_, db, mut store) = load(paths)?;
    let cal = store.live_calendars()[0].uid.clone();
    store.add_item(Item::new_task(&cal, text, None));
    opencal::save_store(&db, &store)
}

fn load_css() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../../data/style.css"));
    #[allow(deprecated)]
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn run_gui(paths: Paths, separate: bool) -> glib::ExitCode {
    // A profile is a separate calendar: don't hand over to an instance showing another one.
    let flags = if separate {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        gio::ApplicationFlags::empty()
    };
    let application = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();
    application.connect_startup(|_| load_css());
    let paths = Rc::new(paths);
    application.connect_activate(move |application| {
        if let Some(w) = application.active_window() {
            w.present();
            return;
        }
        let app = ui::App::new(application, (*paths).clone());
        app.present();
    });
    application.run_with_args(&["opencal"])
}

fn temp_profile() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("opencal-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn offline_application() -> gtk::Application {
    gtk::Application::builder()
        .application_id(format!("{APP_ID}.Test"))
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build()
}

/// Store checks that need no display, then every view built (not shown) when there is one.
/// Exit code 77 = skipped (no display).
fn self_test() -> glib::ExitCode {
    let check = || -> anyhow::Result<()> {
        let mut store = opencal::Store::new();
        let cal = store.live_calendars()[0].uid.clone();
        let day = ui::today();
        let at = |h| When::DateTime(day.and_hms_opt(h, 0, 0).unwrap());
        let mut e = Item::new_event(&cal, "Self-test, event; with \\ escapes", at(10), at(11));
        e.rrule = Some("FREQ=DAILY;COUNT=3".into());
        store.add_item(e);
        store.add_item(Item::new_task(
            &cal,
            "Self-test task",
            Some(When::Date(day)),
        ));
        let text = ics::write(&store);
        let back = ics::parse(&text)?;
        anyhow::ensure!(back == store, "round trip changed the store");
        let occ =
            opencal::recur::occurrences(&back, day, day + CDuration::days(7), &Default::default());
        anyhow::ensure!(occ.len() == 4, "expected 4 occurrences, got {}", occ.len());
        let mut other = back.clone();
        let uid = other.items.keys().next().unwrap().clone();
        other.delete_item(&uid);
        let (merged, report) = opencal::merge::merge(&store, &other);
        anyhow::ensure!(
            merged.items[&uid].is_deleted() && report.incoming.len() == 1,
            "merge lost a deletion"
        );
        Ok(())
    };
    if let Err(e) = check() {
        eprintln!("self-test failed: {e:#}");
        return glib::ExitCode::FAILURE;
    }
    println!("store checks: ok");
    if gtk::init().is_err() {
        eprintln!("self-test: GUI checks skipped (no display)");
        return glib::ExitCode::from(77);
    }
    let profile = temp_profile();
    let ok = Rc::new(Cell::new(false));
    let application = offline_application();
    {
        let ok = ok.clone();
        let profile = profile.clone();
        application.connect_activate(move |application| {
            load_css();
            let app = ui::App::new(application, Paths::new(Some(&profile)));
            app.load_demo();
            for v in ui::View::ALL {
                app.set_view(v);
            }
            let day = ui::today();
            app.go_to(day + CDuration::days(40));
            app.go_to(day);
            ok.set(app.store.borrow().live_items().count() >= 10);
            for w in gtk::Window::list_toplevels() {
                if let Ok(w) = w.downcast::<gtk::Window>() {
                    w.destroy();
                }
            }
            application.quit();
        });
    }
    application.run_with_args(&["opencal"]);
    let _ = std::fs::remove_dir_all(&profile);
    if ok.get() {
        println!("GUI checks: ok");
        glib::ExitCode::SUCCESS
    } else {
        eprintln!("self-test failed: GUI checks");
        glib::ExitCode::FAILURE
    }
}

fn save_window_png(window: &gtk::ApplicationWindow, file: &Path) -> Result<(), String> {
    let (width, height) = (window.width(), window.height());
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let node = snapshot
        .to_node()
        .ok_or_else(|| format!("The window drew nothing ({width}x{height})"))?;
    let renderer = gtk::gsk::CairoRenderer::new();
    #[allow(deprecated)]
    renderer
        .realize(None::<&gdk::Surface>)
        .map_err(|err| format!("Could not start the renderer: {err}"))?;
    let bounds = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    let texture = renderer.render_texture(&node, Some(&bounds));
    renderer.unrealize();
    texture
        .save_to_png(file)
        .map_err(|err| format!("Could not save {}: {err}", file.display()))
}

/// Render the window with example data to a PNG (for the README).
fn screenshot(file: &Path, view: &str) -> glib::ExitCode {
    let Some(view) = ui::View::from_name(view) else {
        eprintln!("Unknown view {view:?}");
        return glib::ExitCode::FAILURE;
    };
    let file = file.to_path_buf();
    let profile = temp_profile();
    let ok = Rc::new(Cell::new(false));
    let application = offline_application();
    {
        let ok = ok.clone();
        let profile = profile.clone();
        application.connect_activate(move |application| {
            load_css();
            let app = ui::App::new(application, Paths::new(Some(&profile)));
            app.window.set_default_size(1200, 780);
            app.load_demo();
            app.set_view(view);
            app.window.present();
            let (file, ok) = (file.clone(), ok.clone());
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                let Some(clock) = app.window.frame_clock() else {
                    eprintln!("The window has no frame clock");
                    app.window.close();
                    return;
                };
                let done = Rc::new(Cell::new(false));
                let a = app.clone();
                clock.connect_after_paint(move |_| {
                    if done.replace(true) {
                        return;
                    }
                    match save_window_png(&a.window, &file) {
                        Ok(()) => ok.set(true),
                        Err(e) => eprintln!("{e}"),
                    }
                    a.window.close();
                });
                app.window.queue_draw();
            });
        });
    }
    application.run_with_args(&["opencal"]);
    let _ = std::fs::remove_dir_all(&profile);
    if ok.get() {
        glib::ExitCode::SUCCESS
    } else {
        glib::ExitCode::FAILURE
    }
}
