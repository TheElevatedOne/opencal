//! Sync settings, the sync/merge result, and the calendar dialog.

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

pub fn open_sync_settings(app: &Rc<App>) {
    let window = dialog_window(&app.window, "Sync Settings");
    window.set_default_size(720, -1);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.add_css_class("dialog-content");
    let cfg = app.config.borrow().sync.clone();

    let intro = label(
        "opencal syncs like KeePassXC: the pull command downloads the other copy of your calendar to \
         $CAL_DB_PULL, opencal merges it into $CAL_DB (additions, edits, deletions and completed tasks, \
         newest change wins), then the push command uploads $CAL_DB. Commands run with sh in the calendar's \
         folder; they can be scripts.",
    );
    intro.set_wrap(true);
    intro.set_max_width_chars(80);
    content.append(&intro);

    let presets =
        gtk::DropDown::from_strings(&sync::PRESETS.iter().map(|p| p.0).collect::<Vec<_>>());
    let use_preset = gtk::Button::with_label("Use Example");
    let preset_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    preset_row.append(&label("Examples:"));
    presets.set_hexpand(true);
    preset_row.append(&presets);
    preset_row.append(&use_preset);
    content.append(&preset_row);

    let pull_title = label(
        "Pull command (downloads to $CAL_DB_PULL; leave the file absent if there is no remote copy yet)",
    );
    pull_title.add_css_class("section-title");
    content.append(&pull_title);
    let (pull_sw, pull) = command_view(&cfg.pull);
    content.append(&pull_sw);
    let push_title = label("Push command (uploads $CAL_DB)");
    push_title.add_css_class("section-title");
    content.append(&push_title);
    let (push_sw, push) = command_view(&cfg.push);
    content.append(&push_sw);
    {
        let (pull, push, presets) = (pull.clone(), push.clone(), presets.clone());
        use_preset.connect_clicked(move |_| {
            if let Some(p) = sync::PRESETS.get(presets.selected() as usize) {
                pull.buffer().set_text(p.1);
                push.buffer().set_text(p.2);
            }
        });
    }

    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .build();
    let on_start = gtk::CheckButton::builder()
        .label("Sync when opencal starts")
        .active(cfg.on_start)
        .build();
    let always_push = gtk::CheckButton::builder()
        .label("Push even when there is nothing new to send")
        .active(cfg.always_push)
        .build();
    let interval = gtk::SpinButton::with_range(0.0, 1440.0, 5.0);
    interval.set_value(cfg.interval_minutes as f64);
    let timeout = gtk::SpinButton::with_range(5.0, 3600.0, 5.0);
    timeout.set_value(cfg.timeout_seconds as f64);
    grid.attach(&on_start, 0, 0, 2, 1);
    grid.attach(&always_push, 0, 1, 2, 1);
    grid.attach(&label("Sync every (minutes, 0 = off)"), 0, 2, 1, 1);
    grid.attach(&interval, 1, 2, 1, 1);
    grid.attach(&label("Command timeout (seconds)"), 0, 3, 1, 1);
    grid.attach(&timeout, 1, 3, 1, 1);
    content.append(&grid);

    let mut vars = String::from("Variables:\n");
    for (k, d) in sync::VARIABLES {
        vars.push_str(&format!("  ${k:<18} {d}\n"));
    }
    vars.push_str(&format!("\nCalendar file: {}", app.db_path().display()));
    let vars_label = label(vars.trim_end());
    vars_label.add_css_class("monospace");
    vars_label.add_css_class("caption");
    vars_label.set_selectable(true);
    let expander = gtk::Expander::builder()
        .label("Environment variables")
        .child(&vars_label)
        .build();
    content.append(&expander);

    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label("Save");
    let save_sync = gtk::Button::with_label("Save and Sync");
    save_sync.add_css_class("suggested-action");
    content.append(&button_row(None, &[&cancel, &save, &save_sync]));
    window.set_child(Some(&widgets::titled(&window, &content)));

    {
        let w = window.clone();
        cancel.connect_clicked(move |_| w.close());
    }
    let apply = {
        let app = app.clone();
        move || {
            {
                let mut c = app.config.borrow_mut();
                c.sync.pull = text_of(&pull);
                c.sync.push = text_of(&push);
                c.sync.on_start = on_start.is_active();
                c.sync.always_push = always_push.is_active();
                c.sync.interval_minutes = interval.value() as u32;
                c.sync.timeout_seconds = timeout.value() as u64;
                if let Err(e) = c.save(&app.paths) {
                    widgets::error(
                        &app.window,
                        "Could not save the settings",
                        &format!("{e:#}"),
                    );
                }
            }
            app.restart_sync_timer();
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
        save_sync.connect_clicked(move |_| {
            apply();
            w.close();
            app.sync_now(true);
        });
    }
    window.present();
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
