//! The settings window, the sync/merge result, and the calendar dialog.

use std::rc::Rc;

use gtk::prelude::*;
use opencal::merge::Change;
use opencal::{Calendar, sync};

use super::App;
use super::widgets::{self, button_row, dialog_window, label};

fn command_view(text: &str) -> (gtk::ScrolledWindow, gtk::TextView) {
    let tv = gtk::TextView::builder()
        .monospace(true)
        .wrap_mode(gtk::WrapMode::Char)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(6)
        .right_margin(6)
        .build();
    tv.buffer().set_text(text);
    let sw = gtk::ScrolledWindow::builder()
        .child(&tv)
        .min_content_height(64)
        .hexpand(true)
        .build();
    sw.add_css_class("frame");
    (sw, tv)
}

fn text_of(tv: &gtk::TextView) -> String {
    let b = tv.buffer();
    b.text(&b.start_iter(), &b.end_iter(), false)
        .trim()
        .to_string()
}

/// Settings → Sync.
pub fn open_sync_settings(app: &Rc<App>) {
    open_settings(app, Some("sync"));
}

fn page(title: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    b.add_css_class("settings-page");
    let t = label(title);
    t.add_css_class("title-3");
    b.append(&t);
    b
}

fn section(parent: &gtk::Box, title: &str) {
    let t = label(title);
    t.add_css_class("section-title");
    parent.append(&t);
}

fn hint(parent: &gtk::Box, text: &str) {
    let l = widgets::dim_label(text);
    l.set_wrap(true);
    l.set_max_width_chars(70);
    l.add_css_class("caption");
    parent.append(&l);
}

fn check(parent: &gtk::Box, text: &str, active: bool) -> gtk::CheckButton {
    let c = gtk::CheckButton::builder()
        .label(text)
        .active(active)
        .build();
    parent.append(&c);
    c
}

/// A "label [spin] unit" row.
fn spin_row(
    parent: &gtk::Box,
    text: &str,
    min: f64,
    max: f64,
    step: f64,
    value: f64,
    unit: &str,
) -> gtk::SpinButton {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = label(text);
    l.set_hexpand(true);
    row.append(&l);
    let s = gtk::SpinButton::with_range(min, max, step);
    s.set_value(value);
    row.append(&s);
    if !unit.is_empty() {
        row.append(&widgets::dim_label(unit));
    }
    parent.append(&row);
    s
}

fn dropdown_row(parent: &gtk::Box, text: &str, items: &[&str], selected: usize) -> gtk::DropDown {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = label(text);
    l.set_hexpand(true);
    row.append(&l);
    let d = gtk::DropDown::from_strings(items);
    d.set_selected(selected as u32);
    row.append(&d);
    parent.append(&row);
    d
}

/// The settings window. `start_page`: general, calendar, reminders, sync or storage.
pub fn open_settings(app: &Rc<App>, start_page: Option<&str>) {
    let window = dialog_window(&app.window, "Settings");
    window.set_default_size(820, 600);
    let cfg = app.config.borrow().clone();
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    // --- General -------------------------------------------------------------------------
    let general = page("General");
    section(&general, "Background");
    let background = check(
        &general,
        "Keep opencal running in the tray when the window is closed",
        cfg.app.background,
    );
    hint(
        &general,
        "Reminders and timed sync keep working while the window is closed. Click the tray icon to \
         open the window again; quit from the tray menu or with Ctrl+Q. Starting opencal again \
         shows the running window. Needs a tray (Waybar's tray module, KDE, …).",
    );
    let autostart_state = crate::autostart::state();
    let autostart = check(
        &general,
        "Start opencal when I log in",
        autostart_state.is_some(),
    );
    let autostart_hidden = check(
        &general,
        "Start in the tray, without opening the window",
        autostart_state.unwrap_or(true),
    );
    autostart_hidden.set_margin_start(24);
    autostart_hidden.set_sensitive(autostart.is_active());
    {
        let h = autostart_hidden.clone();
        autostart.connect_toggled(move |c| h.set_sensitive(c.is_active()));
    }
    section(&general, "Start-up");
    let views: Vec<&str> = super::View::ALL.iter().map(|v| v.label()).collect();
    let current = super::View::ALL
        .iter()
        .position(|v| v.name() == cfg.view.view)
        .unwrap_or(2);
    let start_view = dropdown_row(&general, "Open with the view", &views, current);
    stack.add_titled(&scrolled(&general), Some("general"), "General");

    // --- Calendar ------------------------------------------------------------------------
    let calendar = page("Calendar");
    section(&calendar, "Display");
    let monday = check(
        &calendar,
        "Weeks start on Monday (otherwise Sunday)",
        cfg.view.week_starts_monday,
    );
    let day_start = spin_row(
        &calendar,
        "Day and week views scroll to",
        0.0,
        23.0,
        1.0,
        cfg.view.day_start_hour as f64,
        ":00",
    );
    let show_done = check(
        &calendar,
        "Show completed tasks in the Tasks view",
        cfg.view.show_completed,
    );
    section(&calendar, "New entries");
    let event_minutes = spin_row(
        &calendar,
        "Length of a new event",
        5.0,
        1440.0,
        5.0,
        cfg.view.event_minutes as f64,
        "minutes",
    );
    let (cal_names, cal_uids): (Vec<String>, Vec<String>) = {
        let store = app.store.borrow();
        std::iter::once(("First visible calendar".to_string(), String::new()))
            .chain(
                store
                    .live_calendars()
                    .iter()
                    .map(|c| (c.name.clone(), c.uid.clone())),
            )
            .unzip()
    };
    let names: Vec<&str> = cal_names.iter().map(String::as_str).collect();
    let sel = cal_uids
        .iter()
        .position(|u| *u == cfg.view.default_calendar)
        .unwrap_or(0);
    let default_cal = dropdown_row(&calendar, "New events and tasks go to", &names, sel);
    stack.add_titled(&scrolled(&calendar), Some("calendar"), "Calendar");

    // --- Reminders -----------------------------------------------------------------------
    let rem = page("Reminders");
    let rem_events = check(&rem, "Notify me before events", cfg.reminders.events);
    let rem_minutes = spin_row(
        &rem,
        "How long before",
        0.0,
        1440.0,
        5.0,
        cfg.reminders.minutes_before as f64,
        "minutes",
    );
    let rem_tasks = check(&rem, "Notify me when tasks are due", cfg.reminders.tasks);
    hint(
        &rem,
        "Notifications go to your notification daemon (mako, dunst, swaync, …). All-day events \
         and tasks without a time are announced at the hour set under Calendar. Reminders only \
         work while opencal runs; turn on the tray under General to get them with the window \
         closed.",
    );
    stack.add_titled(&scrolled(&rem), Some("reminders"), "Reminders");

    // --- Sync ----------------------------------------------------------------------------
    let syncp = page("Sync");
    let intro = label(
        "opencal syncs like KeePassXC: the pull command downloads the other copy of your calendar to \
         $CAL_DB_PULL, opencal merges it into $CAL_DB (additions, edits, deletions and completed \
         tasks; the newest change wins), then the push command uploads $CAL_DB. Commands run with sh \
         in the calendar's folder; they can be scripts.",
    );
    intro.set_wrap(true);
    intro.set_max_width_chars(80);
    syncp.append(&intro);
    section(&syncp, "Automatic sync");
    let on_start = check(&syncp, "When opencal starts", cfg.sync.on_start);
    let every = gtk::CheckButton::builder()
        .label("Every")
        .active(cfg.sync.interval_minutes > 0)
        .build();
    let interval = gtk::SpinButton::with_range(1.0, 1440.0, 5.0);
    interval.set_value(cfg.sync.interval_minutes.max(15) as f64);
    interval.set_sensitive(every.is_active());
    {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&every);
        row.append(&interval);
        row.append(&widgets::dim_label("minutes"));
        syncp.append(&row);
        let i = interval.clone();
        every.connect_toggled(move |c| i.set_sensitive(c.is_active()));
    }
    let after = gtk::CheckButton::builder()
        .label("After changes, once nothing changed for")
        .active(cfg.sync.after_change_seconds > 0)
        .build();
    let after_secs = gtk::SpinButton::with_range(5.0, 3600.0, 5.0);
    after_secs.set_value(if cfg.sync.after_change_seconds > 0 {
        cfg.sync.after_change_seconds
    } else {
        30
    } as f64);
    after_secs.set_sensitive(after.is_active());
    {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&after);
        row.append(&after_secs);
        row.append(&widgets::dim_label("seconds"));
        syncp.append(&row);
        let a = after_secs.clone();
        after.connect_toggled(move |c| a.set_sensitive(c.is_active()));
    }
    hint(
        &syncp,
        "Timed sync runs while opencal runs, also in the tray. You can always sync with F5, or from cron with `opencal --sync`.",
    );

    section(&syncp, "Commands");
    let presets =
        gtk::DropDown::from_strings(&sync::PRESETS.iter().map(|p| p.0).collect::<Vec<_>>());
    let use_preset = gtk::Button::with_label("Use Example");
    let preset_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    preset_row.append(&label("Examples:"));
    presets.set_hexpand(true);
    preset_row.append(&presets);
    preset_row.append(&use_preset);
    syncp.append(&preset_row);
    syncp.append(&label(
        "Pull (download to $CAL_DB_PULL; leave the file absent if there is no remote copy yet)",
    ));
    let (pull_sw, pull) = command_view(&cfg.sync.pull);
    syncp.append(&pull_sw);
    syncp.append(&label("Push (upload $CAL_DB)"));
    let (push_sw, push) = command_view(&cfg.sync.push);
    syncp.append(&push_sw);
    {
        let (pull, push, presets) = (pull.clone(), push.clone(), presets.clone());
        use_preset.connect_clicked(move |_| {
            if let Some(p) = sync::PRESETS.get(presets.selected() as usize) {
                pull.buffer().set_text(p.1);
                push.buffer().set_text(p.2);
            }
        });
    }
    let always_push = check(
        &syncp,
        "Push even when there is nothing new to send",
        cfg.sync.always_push,
    );
    let timeout = spin_row(
        &syncp,
        "Stop a command after",
        5.0,
        3600.0,
        5.0,
        cfg.sync.timeout_seconds as f64,
        "seconds",
    );
    let mut vars = String::new();
    for (k, d) in sync::VARIABLES {
        vars.push_str(&format!("${k:<18} {d}\n"));
    }
    let vars_label = label(vars.trim_end());
    vars_label.add_css_class("monospace");
    vars_label.add_css_class("caption");
    vars_label.set_selectable(true);
    syncp.append(
        &gtk::Expander::builder()
            .label("Environment variables for the commands")
            .child(&vars_label)
            .build(),
    );
    stack.add_titled(&scrolled(&syncp), Some("sync"), "Sync");

    // --- Storage -------------------------------------------------------------------------
    let storage = page("Storage");
    section(&storage, "Calendar file");
    let path_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let path_entry = gtk::Entry::builder()
        .text(app.db_path().display().to_string())
        .hexpand(true)
        .build();
    path_entry.add_css_class("monospace");
    let choose = gtk::Button::with_label("Choose…");
    let open_folder = gtk::Button::with_label("Open Folder");
    path_row.append(&path_entry);
    path_row.append(&choose);
    path_row.append(&open_folder);
    storage.append(&path_row);
    hint(
        &storage,
        "A plain iCalendar file. Another existing opencal file is opened as it is; a new path is \
         created with the current calendar. Leave the default unless you keep the file somewhere \
         special (a synced folder, a git checkout).",
    );
    {
        let (entry, w) = (path_entry.clone(), window.clone());
        choose.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title("Calendar File")
                .modal(true)
                .initial_name("calendar.ics")
                .build();
            let entry = entry.clone();
            dialog.save(Some(&w), gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(Some(p)) = res.map(|f| f.path()) {
                    entry.set_text(&p.display().to_string());
                }
            });
        });
        let entry = path_entry.clone();
        open_folder.connect_clicked(move |_| {
            let p = std::path::PathBuf::from(entry.text().as_str());
            if let Some(dir) = p.parent() {
                let uri = gtk::gio::File::for_path(dir).uri();
                let _ = gtk::gio::AppInfo::launch_default_for_uri(
                    &uri,
                    gtk::gio::AppLaunchContext::NONE,
                );
            }
        });
    }
    section(&storage, "Deleted entries");
    let tomb = spin_row(
        &storage,
        "Forget deleted entries after (0 = never)",
        0.0,
        3650.0,
        30.0,
        cfg.storage.tombstone_days as f64,
        "days",
    );
    hint(
        &storage,
        "Deleted events and tasks stay in the file as small markers so that a sync can remove them \
         on your other devices too. A device that has not synced for longer than this may bring \
         them back.",
    );
    stack.add_titled(&scrolled(&storage), Some("storage"), "Storage");

    // --- Layout ---------------------------------------------------------------------------
    let side = gtk::StackSidebar::new();
    side.set_stack(&stack);
    side.add_css_class("settings-sidebar");
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&side);
    body.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    body.append(&stack);
    if let Some(p) = start_page {
        stack.set_visible_child_name(p);
    }
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&body);
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let sync_now = gtk::Button::with_label("Save and Sync Now");
    let buttons = button_row(None, &[&cancel, &sync_now, &save]);
    buttons.add_css_class("dialog-content");
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&buttons);
    window.set_child(Some(&widgets::titled(&window, &content)));
    window.set_default_widget(Some(&save));
    {
        let w = window.clone();
        cancel.connect_clicked(move |_| w.close());
    }

    let apply = {
        let app = app.clone();
        let w = window.clone();
        move || {
            let mut c = app.config.borrow().clone();
            c.app.background = background.is_active();
            c.view.view = super::View::ALL[start_view.selected() as usize % 5]
                .name()
                .to_string();
            c.view.week_starts_monday = monday.is_active();
            c.view.day_start_hour = day_start.value() as u32;
            c.view.show_completed = show_done.is_active();
            c.view.event_minutes = event_minutes.value() as u32;
            c.view.default_calendar = cal_uids
                .get(default_cal.selected() as usize)
                .cloned()
                .unwrap_or_default();
            c.reminders.events = rem_events.is_active();
            c.reminders.minutes_before = rem_minutes.value() as u32;
            c.reminders.tasks = rem_tasks.is_active();
            c.sync.on_start = on_start.is_active();
            c.sync.interval_minutes = if every.is_active() {
                interval.value() as u32
            } else {
                0
            };
            c.sync.after_change_seconds = if after.is_active() {
                after_secs.value() as u32
            } else {
                0
            };
            c.sync.pull = text_of(&pull);
            c.sync.push = text_of(&push);
            c.sync.always_push = always_push.is_active();
            c.sync.timeout_seconds = timeout.value() as u64;
            c.storage.tombstone_days = tomb.value() as i64;
            let new_path = path_entry.text().trim().to_string();
            let default_path = app
                .paths
                .data_dir
                .join("calendar.ics")
                .display()
                .to_string();
            let path_changed = new_path != app.db_path().display().to_string();
            c.storage.path = if new_path.is_empty() || new_path == default_path {
                String::new()
            } else {
                new_path.clone()
            };

            let autostart_result = if autostart.is_active() {
                crate::autostart::enable(autostart_hidden.is_active())
            } else {
                crate::autostart::disable()
            };
            if let Err(e) = autostart_result {
                widgets::error(
                    &w,
                    "Could not change the login start-up entry",
                    &format!("{e:#}"),
                );
            }
            *app.config.borrow_mut() = c;
            if let Err(e) = app.config.borrow().save(&app.paths) {
                widgets::error(
                    &app.window,
                    "Could not save the settings",
                    &format!("{e:#}"),
                );
            }
            if path_changed {
                let target = app.paths.db(&app.config.borrow());
                if !target.exists() {
                    // A new location starts with the current calendar.
                    let _ = opencal::save_store(&target, &app.store.borrow());
                }
                if let Err(e) = app.set_db_path(target.clone()) {
                    widgets::error(
                        &app.window,
                        "Could not open the calendar file",
                        &format!("{}: {e:#}", target.display()),
                    );
                }
            }
            app.apply_background();
            app.restart_sync_timer();
            app.refresh();
        }
    };
    let apply = Rc::new(apply);
    {
        let (w, apply) = (window.clone(), apply.clone());
        save.connect_clicked(move |_| {
            apply();
            w.close();
        });
    }
    {
        let (w, app) = (window.clone(), app.clone());
        sync_now.connect_clicked(move |_| {
            apply();
            w.close();
            app.sync_now(true);
        });
    }
    window.present();
}

fn scrolled(page: &gtk::Box) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(page)
        .build()
}

fn change_list(title: &str, changes: &[Change]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    b.set_hexpand(true);
    let t = label(&format!("{title} ({})", changes.len()));
    t.add_css_class("section-title");
    b.append(&t);
    let list = gtk::ListBox::new();
    list.add_css_class("panel");
    list.set_selection_mode(gtk::SelectionMode::None);
    if changes.is_empty() {
        list.append(
            &gtk::ListBoxRow::builder()
                .activatable(false)
                .child(&widgets::dim_label("Nothing"))
                .build(),
        );
    }
    for c in changes {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let kind = gtk::Label::new(Some(&c.kind.to_string()));
        kind.add_css_class("badge");
        kind.add_css_class(&format!("change-{}", c.kind));
        row.append(&kind);
        let subject = widgets::dim_label(match c.subject {
            opencal::merge::Subject::Calendar => "calendar",
            opencal::merge::Subject::Event => "event",
            opencal::merge::Subject::Task => "task",
        });
        row.append(&subject);
        let s = label(&c.summary);
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&s);
        list.append(
            &gtk::ListBoxRow::builder()
                .activatable(false)
                .child(&row)
                .build(),
        );
    }
    let sw = gtk::ScrolledWindow::builder()
        .child(&list)
        .min_content_height(160)
        .max_content_height(320)
        .propagate_natural_height(true)
        .build();
    b.append(&sw);
    b
}

/// What a sync or merge did: the changes both ways and the command output.
pub fn show_sync_result(app: &Rc<App>, what: &str, result: &sync::SyncResult) {
    let window = dialog_window(&app.window, &format!("{what} Result"));
    window.set_default_size(720, -1);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("dialog-content");
    let heading = label(&match &result.error {
        Some(e) => format!("{what} failed: {e}"),
        None if what == "Merge" => format!("Merged: {}", result.report.summary()),
        None => result.status_line(),
    });
    heading.add_css_class("title-3");
    heading.set_wrap(true);
    if result.error.is_some() {
        heading.add_css_class("error-text");
    }
    content.append(&heading);
    let lists = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    lists.set_homogeneous(true);
    lists.append(&change_list(
        if what == "Merge" {
            "From the file"
        } else {
            "Incoming (from remote)"
        },
        &result.report.incoming,
    ));
    lists.append(&change_list(
        if what == "Merge" {
            "Not in the file"
        } else {
            "Outgoing (pushed)"
        },
        &result.report.outgoing,
    ));
    content.append(&lists);
    if !result.logs.is_empty() {
        let tv = gtk::TextView::builder()
            .monospace(true)
            .editable(false)
            .wrap_mode(gtk::WrapMode::Char)
            .left_margin(6)
            .top_margin(6)
            .build();
        tv.buffer().set_text(&result.log_text());
        let sw = gtk::ScrolledWindow::builder()
            .child(&tv)
            .min_content_height(140)
            .build();
        sw.add_css_class("frame");
        let exp = gtk::Expander::builder()
            .label("Command output")
            .child(&sw)
            .expanded(result.error.is_some())
            .build();
        content.append(&exp);
    }
    let close = gtk::Button::with_label("Close");
    close.add_css_class("suggested-action");
    let settings = gtk::Button::with_label("Sync Settings…");
    let left: Option<&gtk::Widget> = if what == "Sync" {
        Some(settings.upcast_ref())
    } else {
        None
    };
    content.append(&button_row(left, &[&close]));
    window.set_child(Some(&widgets::titled(&window, &content)));
    window.set_default_widget(Some(&close));
    {
        let w = window.clone();
        close.connect_clicked(move |_| w.close());
    }
    {
        let (w, app) = (window.clone(), app.clone());
        settings.connect_clicked(move |_| {
            w.close();
            open_sync_settings(&app);
        });
    }
    window.present();
}

/// Add (uid = None) or edit a calendar.
pub fn open_calendar(app: &Rc<App>, uid: Option<&str>) {
    let existing = uid.and_then(|u| app.store.borrow().calendars.get(u).cloned());
    let n = app.store.borrow().calendars.len();
    let cal = existing.clone().unwrap_or_else(|| {
        Calendar::new(
            "",
            opencal::model::CALENDAR_COLORS[n % opencal::model::CALENDAR_COLORS.len()],
        )
    });
    let window = dialog_window(
        &app.window,
        if existing.is_some() {
            "Edit Calendar"
        } else {
            "New Calendar"
        },
    );
    window.set_default_size(400, -1);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("dialog-content");
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .build();
    let name = gtk::Entry::builder()
        .text(&cal.name)
        .placeholder_text("Calendar name")
        .hexpand(true)
        .activates_default(true)
        .build();
    let color = gtk::ColorDialogButton::new(Some(
        gtk::ColorDialog::builder()
            .with_alpha(false)
            .modal(true)
            .build(),
    ));
    color.set_rgba(&widgets::parse_hex(&cal.color));
    // Quick picks.
    let picks = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    for hex in opencal::model::CALENDAR_COLORS {
        let b = gtk::Button::new();
        b.add_css_class("color-pick");
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&format!(
            "button {{ background: {hex}; min-width: 20px; min-height: 20px; padding: 0; }}"
        ));
        #[allow(deprecated)]
        b.style_context()
            .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_USER + 3);
        b.set_tooltip_text(Some(hex));
        let c = color.clone();
        b.connect_clicked(move |_| c.set_rgba(&widgets::parse_hex(hex)));
        picks.append(&b);
    }
    grid.attach(&label("Name"), 0, 0, 1, 1);
    grid.attach(&name, 1, 0, 1, 1);
    grid.attach(&label("Colour"), 0, 1, 1, 1);
    let color_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    color_row.append(&color);
    color_row.append(&picks);
    grid.attach(&color_row, 1, 1, 1, 1);
    content.append(&grid);
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label(if existing.is_some() { "Save" } else { "Add" });
    save.add_css_class("suggested-action");
    let delete = gtk::Button::with_label("Delete Calendar");
    delete.add_css_class("destructive-action");
    let can_delete = existing.is_some() && app.store.borrow().live_calendars().len() > 1;
    content.append(&button_row(
        if can_delete {
            Some(delete.upcast_ref())
        } else {
            None
        },
        &[&cancel, &save],
    ));
    window.set_child(Some(&widgets::titled(&window, &content)));
    window.set_default_widget(Some(&save));
    {
        let w = window.clone();
        cancel.connect_clicked(move |_| w.close());
    }
    {
        let (w, app) = (window.clone(), app.clone());
        let cal = cal.clone();
        let name = name.clone();
        save.connect_clicked(move |_| {
            let n = name.text().trim().to_string();
            if n.is_empty() {
                name.grab_focus();
                return;
            }
            let mut c = cal.clone();
            c.name = n;
            c.color = widgets::to_hex(&color.rgba());
            if existing.as_ref() != Some(&c) {
                c.modified = opencal::model::now().max(c.modified);
                app.change(|s| s.add_calendar(c.clone()));
            }
            w.close();
        });
    }
    {
        let (w, app) = (window.clone(), app.clone());
        let (uid, cname) = (cal.uid.clone(), cal.name.clone());
        delete.connect_clicked(move |_| {
            let count = app
                .store
                .borrow()
                .live_items()
                .filter(|i| i.calendar == uid)
                .count();
            let (app, w2, uid) = (app.clone(), w.clone(), uid.clone());
            widgets::confirm(
                &w,
                &format!("Delete the calendar \"{cname}\"?"),
                &format!("Its {count} events and tasks are deleted too."),
                "Delete",
                move || {
                    app.change(|s| s.delete_calendar(&uid));
                    w2.close();
                },
            );
        });
    }
    window.present();
    name.grab_focus();
}
