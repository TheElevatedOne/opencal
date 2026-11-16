//! Desktop notifications before events and when tasks are due. They work while opencal runs,
//! including in the background (tray).

use std::collections::HashSet;

use chrono::{Duration, Local, NaiveDateTime, NaiveTime};
use gtk::gio;
use gtk::prelude::*;
use opencal::Kind;

use super::App;

/// Check for reminders due since the previous check. Called every 30 seconds.
pub fn tick(app: &App, last: &mut Option<NaiveDateTime>, sent: &mut HashSet<String>) {
    let now = Local::now().naive_local();
    // The first check only remembers the time: nothing from before the start is announced.
    let Some(prev) = last.replace(now) else {
        return;
    };
    // A long suspend: don't fire a pile of stale reminders.
    let prev = prev.max(now - Duration::minutes(15));
    let cfg = app.config.borrow().clone();
    if !cfg.reminders.events && !cfg.reminders.tasks {
        return;
    }
    let lead = Duration::minutes(cfg.reminders.minutes_before as i64);
    let day_start = NaiveTime::from_hms_opt(cfg.view.day_start_hour.min(23), 0, 0).unwrap();
    let from = (prev - Duration::days(1)).date();
    let to = (now + lead + Duration::days(2)).date();
    let occ = opencal::recur::occurrences(&app.store.borrow(), from, to, &app.hidden_calendars());
    let Some(application) = app.window.application() else {
        return;
    };
    for o in occ {
        let (fire, body) = match o.kind {
            Kind::Event if !cfg.reminders.events => continue,
            Kind::Task if !cfg.reminders.tasks || o.completed => continue,
            Kind::Event if o.all_day => (
                o.start.date().and_time(day_start),
                "Today, all day".to_string(),
            ),
            Kind::Event => {
                let when = if o.start.date() == now.date() {
                    format!("{} – {}", o.start.format("%H:%M"), o.end.format("%H:%M"))
                } else {
                    o.start.format("%a %-d %b, %H:%M").to_string()
                };
                (o.start - lead, when)
            }
            Kind::Task if o.all_day => {
                (o.start.date().and_time(day_start), "Due today".to_string())
            }
            Kind::Task => (o.start, format!("Due at {}", o.start.format("%H:%M"))),
        };
        if fire <= prev || fire > now {
            continue;
        }
        let key = format!("{}@{}", o.uid, o.start.format("%Y%m%dT%H%M"));
        if !sent.insert(key.clone()) {
            continue;
        }
        let location = app
            .store
            .borrow()
            .items
            .get(&o.uid)
            .map(|i| i.location.clone())
            .unwrap_or_default();
        let body = if location.is_empty() {
            body
        } else {
            format!("{body} · {location}")
        };
        let n = gio::Notification::new(&o.summary);
        n.set_body(Some(&body));
        n.set_icon(&gio::ThemedIcon::new(crate::APP_ID));
        n.set_default_action_and_target_value("app.open-item", Some(&o.uid.to_variant()));
        if o.kind == Kind::Task {
            n.add_button_with_target_value("Mark Done", "app.task-done", Some(&o.uid.to_variant()));
        }
        application.send_notification(Some(&key), &n);
    }
    // Forget keys of reminders long past.
    if sent.len() > 500 {
        sent.clear();
    }
}

/// "Next: 14:00 Dentist" for the tray tooltip.
pub fn next_up(app: &App) -> String {
    let now = Local::now().naive_local();
    let occ = opencal::recur::occurrences(
        &app.store.borrow(),
        now.date(),
        now.date() + Duration::days(2),
        &app.hidden_calendars(),
    );
    occ.iter()
        .filter(|o| o.kind == Kind::Event && !o.all_day && o.start >= now)
        .min_by_key(|o| o.start)
        .map(|o| {
            let day = if o.start.date() == now.date() {
                String::new()
            } else {
                "tomorrow ".into()
            };
            format!("Next: {day}{} {}", o.start.format("%H:%M"), o.summary)
        })
        .unwrap_or_else(|| "Nothing else today".into())
}
