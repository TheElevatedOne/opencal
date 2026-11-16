//! The event and task dialogs.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use gtk::prelude::*;
use opencal::{Item, When};

use super::widgets::{self, DatePicker, TimePicker, button_row, dialog_window, label};
use super::{App, today};

const FREQS: [&str; 6] = [
    "Does not repeat",
    "Daily",
    "Weekly",
    "Monthly",
    "Yearly",
    "Custom rule",
];
const FREQ_CODES: [&str; 5] = ["", "DAILY", "WEEKLY", "MONTHLY", "YEARLY"];
const UNITS: [&str; 5] = ["", "day(s)", "week(s)", "month(s)", "year(s)"];
const WEEKDAYS: [(Weekday, &str, &str); 7] = [
    (Weekday::Mon, "MO", "Mo"),
    (Weekday::Tue, "TU", "Tu"),
    (Weekday::Wed, "WE", "We"),
    (Weekday::Thu, "TH", "Th"),
    (Weekday::Fri, "FR", "Fr"),
    (Weekday::Sat, "SA", "Sa"),
    (Weekday::Sun, "SU", "Su"),
];

/// A repeat rule in the shape the simple editor can show.
#[derive(Debug, Default, PartialEq)]
struct SimpleRule {
    freq: usize,
    interval: u32,
    days: Vec<String>,
    count: Option<u32>,
    until: Option<NaiveDate>,
}

fn parse_simple(rule: &str) -> Option<SimpleRule> {
    let mut map = BTreeMap::new();
    for part in rule.split(';').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=')?;
        map.insert(k.to_ascii_uppercase(), v.to_string());
    }
    let freq = FREQ_CODES.iter().position(|f| {
        !f.is_empty() && map.get("FREQ").is_some_and(|v| v.eq_ignore_ascii_case(f))
    })?;
    let mut r = SimpleRule {
        freq,
        interval: 1,
        ..Default::default()
    };
    for (k, v) in &map {
        match k.as_str() {
            "FREQ" | "WKST" => {}
            "INTERVAL" => r.interval = v.parse().ok()?,
            "COUNT" => r.count = Some(v.parse().ok()?),
            "UNTIL" => r.until = Some(NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d").ok()?),
            "BYDAY" if freq == 2 => {
                for d in v.split(',') {
                    if !WEEKDAYS.iter().any(|w| w.1 == d) {
                        return None;
                    }
                    r.days.push(d.to_string());
                }
            }
            _ => return None,
        }
    }
    Some(r)
}

fn build_simple(r: &SimpleRule) -> String {
    let mut s = format!("FREQ={}", FREQ_CODES[r.freq]);
    if r.interval > 1 {
        s.push_str(&format!(";INTERVAL={}", r.interval));
    }
    if r.freq == 2 && !r.days.is_empty() {
        s.push_str(&format!(";BYDAY={}", r.days.join(",")));
    }
    if let Some(c) = r.count {
        s.push_str(&format!(";COUNT={c}"));
    } else if let Some(u) = r.until {
        // Floating like DTSTART (RFC 5545 3.3.10).
        s.push_str(&format!(";UNTIL={}T235959", u.format("%Y%m%d")));
    }
    s
}

fn grid_row(grid: &gtk::Grid, row: i32, title: &str, widget: &impl IsA<gtk::Widget>) {
    let l = label(title);
    l.add_css_class("field-label");
    l.set_valign(gtk::Align::Center);
    grid.attach(&l, 0, row, 1, 1);
    grid.attach(widget, 1, row, 1, 1);
}

fn calendar_dropdown(app: &App, selected: &str) -> (gtk::DropDown, Vec<String>) {
    let store = app.store.borrow();
    let cals = store.live_calendars();
    let names: Vec<&str> = cals.iter().map(|c| c.name.as_str()).collect();
    let uids: Vec<String> = cals.iter().map(|c| c.uid.clone()).collect();
    let dd = gtk::DropDown::from_strings(&names);
    let sel = uids.iter().position(|u| u == selected).or_else(|| {
        let fallback = store
            .calendars
            .get(selected)
            .filter(|c| !c.is_deleted())
            .map(|c| c.uid.clone())
            .unwrap_or_else(|| app.default_calendar());
        uids.iter().position(|u| *u == fallback)
    });
    dd.set_selected(sel.unwrap_or(0) as u32);
    (dd, uids)
}

fn text_view(text: &str) -> (gtk::ScrolledWindow, gtk::TextView) {
    let tv = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(6)
        .right_margin(6)
        .build();
    tv.buffer().set_text(text);
    let sw = gtk::ScrolledWindow::builder()
        .child(&tv)
        .min_content_height(90)
        .vexpand(true)
        .build();
    sw.add_css_class("frame");
    (sw, tv)
}

fn buffer_text(tv: &gtk::TextView) -> String {
    let b = tv.buffer();
    b.text(&b.start_iter(), &b.end_iter(), false).to_string()
}

fn register(app: &Rc<App>, uid: &str, window: &gtk::Window) {
    app.open_editors
        .borrow_mut()
        .insert(uid.to_string(), window.clone());
    let a = app.weak();
    let uid = uid.to_string();
    window.connect_destroy(move |_| {
        if let Some(app) = a.upgrade() {
            app.open_editors.borrow_mut().remove(&uid);
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------------------------

/// Edit `existing`, or create from `template`. `occurrence` is the clicked occurrence of a
/// recurring event (offers deleting just that one).
pub fn open_event(
    app: &Rc<App>,
    existing: Option<Item>,
    template: Option<Item>,
    occurrence: Option<NaiveDateTime>,
) {
    let is_new = existing.is_none();
    let Some(item) = existing.or(template) else {
        return;
    };
    let window = dialog_window(&app.window, if is_new { "New Event" } else { "Edit Event" });
    window.set_default_size(560, -1);
    if !is_new {
        register(app, &item.uid, &window);
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("dialog-content");
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .build();

    let title = gtk::Entry::builder()
        .text(&item.summary)
        .placeholder_text("Event name")
        .hexpand(true)
        .activates_default(true)
        .build();
    let location = gtk::Entry::builder()
        .text(&item.location)
        .placeholder_text("Optional")
        .build();
    let (calendar, cal_uids) = calendar_dropdown(app, &item.calendar);
    let all_day = gtk::CheckButton::builder()
        .label("All day")
        .active(item.is_all_day())
        .build();

    let start = item.start.unwrap_or(When::Date(today()));
    let end_dt = item
        .effective_end()
        .unwrap_or(start.naive() + Duration::hours(1));
    // All-day ends are exclusive in the file and inclusive on screen.
    let end_shown = if start.is_date() {
        (end_dt - Duration::days(1)).date().max(start.date())
    } else {
        end_dt.date()
    };
    let start_date = DatePicker::new(start.date());
    let start_time = TimePicker::new(if start.is_date() {
        NaiveTime::from_hms_opt(9, 0, 0).unwrap()
    } else {
        start.naive().time()
    });
    let end_date = DatePicker::new(end_shown);
    let end_time = TimePicker::new(if start.is_date() {
        NaiveTime::from_hms_opt(10, 0, 0).unwrap()
    } else {
        end_dt.time()
    });
    let start_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    start_box.append(&start_date.widget);
    start_box.append(&start_time.widget);
    let end_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    end_box.append(&end_date.widget);
    end_box.append(&end_time.widget);

    // Keep the length when the start moves.
    {
        let length = Rc::new(RefCell::new(end_dt - start.naive()));
        let (sd, st, ed, et, ad) = (
            start_date.clone(),
            start_time.clone(),
            end_date.clone(),
            end_time.clone(),
            all_day.clone(),
        );
        let len = length.clone();
        let update_end = Rc::new(move || {
            let s = sd.get().and_time(st.get());
            let e = s + *len.borrow();
            ed.set(if ad.is_active() {
                e.date().max(sd.get())
            } else {
                e.date()
            });
            et.set(e.time());
        });
        let u = update_end.clone();
        start_date.connect_changed(move |_| u());
        let u = update_end.clone();
        start_time.connect_changed(move || u());
        let (sd, st, ed, et) = (
            start_date.clone(),
            start_time.clone(),
            end_date.clone(),
            end_time.clone(),
        );
        let remember = Rc::new(move || {
            let l = ed.get().and_time(et.get()) - sd.get().and_time(st.get());
            if l >= Duration::zero() {
                *length.borrow_mut() = l;
            }
        });
        let r = remember.clone();
        end_date.connect_changed(move |_| r());
        end_time.connect_changed(move || remember());
    }
    {
        let (st, et) = (start_time.widget.clone(), end_time.widget.clone());
        let sync = move |c: &gtk::CheckButton| {
            st.set_visible(!c.is_active());
            et.set_visible(!c.is_active());
        };
        sync(&all_day);
        all_day.connect_toggled(sync);
    }

    // Repeat
    let simple = item.rrule.as_deref().map(parse_simple);
    let freq_index = match &simple {
        None => 0,
        Some(Some(r)) => r.freq,
        Some(None) => 5,
    };
    let rule = simple.flatten().unwrap_or(SimpleRule {
        interval: 1,
        ..Default::default()
    });
    let freq = gtk::DropDown::from_strings(&FREQS);
    freq.set_selected(freq_index as u32);
    let interval = gtk::SpinButton::with_range(1.0, 999.0, 1.0);
    interval.set_value(rule.interval as f64);
    let unit = label(UNITS.get(freq_index).copied().unwrap_or(""));
    let every_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    every_box.append(&label("Every"));
    every_box.append(&interval);
    every_box.append(&unit);
    let days_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    days_box.add_css_class("linked");
    let mut day_buttons = Vec::new();
    let start_weekday = WEEKDAYS
        .iter()
        .find(|w| w.0 == start.date().weekday())
        .map(|w| w.1)
        .unwrap_or("MO");
    for (_, code, short) in WEEKDAYS {
        let active = if rule.days.is_empty() {
            code == start_weekday
        } else {
            rule.days.iter().any(|d| d == code)
        };
        let b = gtk::ToggleButton::builder()
            .label(short)
            .active(active)
            .build();
        days_box.append(&b);
        day_buttons.push((code, b));
    }
    let ends = gtk::DropDown::from_strings(&["Never", "After", "On"]);
    ends.set_selected(if rule.count.is_some() {
        1
    } else if rule.until.is_some() {
        2
    } else {
        0
    });
    let count = gtk::SpinButton::with_range(1.0, 9999.0, 1.0);
    count.set_value(rule.count.unwrap_or(10) as f64);
    let count_label = label("times");
    let until = DatePicker::new(rule.until.unwrap_or(start.date() + Duration::days(90)));
    let ends_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    ends_box.append(&ends);
    ends_box.append(&count);
    ends_box.append(&count_label);
    ends_box.append(&until.widget);
    let custom = gtk::Entry::builder()
        .text(item.rrule.as_deref().unwrap_or(""))
        .placeholder_text("RRULE, e.g. FREQ=MONTHLY;BYDAY=-1FR")
        .build();
    custom.add_css_class("monospace");
    let repeat_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    repeat_box.append(&freq);
    repeat_box.append(&every_box);
    repeat_box.append(&days_box);
    repeat_box.append(&ends_box);
    repeat_box.append(&custom);
    {
        let (every_box, days_box, ends_box, custom, unit) = (
            every_box.clone(),
            days_box.clone(),
            ends_box.clone(),
            custom.clone(),
            unit.clone(),
        );
        let (ends_c, count, count_label, until_w) = (
            ends.clone(),
            count.clone(),
            count_label.clone(),
            until.widget.clone(),
        );
        let update = Rc::new(move |f: u32| {
            let simple = (1..=4).contains(&f);
            every_box.set_visible(simple);
            days_box.set_visible(f == 2);
            ends_box.set_visible(simple);
            custom.set_visible(f == 5);
            unit.set_label(UNITS.get(f as usize).copied().unwrap_or(""));
            let e = ends_c.selected();
            count.set_visible(e == 1);
            count_label.set_visible(e == 1);
            until_w.set_visible(e == 2);
        });
        update(freq.selected());
        let u = update.clone();
        freq.connect_selected_notify(move |d| u(d.selected()));
        let f = freq.clone();
        ends.connect_selected_notify(move |_| update(f.selected()));
    }

    let (notes_scroll, notes) = text_view(&item.description);
    let error = label("");
    error.add_css_class("error-text");
    error.set_visible(false);

    let mut row = 0;
    for (t, w) in [
        ("Title", title.clone().upcast::<gtk::Widget>()),
        ("Location", location.clone().upcast()),
        ("Calendar", calendar.clone().upcast()),
        ("", all_day.clone().upcast()),
        ("Starts", start_box.upcast()),
        ("Ends", end_box.upcast()),
        ("Repeat", repeat_box.upcast()),
        ("Notes", notes_scroll.upcast()),
    ] {
        grid_row(&grid, row, t, &w);
        row += 1;
    }
    content.append(&grid);
    content.append(&error);

    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label(if is_new { "Add" } else { "Save" });
    save.add_css_class("suggested-action");
    let delete = gtk::Button::with_label("Delete");
    delete.add_css_class("destructive-action");
    let dup = gtk::Button::with_label("Duplicate");
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if !is_new {
        left.append(&delete);
        left.append(&dup);
    }
    content.append(&button_row(Some(left.upcast_ref()), &[&cancel, &save]));
    window.set_child(Some(&widgets::titled(&window, &content)));
    window.set_default_widget(Some(&save));

    {
        let w = window.clone();
        cancel.connect_clicked(move |_| w.close());
    }

    // Build the edited item from the form.
    let form = {
        let item = item.clone();
        let (title, location, notes, all_day, freq, interval, ends, count, custom) = (
            title.clone(),
            location.clone(),
            notes.clone(),
            all_day.clone(),
            freq.clone(),
            interval.clone(),
            ends.clone(),
            count.clone(),
            custom.clone(),
        );
        let (start_date, start_time, end_date, end_time, until) = (
            start_date.clone(),
            start_time.clone(),
            end_date.clone(),
            end_time.clone(),
            until.clone(),
        );
        let calendar = calendar.clone();
        let day_buttons = day_buttons.clone();
        move || -> Result<Item, String> {
            let mut it = item.clone();
            it.summary = title.text().trim().to_string();
            if it.summary.is_empty() {
                return Err("The event needs a title.".into());
            }
            it.location = location.text().trim().to_string();
            it.description = buffer_text(&notes);
            it.calendar = cal_uids
                .get(calendar.selected() as usize)
                .cloned()
                .unwrap_or_default();
            if all_day.is_active() {
                let (s, e) = (start_date.get(), end_date.get());
                if e < s {
                    return Err("The event ends before it starts.".into());
                }
                it.start = Some(When::Date(s));
                it.end = Some(When::Date(e + Duration::days(1)));
            } else {
                let s = start_date.get().and_time(start_time.get());
                let e = end_date.get().and_time(end_time.get());
                if e < s {
                    return Err("The event ends before it starts.".into());
                }
                it.start = Some(When::DateTime(s));
                it.end = Some(When::DateTime(e));
            }
            let f = freq.selected() as usize;
            it.rrule = match f {
                0 => None,
                5 => {
                    let r = custom
                        .text()
                        .trim()
                        .trim_start_matches("RRULE:")
                        .to_string();
                    if r.is_empty() {
                        None
                    } else {
                        let test = opencal::recur::expand(
                            &Item {
                                rrule: Some(r.clone()),
                                ..it.clone()
                            },
                            it.start.unwrap().naive(),
                            it.start.unwrap().naive() + Duration::days(1),
                            1,
                        );
                        if test.is_none() {
                            return Err("The repeat rule is not valid.".into());
                        }
                        Some(r)
                    }
                }
                _ => {
                    let rule = SimpleRule {
                        freq: f,
                        interval: interval.value() as u32,
                        days: if f == 2 {
                            day_buttons
                                .iter()
                                .filter(|(_, b)| b.is_active())
                                .map(|(c, _)| c.to_string())
                                .collect()
                        } else {
                            Vec::new()
                        },
                        count: (ends.selected() == 1).then(|| count.value() as u32),
                        until: (ends.selected() == 2).then(|| until.get()),
                    };
                    Some(build_simple(&rule))
                }
            };
            if it.rrule != item.rrule || it.start != item.start {
                // Old exceptions don't apply to a different rule.
                it.exdates.clear();
            }
            Ok(it)
        }
    };
    let form = Rc::new(form);

    {
        let (w, app, form, error) = (window.clone(), app.clone(), form.clone(), error.clone());
        let original = item.clone();
        save.connect_clicked(move |_| match form() {
            Ok(mut it) => {
                if !is_new {
                    if it == original {
                        w.close();
                        return;
                    }
                    it.touch();
                }
                app.change(|s| s.add_item(it.clone()));
                app.set_status(&format!(
                    "{} \"{}\"",
                    if is_new { "Added" } else { "Saved" },
                    it.summary
                ));
                w.close();
            }
            Err(e) => {
                error.set_label(&e);
                error.set_visible(true);
            }
        });
    }
    {
        let (w, app, form, error) = (window.clone(), app.clone(), form.clone(), error.clone());
        dup.connect_clicked(move |_| match form() {
            Ok(it) => {
                let mut copy = Item::new_event(
                    &it.calendar,
                    &format!("{} (copy)", it.summary),
                    it.start.unwrap(),
                    it.end.unwrap(),
                );
                copy.location = it.location.clone();
                copy.description = it.description.clone();
                copy.rrule = it.rrule.clone();
                w.close();
                open_event(&app, None, Some(copy), None);
            }
            Err(e) => {
                error.set_label(&e);
                error.set_visible(true);
            }
        });
    }
    {
        let (w, app) = (window.clone(), app.clone());
        let uid = item.uid.clone();
        let summary = item.summary.clone();
        let recurring = item.rrule.is_some();
        let all = item.is_all_day();
        delete.connect_clicked(move |_| {
            let (app2, w2, uid2) = (app.clone(), w.clone(), uid.clone());
            let delete_all = move || {
                app2.change(|s| {
                    s.delete_item(&uid2);
                });
                app2.set_status("Event deleted");
                w2.close();
            };
            match (recurring, occurrence) {
                (true, Some(occ)) => {
                    let dialog = gtk::AlertDialog::builder()
                        .message(format!("Delete \"{summary}\"?"))
                        .detail("This event repeats.")
                        .buttons(["Only This Occurrence", "All Occurrences", "Cancel"])
                        .cancel_button(2)
                        .default_button(2)
                        .modal(true)
                        .build();
                    let (app3, w3, uid3) = (app.clone(), w.clone(), uid.clone());
                    dialog.choose(
                        Some(&w),
                        gtk::gio::Cancellable::NONE,
                        move |res| match res {
                            Ok(0) => {
                                app3.change(|s| {
                                    if let Some(i) = s.items.get_mut(&uid3) {
                                        i.exdates.push(if all {
                                            When::Date(occ.date())
                                        } else {
                                            When::DateTime(occ)
                                        });
                                        i.touch();
                                    }
                                });
                                app3.set_status("Occurrence deleted");
                                w3.close();
                            }
                            Ok(1) => delete_all(),
                            _ => {}
                        },
                    );
                }
                _ => widgets::confirm(
                    &w,
                    &format!("Delete \"{summary}\"?"),
                    "",
                    "Delete",
                    delete_all,
                ),
            }
        });
    }
    window.present();
    title.grab_focus();
}

// ---------------------------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------------------------

pub fn open_task(app: &Rc<App>, existing: Option<Item>, template: Option<Item>) {
    let is_new = existing.is_none();
    let item = existing
        .or(template)
        .unwrap_or_else(|| Item::new_task(&app.default_calendar(), "", None));
    let window = dialog_window(&app.window, if is_new { "New Task" } else { "Edit Task" });
    window.set_default_size(500, -1);
    if !is_new {
        register(app, &item.uid, &window);
    }
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("dialog-content");
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .build();

    let title = gtk::Entry::builder()
        .text(&item.summary)
        .placeholder_text("Task")
        .hexpand(true)
        .activates_default(true)
        .build();
    let (calendar, cal_uids) = calendar_dropdown(app, &item.calendar);
    let has_due = gtk::CheckButton::builder()
        .label("Due")
        .active(item.due.is_some())
        .build();
    let due_default = item.due.unwrap_or(When::Date(app.date.get()));
    let due_date = DatePicker::new(due_default.date());
    let has_time = gtk::CheckButton::builder()
        .label("at")
        .active(!due_default.is_date())
        .build();
    let due_time = TimePicker::new(if due_default.is_date() {
        NaiveTime::from_hms_opt(17, 0, 0).unwrap()
    } else {
        due_default.naive().time()
    });
    let due_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    due_box.append(&has_due);
    due_box.append(&due_date.widget);
    due_box.append(&has_time);
    due_box.append(&due_time.widget);
    {
        let (dd, ht, dt) = (
            due_date.widget.clone(),
            has_time.clone(),
            due_time.widget.clone(),
        );
        let hd = has_due.clone();
        let update = Rc::new(move || {
            dd.set_sensitive(hd.is_active());
            ht.set_sensitive(hd.is_active());
            dt.set_sensitive(hd.is_active() && ht.is_active());
        });
        update();
        let u = update.clone();
        has_due.connect_toggled(move |_| u());
        has_time.connect_toggled(move |_| update());
    }
    let priority = gtk::DropDown::from_strings(&["None", "High", "Medium", "Low"]);
    priority.set_selected(match item.priority {
        0 => 0,
        1..=4 => 1,
        5 => 2,
        _ => 3,
    });
    let done = gtk::CheckButton::builder()
        .label("Completed")
        .active(item.is_completed())
        .build();
    let (notes_scroll, notes) = text_view(&item.description);
    let error = label("");
    error.add_css_class("error-text");
    error.set_visible(false);

    let mut row = 0;
    for (t, w) in [
        ("Title", title.clone().upcast::<gtk::Widget>()),
        ("Calendar", calendar.clone().upcast()),
        ("", due_box.upcast()),
        ("Priority", priority.clone().upcast()),
        ("", done.clone().upcast()),
        ("Notes", notes_scroll.upcast()),
    ] {
        grid_row(&grid, row, t, &w);
        row += 1;
    }
    content.append(&grid);
    content.append(&error);
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label(if is_new { "Add" } else { "Save" });
    save.add_css_class("suggested-action");
    let delete = gtk::Button::with_label("Delete");
    delete.add_css_class("destructive-action");
    content.append(&button_row(
        if is_new {
            None
        } else {
            Some(delete.upcast_ref())
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
        let original = item.clone();
        save.connect_clicked(move |_| {
            let mut it = original.clone();
            it.summary = title.text().trim().to_string();
            if it.summary.is_empty() {
                error.set_label("The task needs a title.");
                error.set_visible(true);
                return;
            }
            it.calendar = cal_uids
                .get(calendar.selected() as usize)
                .cloned()
                .unwrap_or_default();
            it.due = has_due.is_active().then(|| {
                if has_time.is_active() {
                    When::DateTime(due_date.get().and_time(due_time.get()))
                } else {
                    When::Date(due_date.get())
                }
            });
            it.priority = [0, 1, 5, 9][priority.selected().min(3) as usize];
            it.description = buffer_text(&notes);
            let completion_changed = done.is_active() != original.is_completed();
            let mut content_check = it.clone();
            content_check.completed = original.completed;
            if !is_new && content_check != original {
                it.touch();
            }
            if completion_changed {
                it.set_completed(done.is_active());
            }
            if !is_new && it == original {
                w.close();
                return;
            }
            app.change(|s| s.add_item(it.clone()));
            app.set_status(&format!(
                "{} task \"{}\"",
                if is_new { "Added" } else { "Saved" },
                it.summary
            ));
            w.close();
        });
    }
    {
        let (w, app) = (window.clone(), app.clone());
        let uid = item.uid.clone();
        let summary = item.summary.clone();
        delete.connect_clicked(move |_| {
            let (app, w2, uid) = (app.clone(), w.clone(), uid.clone());
            widgets::confirm(
                &w,
                &format!("Delete \"{summary}\"?"),
                "",
                "Delete",
                move || {
                    app.change(|s| {
                        s.delete_item(&uid);
                    });
                    app.set_status("Task deleted");
                    w2.close();
                },
            );
        });
    }
    window.present();
}
