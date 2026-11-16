//! Small widgets and helpers shared by the views and dialogs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use gtk::prelude::*;
use gtk::{gdk, glib};

/// A colour from the GTK theme (pywal themes define these), with a fallback.
pub fn theme_color(widget: &impl IsA<gtk::Widget>, name: &str, fallback: &gdk::RGBA) -> gdk::RGBA {
    #[allow(deprecated)]
    widget
        .style_context()
        .lookup_color(name)
        .unwrap_or(*fallback)
}

pub fn parse_hex(hex: &str) -> gdk::RGBA {
    gdk::RGBA::parse(hex).unwrap_or(gdk::RGBA::new(0.21, 0.52, 0.89, 1.0))
}

pub fn to_hex(c: &gdk::RGBA) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.red()), b(c.green()), b(c.blue()))
}

/// Black or white, whichever reads better on `bg`.
pub fn contrast(bg: &gdk::RGBA) -> gdk::RGBA {
    let l = 0.2126 * bg.red() + 0.7152 * bg.green() + 0.0722 * bg.blue();
    if l > 0.55 {
        gdk::RGBA::new(0.0, 0.0, 0.0, 1.0)
    } else {
        gdk::RGBA::new(1.0, 1.0, 1.0, 1.0)
    }
}

pub fn label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).build()
}

pub fn dim_label(text: &str) -> gtk::Label {
    let l = label(text);
    l.add_css_class("dim-label");
    l
}

/// A small coloured square for a calendar.
pub fn swatch(class: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("swatch");
    b.add_css_class(class);
    b.set_valign(gtk::Align::Center);
    b
}

pub fn format_date_long(d: NaiveDate) -> String {
    d.format("%a %-d %b %Y").to_string()
}

pub fn glib_date(d: NaiveDate) -> glib::DateTime {
    glib::DateTime::from_local(d.year(), d.month() as i32, d.day() as i32, 12, 0, 0.0)
        .unwrap_or_else(|_| glib::DateTime::now_local().unwrap())
}

pub fn from_glib_date(dt: &glib::DateTime) -> NaiveDate {
    NaiveDate::from_ymd_opt(dt.year(), dt.month() as u32, dt.day_of_month() as u32)
        .unwrap_or_default()
}

/// A button showing a date that opens a month calendar.
#[derive(Clone)]
pub struct DatePicker {
    pub widget: gtk::MenuButton,
    calendar: gtk::Calendar,
    date: Rc<Cell<NaiveDate>>,
    handlers: Rc<RefCell<Vec<Box<dyn Fn(NaiveDate)>>>>,
}

impl DatePicker {
    pub fn new(date: NaiveDate) -> Self {
        let calendar = gtk::Calendar::new();
        let popover = gtk::Popover::new();
        popover.set_child(Some(&calendar));
        let widget = gtk::MenuButton::builder().popover(&popover).build();
        let picker = DatePicker {
            widget,
            calendar,
            date: Rc::new(Cell::new(date)),
            handlers: Rc::new(RefCell::new(Vec::new())),
        };
        picker.set(date);
        // GtkCalendar emits day-selected only when a day is picked (or set from code, which
        // `set` does with the same date); the month/year arrows move its date silently.
        let p = picker.clone();
        let pop = popover.clone();
        picker.calendar.connect_day_selected(move |cal| {
            let d = from_glib_date(&cal.date());
            if d == p.date.get() {
                return;
            }
            p.date.set(d);
            p.widget.set_label(&format_date_long(d));
            for h in p.handlers.borrow().iter() {
                h(d);
            }
            pop.popdown();
        });
        // Arrows used without picking a day: open on the chosen date next time.
        let p = picker.clone();
        popover.connect_show(move |_| {
            p.calendar.select_day(&glib_date(p.date.get()));
        });
        picker
    }

    pub fn get(&self) -> NaiveDate {
        self.date.get()
    }

    pub fn set(&self, d: NaiveDate) {
        self.date.set(d);
        self.widget.set_label(&format_date_long(d));
        self.calendar.select_day(&glib_date(d));
    }

    pub fn connect_changed(&self, f: impl Fn(NaiveDate) + 'static) {
        self.handlers.borrow_mut().push(Box::new(f));
    }
}

fn spin(max: f64, step: f64) -> gtk::SpinButton {
    let s = gtk::SpinButton::with_range(0.0, max, step);
    s.set_wrap(true);
    s.set_numeric(true);
    s.set_width_chars(2);
    s.connect_output(|s| {
        s.set_text(&format!("{:02}", s.value() as i64));
        glib::Propagation::Stop
    });
    s
}

/// Hour and minute spin buttons.
#[derive(Clone)]
pub struct TimePicker {
    pub widget: gtk::Box,
    hour: gtk::SpinButton,
    minute: gtk::SpinButton,
}

impl TimePicker {
    pub fn new(t: NaiveTime) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let hour = spin(23.0, 1.0);
        let minute = spin(59.0, 5.0);
        widget.append(&hour);
        widget.append(&gtk::Label::new(Some(":")));
        widget.append(&minute);
        let p = TimePicker {
            widget,
            hour,
            minute,
        };
        p.set(t);
        p
    }

    pub fn get(&self) -> NaiveTime {
        NaiveTime::from_hms_opt(self.hour.value() as u32, self.minute.value() as u32, 0)
            .unwrap_or(NaiveTime::MIN)
    }

    pub fn set(&self, t: NaiveTime) {
        self.hour.set_value(t.hour() as f64);
        self.minute.set_value(t.minute() as f64);
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        let f = Rc::new(f);
        let g = f.clone();
        self.hour.connect_value_changed(move |_| f());
        self.minute.connect_value_changed(move |_| g());
    }
}

/// A modal window for dialogs: square, opaque, no custom titlebar.
pub fn dialog_window(parent: &impl IsA<gtk::Window>, title: &str) -> gtk::Window {
    let w = gtk::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .build();
    w.add_css_class("opencal");
    w.add_css_class("dialog-window");
    w.set_decorated(false);
    let esc = gtk::EventControllerKey::new();
    let ww = w.clone();
    esc.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            ww.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    w.add_controller(esc);
    w
}

/// The bottom row of a dialog: optional left widget, then buttons on the right.
pub fn button_row(left: Option<&gtk::Widget>, buttons: &[&gtk::Button]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("dialog-buttons");
    if let Some(l) = left {
        row.append(l);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    row.append(&spacer);
    for b in buttons {
        row.append(*b);
    }
    row
}

/// A Yes/No question. `on_yes` runs when the first button is chosen.
pub fn confirm(
    parent: &impl IsA<gtk::Window>,
    message: &str,
    detail: &str,
    yes: &str,
    on_yes: impl FnOnce() + 'static,
) {
    let dialog = gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .buttons([yes, "Cancel"])
        .default_button(1)
        .cancel_button(1)
        .modal(true)
        .build();
    dialog.choose(Some(parent), gtk::gio::Cancellable::NONE, move |res| {
        if res == Ok(0) {
            on_yes();
        }
    });
}

pub fn error(parent: &impl IsA<gtk::Window>, message: &str, detail: &str) {
    gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .modal(true)
        .build()
        .show(Some(parent));
}

/// A dialog's content with its title on top (dialogs have no title bar).
pub fn titled(window: &gtk::Window, content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let title = label(&window.title().map(|t| t.to_string()).unwrap_or_default());
    title.add_css_class("dialog-title");
    b.append(&title);
    b.append(content);
    b
}
