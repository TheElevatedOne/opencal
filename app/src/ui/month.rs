//! Month view: six weeks of day cells with the events of each day.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use chrono::{Datelike, Duration, NaiveDate};
use gtk::prelude::*;
use opencal::{Kind, When};

use super::{App, today};

const MAX_PER_DAY: usize = 4;

pub struct MonthView {
    pub widget: gtk::Box,
    header: gtk::Grid,
    grid: gtk::Grid,
    cells: Rc<RefCell<Vec<(NaiveDate, gtk::Box)>>>,
    app: Weak<App>,
}

impl MonthView {
    pub fn new(app: &Rc<App>) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("month-view");
        let header = gtk::Grid::builder().column_homogeneous(true).build();
        header.add_css_class("month-header");
        let grid = gtk::Grid::builder()
            .column_homogeneous(true)
            .row_homogeneous(true)
            .vexpand(true)
            .hexpand(true)
            .build();
        grid.add_css_class("month-grid");
        widget.append(&header);
        widget.append(&grid);
        MonthView {
            widget,
            header,
            grid,
            cells: Rc::default(),
            app: app.weak(),
        }
    }

    pub fn refresh(&self, app: &App) {
        while let Some(c) = self.header.first_child() {
            self.header.remove(&c);
        }
        while let Some(c) = self.grid.first_child() {
            self.grid.remove(&c);
        }
        let selected = app.date.get();
        let first = selected.with_day(1).unwrap_or(selected);
        let start = app.week_start(first);
        for i in 0..7 {
            let l = gtk::Label::new(Some(&(start + Duration::days(i)).format("%a").to_string()));
            l.add_css_class("weekday");
            self.header.attach(&l, i as i32, 0, 1, 1);
        }
        let end = start + Duration::days(42);
        let occ =
            opencal::recur::occurrences(&app.store.borrow(), start, end, &app.hidden_calendars());
        let t = today();
        let mut cells = Vec::new();
        for i in 0..42 {
            let day = start + Duration::days(i);
            let cell = self.day_cell(
                app,
                day,
                day.month() == first.month(),
                day == t,
                day == selected,
            );
            let todays: Vec<_> = occ.iter().filter(|o| o.on_day(day)).collect();
            for o in todays.iter().take(if todays.len() > MAX_PER_DAY {
                MAX_PER_DAY - 1
            } else {
                MAX_PER_DAY
            }) {
                cell.append(&self.pill(app, o, day));
            }
            if todays.len() > MAX_PER_DAY {
                let more = gtk::Button::builder()
                    .label(format!("{} more", todays.len() - (MAX_PER_DAY - 1)))
                    .css_classes(["flat", "more"])
                    .build();
                let a = self.app.clone();
                more.connect_clicked(move |_| {
                    if let Some(app) = a.upgrade() {
                        app.show_day(day);
                    }
                });
                cell.append(&more);
            }
            self.grid
                .attach(&cell, (i % 7) as i32, (i / 7) as i32, 1, 1);
            cells.push((day, cell));
        }
        *self.cells.borrow_mut() = cells;
    }

    fn day_cell(
        &self,
        _app: &App,
        day: NaiveDate,
        in_month: bool,
        is_today: bool,
        selected: bool,
    ) -> gtk::Box {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 1);
        cell.add_css_class("day-cell");
        cell.set_overflow(gtk::Overflow::Hidden);
        if !in_month {
            cell.add_css_class("other-month");
        }
        if is_today {
            cell.add_css_class("today");
        }
        if selected {
            cell.add_css_class("selected");
        }
        let number = gtk::Button::builder()
            .label(day.day().to_string())
            .halign(gtk::Align::End)
            .css_classes(["flat", "day-number"])
            .tooltip_text("Open this day")
            .build();
        let a = self.app.clone();
        number.connect_clicked(move |_| {
            if let Some(app) = a.upgrade() {
                app.show_day(day);
            }
        });
        cell.append(&number);
        let click = gtk::GestureClick::new();
        let a = self.app.clone();
        let cells = self.cells.clone();
        click.connect_pressed(move |_, n, _, _| {
            let Some(app) = a.upgrade() else { return };
            if n == 2 {
                app.new_event_at(Some((When::Date(day), When::Date(day + Duration::days(1)))));
            } else if app.date.get() != day {
                if app.date.get().month() == day.month() {
                    // Only move the selection: rebuilding the grid here would lose the double click.
                    app.date.set(day);
                    for (d, c) in cells.borrow().iter() {
                        if *d == day {
                            c.add_css_class("selected");
                        } else {
                            c.remove_css_class("selected");
                        }
                    }
                    app.sync_mini();
                } else {
                    app.go_to(day);
                }
            }
        });
        cell.add_controller(click);
        cell
    }

    fn pill(&self, app: &App, o: &opencal::Occurrence, day: NaiveDate) -> gtk::Button {
        let class = app.color_class(&o.calendar);
        let text = match o.kind {
            Kind::Task => format!("{} {}", if o.completed { "☑" } else { "☐" }, o.summary),
            Kind::Event if o.all_day || o.start.date() != day => o.summary.clone(),
            Kind::Event => format!("{} {}", o.start.format("%H:%M"), o.summary),
        };
        let label = gtk::Label::builder()
            .label(&text)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let button = gtk::Button::builder()
            .css_classes(["flat"])
            .tooltip_text(&o.summary)
            .build();
        let filled = o.kind == Kind::Event && (o.all_day || o.end - o.start >= Duration::hours(24));
        if filled {
            button.add_css_class("event-pill");
            button.add_css_class(&class);
            button.set_child(Some(&label));
        } else {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let dot = gtk::Label::new(Some("●"));
            dot.add_css_class("dot");
            row.append(&dot);
            row.append(&label);
            label.set_hexpand(true);
            button.add_css_class("event-dot");
            button.add_css_class(&class);
            button.set_child(Some(&row));
        }
        if o.completed {
            label.add_css_class("done");
        }
        let uid = o.uid.clone();
        let start = o.start;
        let a = self.app.clone();
        button.connect_clicked(move |_| {
            if let Some(app) = a.upgrade() {
                app.open_item(&uid, Some(start));
            }
        });
        button
    }
}
