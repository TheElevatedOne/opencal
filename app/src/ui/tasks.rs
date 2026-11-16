//! Tasks view: all to-dos with a quick-add field.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use opencal::{Item, Kind};

use super::{App, today, widgets};

pub struct TasksView {
    pub widget: gtk::Box,
    list: gtk::ListBox,
    show_done: gtk::CheckButton,
    rows: Rc<RefCell<Vec<String>>>,
}

impl TasksView {
    pub fn new(app: &Rc<App>) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 12);
        widget.add_css_class("page");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let entry = gtk::Entry::builder()
            .placeholder_text("Add a task and press Enter")
            .hexpand(true)
            .build();
        let show_done = gtk::CheckButton::builder()
            .label("Show completed")
            .active(app.config.borrow().view.show_completed)
            .build();
        top.append(&entry);
        top.append(&show_done);
        widget.append(&top);

        let list = gtk::ListBox::new();
        list.add_css_class("panel");
        list.set_selection_mode(gtk::SelectionMode::None);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        widget.append(&scroll);

        let a: Weak<App> = app.weak();
        entry.connect_activate(move |e| {
            let text = e.text().trim().to_string();
            if text.is_empty() {
                return;
            }
            if let Some(app) = a.upgrade() {
                let cal = app.default_calendar();
                app.change(|s| s.add_item(Item::new_task(&cal, &text, None)));
                app.set_status(&format!("Added task \"{text}\""));
            }
            e.set_text("");
        });
        let a: Weak<App> = app.weak();
        show_done.connect_toggled(move |c| {
            if let Some(app) = a.upgrade() {
                app.config.borrow_mut().view.show_completed = c.is_active();
                let _ = app.config.borrow().save(&app.paths);
                app.queue_refresh();
            }
        });
        let rows: Rc<RefCell<Vec<String>>> = Rc::default();
        let r = rows.clone();
        let a: Weak<App> = app.weak();
        list.connect_row_activated(move |_, row| {
            let uid = r.borrow().get(row.index() as usize).cloned();
            if let (Some(uid), Some(app)) = (uid, a.upgrade()) {
                app.open_item(&uid, None);
            }
        });
        let menu_click = gtk::GestureClick::builder().button(3).build();
        let (r, a, l) = (rows.clone(), app.weak(), list.clone());
        menu_click.connect_pressed(move |_, _, x, y| {
            let Some(row) = l.row_at_y(y as i32) else {
                return;
            };
            let uid = r
                .borrow()
                .get(row.index() as usize)
                .cloned()
                .filter(|u| !u.is_empty());
            if let (Some(uid), Some(app)) = (uid, a.upgrade()) {
                super::popup_menu(&l, &app.item_menu(&uid, None), x, y);
            }
        });
        list.add_controller(menu_click);
        TasksView {
            widget,
            list,
            show_done,
            rows,
        }
    }

    pub fn refresh(&self, app: &App) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let show_done = self.show_done.is_active();
        let hidden = app.hidden_calendars();
        let store = app.store.borrow();
        let mut tasks: Vec<&Item> = store
            .live_items()
            .filter(|i| i.kind == Kind::Task)
            .filter(|i| show_done || !i.is_completed())
            .filter(|i| {
                store
                    .calendar_of(i)
                    .is_none_or(|c| !hidden.contains(&c.uid))
            })
            .collect();
        tasks.sort_by(|a, b| {
            let prio = |i: &Item| if i.priority == 0 { 10 } else { i.priority };
            a.is_completed()
                .cmp(&b.is_completed())
                .then(a.due.is_none().cmp(&b.due.is_none()))
                .then(a.due.map(|d| d.naive()).cmp(&b.due.map(|d| d.naive())))
                .then(prio(a).cmp(&prio(b)))
                .then(a.summary.to_lowercase().cmp(&b.summary.to_lowercase()))
        });
        let mut rows = Vec::new();
        let t = today();
        for task in &tasks {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let cal = store
                .calendar_of(task)
                .map(|c| c.uid.clone())
                .unwrap_or_default();
            let check = gtk::CheckButton::builder()
                .active(task.is_completed())
                .tooltip_text("Done")
                .build();
            check.add_css_class("colored-check");
            check.add_css_class(&app.color_class(&cal));
            let uid = task.uid.clone();
            let a = app.weak();
            check.connect_toggled(move |c| {
                if let Some(app) = a.upgrade() {
                    app.set_task_done(&uid, c.is_active());
                }
            });
            line.append(&check);
            let summary = widgets::label(&task.summary);
            summary.set_hexpand(true);
            summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
            if task.is_completed() {
                summary.add_css_class("done");
            }
            line.append(&summary);
            if task.priority > 0 {
                let p = match task.priority {
                    1..=4 => "High",
                    5 => "Medium",
                    _ => "Low",
                };
                let l = gtk::Label::new(Some(p));
                l.add_css_class("badge");
                l.add_css_class(&format!("priority-{}", p.to_lowercase()));
                line.append(&l);
            }
            if let Some(due) = task.due {
                let text = match due {
                    opencal::When::Date(d) if d == t => "Today".to_string(),
                    opencal::When::Date(d) => d.format("%a %-d %b %Y").to_string(),
                    opencal::When::DateTime(dt) if dt.date() == t => {
                        format!("Today {}", dt.format("%H:%M"))
                    }
                    opencal::When::DateTime(dt) => dt.format("%a %-d %b %Y %H:%M").to_string(),
                };
                let l = widgets::dim_label(&text);
                if !task.is_completed() && due.date() < t {
                    l.remove_css_class("dim-label");
                    l.add_css_class("error-text");
                }
                line.append(&l);
            }
            line.append(&widgets::swatch(&app.color_class(&cal)));
            self.list
                .append(&gtk::ListBoxRow::builder().child(&line).build());
            rows.push(task.uid.clone());
        }
        if tasks.is_empty() {
            let empty = gtk::ListBoxRow::builder()
                .activatable(false)
                .child(&widgets::dim_label("No tasks."))
                .build();
            self.list.append(&empty);
            rows.push(String::new());
        }
        *self.rows.borrow_mut() = rows;
    }
}
