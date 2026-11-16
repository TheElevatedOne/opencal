//! Agenda view: the coming days as a list.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use chrono::{Duration, NaiveDateTime};
use gtk::prelude::*;
use opencal::Kind;

use super::{App, today, widgets};

const DAYS: i64 = 60;

pub struct AgendaView {
    pub widget: gtk::ScrolledWindow,
    list: gtk::ListBox,
    rows: Rc<RefCell<Vec<Option<(String, NaiveDateTime)>>>>,
}

impl AgendaView {
    pub fn new(app: &Rc<App>) -> Self {
        let list = gtk::ListBox::new();
        list.add_css_class("panel");
        list.add_css_class("agenda");
        list.set_selection_mode(gtk::SelectionMode::None);
        let rows: Rc<RefCell<Vec<Option<(String, NaiveDateTime)>>>> = Rc::default();
        let r = rows.clone();
        let a: Weak<App> = app.weak();
        list.connect_row_activated(move |_, row| {
            let entry = r.borrow().get(row.index() as usize).cloned().flatten();
            if let (Some((uid, start)), Some(app)) = (entry, a.upgrade()) {
                app.open_item(&uid, Some(start));
            }
        });
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.add_css_class("page");
        page.append(&list);
        let widget = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&page)
            .vexpand(true)
            .build();
        AgendaView { widget, list, rows }
    }

    pub fn refresh(&self, app: &App) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let mut rows = Vec::new();
        let from = app.date.get();
        let occ = opencal::recur::occurrences(
            &app.store.borrow(),
            from,
            from + Duration::days(DAYS),
            &app.hidden_calendars(),
        );
        let t = today();
        for i in 0..DAYS {
            let day = from + Duration::days(i);
            let todays: Vec<_> = occ.iter().filter(|o| o.on_day(day)).collect();
            if todays.is_empty() {
                continue;
            }
            let title = if day == t {
                format!("Today · {}", day.format("%A %-d %B"))
            } else {
                day.format("%A %-d %B %Y").to_string()
            };
            let head = gtk::ListBoxRow::builder()
                .activatable(false)
                .child(&widgets::label(&title))
                .build();
            head.add_css_class("agenda-day");
            self.list.append(&head);
            rows.push(None);
            for o in todays {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                line.append(&widgets::swatch(&app.color_class(&o.calendar)));
                let when = match o.kind {
                    Kind::Task if o.all_day => "Due".to_string(),
                    Kind::Task => format!("Due {}", o.start.format("%H:%M")),
                    Kind::Event if o.all_day || o.end - o.start >= Duration::hours(24) => {
                        "All day".to_string()
                    }
                    Kind::Event => {
                        format!("{} – {}", o.start.format("%H:%M"), o.end.format("%H:%M"))
                    }
                };
                let time = widgets::dim_label(&when);
                time.set_width_chars(13);
                line.append(&time);
                let summary = if o.kind == Kind::Task {
                    format!("{} {}", if o.completed { "☑" } else { "☐" }, o.summary)
                } else {
                    o.summary.clone()
                };
                let s = widgets::label(&summary);
                s.set_ellipsize(gtk::pango::EllipsizeMode::End);
                if o.completed {
                    s.add_css_class("done");
                }
                line.append(&s);
                let loc = app
                    .store
                    .borrow()
                    .items
                    .get(&o.uid)
                    .map(|i| i.location.clone())
                    .unwrap_or_default();
                if !loc.is_empty() {
                    let l = widgets::dim_label(&loc);
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    line.append(&l);
                }
                if o.recurring {
                    line.append(&gtk::Image::from_icon_name(
                        "media-playlist-repeat-symbolic",
                    ));
                }
                let row = gtk::ListBoxRow::builder().child(&line).build();
                self.list.append(&row);
                rows.push(Some((o.uid.clone(), o.start)));
            }
        }
        if rows.is_empty() {
            let empty = gtk::ListBoxRow::builder()
                .activatable(false)
                .child(&widgets::dim_label(&format!(
                    "Nothing in the {DAYS} days from {}.",
                    from.format("%-d %B %Y")
                )))
                .build();
            self.list.append(&empty);
            rows.push(None);
        }
        *self.rows.borrow_mut() = rows;
    }
}
