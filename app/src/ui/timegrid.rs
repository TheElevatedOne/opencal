//! Day and week views: an hour grid with events drawn as blocks. Drag on empty space to create
//! an event, drag an event to move it, drag its bottom edge to resize it, click to edit.
//! All-day and multi-day events and tasks are in the strip above the hours.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use gtk::prelude::*;
use gtk::{cairo, gdk, glib, pango};
use opencal::{Kind, Occurrence, When};

use super::widgets::{contrast, theme_color};
use super::{App, today};

const HOUR_H: f64 = 48.0;
const GUTTER: f64 = 56.0;
const SNAP_MIN: i64 = 15;
const DAYNAME_H: f64 = 40.0;
const LANE_H: f64 = 22.0;
const RESIZE_EDGE: f64 = 6.0;

#[derive(Clone, Debug)]
struct Hit {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    uid: String,
    start: NaiveDateTime,
    end: NaiveDateTime,
    movable: bool,
}

impl Hit {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Clone, Debug)]
enum Mode {
    Create { anchor: NaiveDateTime },
    Move { hit: Hit, anchor: NaiveDateTime },
    Resize { hit: Hit },
    Click { hit: Option<Hit> },
}

#[derive(Clone, Debug)]
struct Drag {
    mode: Mode,
    cur: NaiveDateTime,
    moved: bool,
}

struct State {
    days: i64,
    start: Cell<NaiveDate>,
    occs: RefCell<Vec<Occurrence>>,
    hits: RefCell<Vec<Hit>>,
    header_hits: RefCell<Vec<Hit>>,
    lanes: Cell<usize>,
    drag: RefCell<Option<Drag>>,
    colors: RefCell<HashMap<String, gdk::RGBA>>,
    app: Weak<App>,
    scrolled: Cell<bool>,
}

pub struct TimeGrid {
    pub widget: gtk::Box,
    header: gtk::DrawingArea,
    body: gtk::DrawingArea,
    scroller: gtk::ScrolledWindow,
    state: Rc<State>,
}

fn is_banner(o: &Occurrence) -> bool {
    o.kind == Kind::Task || o.all_day || o.end - o.start >= Duration::hours(24)
}

fn set_color(cr: &cairo::Context, c: &gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(
        c.red() as f64,
        c.green() as f64,
        c.blue() as f64,
        c.alpha() as f64 * alpha,
    );
}

fn layout(
    widget: &gtk::DrawingArea,
    cr: &cairo::Context,
    text: &str,
    width: f64,
    bold: bool,
) -> pango::Layout {
    let l = pangocairo::functions::create_layout(cr);
    let mut font = widget
        .pango_context()
        .font_description()
        .unwrap_or_default();
    if bold {
        font.set_weight(pango::Weight::Bold);
    }
    l.set_font_description(Some(&font));
    l.set_text(text);
    l.set_width((width.max(1.0) * pango::SCALE as f64) as i32);
    l.set_ellipsize(pango::EllipsizeMode::End);
    l
}

impl State {
    fn col_w(&self, width: f64) -> f64 {
        ((width - GUTTER) / self.days as f64).max(1.0)
    }

    fn day_index(&self, x: f64, width: f64) -> i64 {
        (((x - GUTTER) / self.col_w(width)).floor() as i64).clamp(0, self.days - 1)
    }

    /// The (snapped) time under a point of the hour grid.
    fn time_at(&self, x: f64, y: f64, width: f64) -> NaiveDateTime {
        let day = self.start.get() + Duration::days(self.day_index(x, width));
        let minutes = ((y / HOUR_H * 60.0) as i64).clamp(0, 24 * 60 - 1);
        let minutes = minutes / SNAP_MIN * SNAP_MIN;
        day.and_time(NaiveTime::MIN) + Duration::minutes(minutes)
    }

    fn color(&self, calendar: &str) -> gdk::RGBA {
        self.colors
            .borrow()
            .get(calendar)
            .copied()
            .unwrap_or(gdk::RGBA::new(0.21, 0.52, 0.89, 1.0))
    }
}

impl TimeGrid {
    pub fn new(app: &Rc<App>, days: i64) -> Self {
        let state = Rc::new(State {
            days,
            start: Cell::new(today()),
            occs: RefCell::new(Vec::new()),
            hits: RefCell::new(Vec::new()),
            header_hits: RefCell::new(Vec::new()),
            lanes: Cell::new(0),
            drag: RefCell::new(None),
            colors: RefCell::new(HashMap::new()),
            app: app.weak(),
            scrolled: Cell::new(false),
        });
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("time-grid");
        let header = gtk::DrawingArea::new();
        header.add_css_class("time-grid-header");
        header.set_content_height(DAYNAME_H as i32);
        let body = gtk::DrawingArea::new();
        body.set_content_height((24.0 * HOUR_H) as i32);
        body.set_hexpand(true);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&body)
            .build();
        widget.append(&header);
        widget.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        widget.append(&scroller);

        {
            let st = state.clone();
            header.set_draw_func(move |a, cr, w, h| draw_header(&st, a, cr, w as f64, h as f64));
            let st = state.clone();
            body.set_draw_func(move |a, cr, w, h| draw_body(&st, a, cr, w as f64, h as f64));
        }
        Self::setup_header_input(&state, &header);
        Self::setup_body_input(&state, &body);

        // Keep the "now" line moving.
        {
            let b = body.downgrade();
            glib::timeout_add_seconds_local(60, move || match b.upgrade() {
                Some(b) => {
                    b.queue_draw();
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            });
        }
        TimeGrid {
            widget,
            header,
            body,
            scroller,
            state,
        }
    }

    fn setup_header_input(state: &Rc<State>, header: &gtk::DrawingArea) {
        let click = gtk::GestureClick::new();
        let st = state.clone();
        let area = header.clone();
        click.connect_released(move |g, n, x, y| {
            let Some(app) = st.app.upgrade() else { return };
            let width = area.width() as f64;
            if x < GUTTER {
                return;
            }
            let day = st.start.get() + Duration::days(st.day_index(x, width));
            if y < DAYNAME_H {
                if st.days > 1 {
                    app.show_day(day);
                }
                return;
            }
            let hit = st
                .header_hits
                .borrow()
                .iter()
                .find(|h| h.contains(x, y))
                .cloned();
            g.set_state(gtk::EventSequenceState::Claimed);
            match hit {
                Some(h) => app.open_item(&h.uid, Some(h.start)),
                None if n == 2 => {
                    app.new_event_at(Some((When::Date(day), When::Date(day + Duration::days(1)))))
                }
                None => {}
            }
        });
        header.add_controller(click);
    }

    fn setup_body_input(state: &Rc<State>, body: &gtk::DrawingArea) {
        let drag = gtk::GestureDrag::new();
        {
            let st = state.clone();
            let area = body.clone();
            drag.connect_drag_begin(move |_, x, y| {
                let width = area.width() as f64;
                if x < GUTTER {
                    *st.drag.borrow_mut() = None;
                    return;
                }
                let t = st.time_at(x, y, width);
                let hit = st
                    .hits
                    .borrow()
                    .iter()
                    .rev()
                    .find(|h| h.contains(x, y))
                    .cloned();
                let mode = match hit {
                    Some(h) if h.movable && y > h.y + h.h - RESIZE_EDGE => Mode::Resize { hit: h },
                    Some(h) if h.movable => Mode::Move { hit: h, anchor: t },
                    Some(h) => Mode::Click { hit: Some(h) },
                    None => Mode::Create { anchor: t },
                };
                *st.drag.borrow_mut() = Some(Drag {
                    mode,
                    cur: t,
                    moved: false,
                });
            });
        }
        {
            let st = state.clone();
            let area = body.clone();
            drag.connect_drag_update(move |g, dx, dy| {
                let Some((x0, y0)) = g.start_point() else {
                    return;
                };
                let width = area.width() as f64;
                let mut d = st.drag.borrow_mut();
                let Some(d) = d.as_mut() else { return };
                if dx.abs() > 4.0 || dy.abs() > 4.0 {
                    d.moved = true;
                }
                d.cur = st.time_at(x0 + dx, y0 + dy, width);
                area.queue_draw();
            });
        }
        {
            let st = state.clone();
            let area = body.clone();
            drag.connect_drag_end(move |_, _, _| {
                let d = st.drag.borrow_mut().take();
                area.queue_draw();
                let (Some(d), Some(app)) = (d, st.app.upgrade()) else {
                    return;
                };
                match d.mode {
                    Mode::Click { hit: Some(h) }
                    | Mode::Move { hit: h, .. }
                    | Mode::Resize { hit: h }
                        if !d.moved =>
                    {
                        app.open_item(&h.uid, Some(h.start));
                    }
                    Mode::Click { .. } => {}
                    Mode::Create { .. } if !d.moved => {}
                    Mode::Create { anchor } => {
                        let (a, b) = if d.cur < anchor {
                            (d.cur, anchor)
                        } else {
                            (anchor, d.cur)
                        };
                        let b = b + Duration::minutes(SNAP_MIN);
                        app.new_event_at(Some((When::DateTime(a), When::DateTime(b))));
                    }
                    Mode::Move { hit, anchor } => {
                        let by = d.cur - anchor;
                        if by != Duration::zero() {
                            app.shift_event(&hit.uid, by, by);
                        }
                    }
                    Mode::Resize { hit } => {
                        let new_end = (d.cur + Duration::minutes(SNAP_MIN))
                            .max(hit.start + Duration::minutes(SNAP_MIN));
                        let by = new_end - hit.end;
                        if by != Duration::zero() {
                            app.shift_event(&hit.uid, Duration::zero(), by);
                        }
                    }
                }
            });
        }
        body.add_controller(drag);

        let click = gtk::GestureClick::new();
        let st = state.clone();
        let area = body.clone();
        click.connect_pressed(move |_, n, x, y| {
            if n != 2 || x < GUTTER {
                return;
            }
            if st.hits.borrow().iter().any(|h| h.contains(x, y)) {
                return;
            }
            if let Some(app) = st.app.upgrade() {
                let t = st.time_at(x, y, area.width() as f64);
                let t = t
                    .with_minute(if t.minute() >= 30 { 30 } else { 0 })
                    .unwrap_or(t);
                app.new_event_at(Some((
                    When::DateTime(t),
                    When::DateTime(t + Duration::hours(1)),
                )));
            }
        });
        body.add_controller(click);

        let motion = gtk::EventControllerMotion::new();
        let st = state.clone();
        let area = body.clone();
        motion.connect_motion(move |_, x, y| {
            let hits = st.hits.borrow();
            let cursor = match hits.iter().rev().find(|h| h.contains(x, y)) {
                Some(h) if h.movable && y > h.y + h.h - RESIZE_EDGE => "ns-resize",
                Some(h) if h.movable => "grab",
                Some(_) => "pointer",
                None => "default",
            };
            area.set_cursor_from_name(Some(cursor));
        });
        body.add_controller(motion);
    }

    pub fn refresh(&self, app: &App) {
        let start = if self.state.days == 1 {
            app.date.get()
        } else {
            app.week_start(app.date.get())
        };
        self.state.start.set(start);
        let end = start + Duration::days(self.state.days);
        let occs =
            opencal::recur::occurrences(&app.store.borrow(), start, end, &app.hidden_calendars());
        let colors = app
            .store
            .borrow()
            .calendars
            .keys()
            .map(|uid| (uid.clone(), app.calendar_rgba(uid)))
            .collect();
        *self.state.colors.borrow_mut() = colors;

        // Lanes of the all-day strip.
        let banners: Vec<_> = occs.iter().filter(|o| is_banner(o)).collect();
        let mut lane_ends: Vec<i64> = Vec::new();
        for o in &banners {
            let (a, b) = banner_span(o, start, self.state.days);
            match lane_ends.iter().position(|e| *e < a) {
                Some(i) => lane_ends[i] = b,
                None => lane_ends.push(b),
            }
        }
        self.state.lanes.set(lane_ends.len());
        self.header.set_content_height(
            (DAYNAME_H
                + lane_ends.len() as f64 * LANE_H
                + if lane_ends.is_empty() { 0.0 } else { 6.0 }) as i32,
        );
        *self.state.occs.borrow_mut() = occs;
        self.header.queue_draw();
        self.body.queue_draw();

        if !self.state.scrolled.get() {
            let hour = app.config.borrow().view.day_start_hour.min(23) as f64;
            let adj = self.scroller.vadjustment();
            let st = self.state.clone();
            let apply = move |adj: &gtk::Adjustment| {
                if !st.scrolled.get() && adj.upper() > adj.page_size() && adj.page_size() > 0.0 {
                    adj.set_value(hour * HOUR_H);
                    st.scrolled.set(true);
                }
            };
            apply(&adj);
            adj.connect_changed(apply);
        }
    }
}

/// First and last column (inclusive) a banner covers.
fn banner_span(o: &Occurrence, start: NaiveDate, days: i64) -> (i64, i64) {
    let first = (o.start.date() - start).num_days().max(0);
    let last_day = if o.end > o.start {
        (o.end - Duration::seconds(1)).date()
    } else {
        o.start.date()
    };
    let last = (last_day - start).num_days().min(days - 1);
    (first, last.max(first))
}

fn draw_header(st: &State, area: &gtk::DrawingArea, cr: &cairo::Context, w: f64, _h: f64) {
    let bg = theme_color(area, "theme_bg_color", &gdk::RGBA::WHITE);
    let fg = area.color();
    let sel = theme_color(
        area,
        "theme_selected_bg_color",
        &gdk::RGBA::new(0.21, 0.52, 0.89, 1.0),
    );
    set_color(cr, &bg, 1.0);
    let _ = cr.paint();
    let col = st.col_w(w);
    let t = today();
    for i in 0..st.days {
        let day = st.start.get() + Duration::days(i);
        let x = GUTTER + i as f64 * col;
        let text = if st.days == 1 {
            day.format("%A %-d").to_string()
        } else {
            day.format("%a %-d").to_string()
        };
        let l = layout(area, cr, &text, col - 8.0, day == t);
        l.set_alignment(pango::Alignment::Center);
        if day == t {
            set_color(cr, &sel, 1.0);
        } else {
            set_color(cr, &fg, 0.85);
        }
        let (_, lh) = l.pixel_size();
        cr.move_to(x + 4.0, (DAYNAME_H - lh as f64) / 2.0);
        pangocairo::functions::show_layout(cr, &l);
        if i > 0 {
            set_color(cr, &fg, 0.12);
            cr.rectangle(
                x.floor(),
                0.0,
                1.0,
                DAYNAME_H + st.lanes.get() as f64 * LANE_H + 6.0,
            );
            let _ = cr.fill();
        }
    }
    // All-day strip.
    let mut hits = Vec::new();
    let mut lane_ends: Vec<i64> = Vec::new();
    for o in st.occs.borrow().iter().filter(|o| is_banner(o)) {
        let (a, b) = banner_span(o, st.start.get(), st.days);
        let lane = match lane_ends.iter().position(|e| *e < a) {
            Some(i) => {
                lane_ends[i] = b;
                i
            }
            None => {
                lane_ends.push(b);
                lane_ends.len() - 1
            }
        };
        let x = GUTTER + a as f64 * col + 2.0;
        let y = DAYNAME_H + lane as f64 * LANE_H;
        let bw = (b - a + 1) as f64 * col - 4.0;
        let c = st.color(&o.calendar);
        let text = match o.kind {
            Kind::Task => format!("{} {}", if o.completed { "☑" } else { "☐" }, o.summary),
            Kind::Event if !o.all_day => format!("{} {}", o.start.format("%H:%M"), o.summary),
            Kind::Event => o.summary.clone(),
        };
        if o.kind == Kind::Task {
            set_color(cr, &c, 0.25);
        } else {
            set_color(cr, &c, 1.0);
        }
        cr.rectangle(x, y, bw, LANE_H - 2.0);
        let _ = cr.fill();
        if o.kind == Kind::Task {
            set_color(cr, &c, 1.0);
            cr.rectangle(x, y, 3.0, LANE_H - 2.0);
            let _ = cr.fill();
            set_color(cr, &fg, if o.completed { 0.5 } else { 1.0 });
        } else {
            set_color(cr, &contrast(&c), 1.0);
        }
        let l = layout(area, cr, &text, bw - 10.0, false);
        let (_, lh) = l.pixel_size();
        cr.move_to(x + 6.0, y + (LANE_H - 2.0 - lh as f64) / 2.0);
        pangocairo::functions::show_layout(cr, &l);
        hits.push(Hit {
            x,
            y,
            w: bw,
            h: LANE_H - 2.0,
            uid: o.uid.clone(),
            start: o.start,
            end: o.end,
            movable: false,
        });
    }
    *st.header_hits.borrow_mut() = hits;
}

fn draw_body(st: &State, area: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64) {
    let base = theme_color(area, "theme_base_color", &gdk::RGBA::WHITE);
    let fg = area.color();
    let sel = theme_color(
        area,
        "theme_selected_bg_color",
        &gdk::RGBA::new(0.21, 0.52, 0.89, 1.0),
    );
    let err = theme_color(area, "error_color", &gdk::RGBA::new(0.75, 0.11, 0.16, 1.0));
    set_color(cr, &base, 1.0);
    let _ = cr.paint();
    let col = st.col_w(w);
    let start = st.start.get();
    let t = today();

    // Today's column.
    let ti = (t - start).num_days();
    if (0..st.days).contains(&ti) && st.days > 1 {
        set_color(cr, &sel, 0.07);
        cr.rectangle(GUTTER + ti as f64 * col, 0.0, col, h);
        let _ = cr.fill();
    }
    // Hour lines and labels.
    for hour in 0..24 {
        let y = hour as f64 * HOUR_H;
        set_color(cr, &fg, 0.13);
        cr.rectangle(GUTTER, y.floor(), w - GUTTER, 1.0);
        let _ = cr.fill();
        set_color(cr, &fg, 0.05);
        cr.rectangle(GUTTER, (y + HOUR_H / 2.0).floor(), w - GUTTER, 1.0);
        let _ = cr.fill();
        if hour > 0 {
            set_color(cr, &fg, 0.6);
            let l = layout(area, cr, &format!("{hour:02}:00"), GUTTER - 10.0, false);
            l.set_alignment(pango::Alignment::Right);
            let (_, lh) = l.pixel_size();
            cr.move_to(2.0, y - lh as f64 / 2.0);
            pangocairo::functions::show_layout(cr, &l);
        }
    }
    for i in 0..st.days {
        set_color(cr, &fg, 0.13);
        cr.rectangle((GUTTER + i as f64 * col).floor(), 0.0, 1.0, h);
        let _ = cr.fill();
    }

    // Events, laid out side by side when they overlap.
    let occs = st.occs.borrow();
    let drag = st.drag.borrow().clone();
    let dragged_uid = drag.as_ref().and_then(|d| match &d.mode {
        Mode::Move { hit, .. } | Mode::Resize { hit } if d.moved => Some(hit.uid.clone()),
        _ => None,
    });
    let mut hits = Vec::new();
    for i in 0..st.days {
        let day = start + Duration::days(i);
        let d0 = day.and_time(NaiveTime::MIN);
        let d1 = d0 + Duration::days(1);
        let mut evs: Vec<(f64, f64, &Occurrence)> = occs
            .iter()
            .filter(|o| !is_banner(o) && o.on_day(day))
            .map(|o| {
                let s = o.start.max(d0);
                let e = o.end.min(d1).max(s);
                (
                    (s - d0).num_minutes() as f64,
                    (e - d0).num_minutes() as f64,
                    o,
                )
            })
            .collect();
        evs.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.total_cmp(&a.1)));
        // Greedy columns within clusters of overlapping events.
        let mut placed: Vec<(usize, usize)> = vec![(0, 1); evs.len()];
        let mut cluster: Vec<usize> = Vec::new();
        let mut col_ends: Vec<f64> = Vec::new();
        let mut cluster_end = f64::MIN;
        let min_len = 20.0 / HOUR_H * 60.0;
        for (k, (s, e, _)) in evs.iter().enumerate() {
            let e = e.max(s + min_len);
            if *s >= cluster_end && !cluster.is_empty() {
                for &m in &cluster {
                    placed[m].1 = col_ends.len();
                }
                cluster.clear();
                col_ends.clear();
            }
            let c = match col_ends.iter().position(|end| *end <= *s) {
                Some(c) => {
                    col_ends[c] = e;
                    c
                }
                None => {
                    col_ends.push(e);
                    col_ends.len() - 1
                }
            };
            placed[k].0 = c;
            cluster.push(k);
            cluster_end = if cluster.len() == 1 {
                e
            } else {
                cluster_end.max(e)
            };
        }
        for &m in &cluster {
            placed[m].1 = col_ends.len().max(1);
        }
        for (k, (s, e, o)) in evs.iter().enumerate() {
            let (c, n) = placed[k];
            let cw = (col - 4.0) / n as f64;
            let x = GUTTER + i as f64 * col + 2.0 + c as f64 * cw;
            let y = s / 60.0 * HOUR_H;
            let eh = ((e - s) / 60.0 * HOUR_H).max(20.0);
            let color = st.color(&o.calendar);
            let ghosted = dragged_uid.as_deref() == Some(o.uid.as_str());
            set_color(cr, &color, if ghosted { 0.35 } else { 1.0 });
            cr.rectangle(x, y + 1.0, cw - 1.0, eh - 2.0);
            let _ = cr.fill();
            set_color(cr, &contrast(&color), 1.0);
            let _ = cr.save();
            cr.rectangle(x, y, cw - 1.0, eh);
            cr.clip();
            let time = format!("{}–{}", o.start.format("%H:%M"), o.end.format("%H:%M"));
            let item_loc = st
                .app
                .upgrade()
                .and_then(|a| {
                    a.store
                        .borrow()
                        .items
                        .get(&o.uid)
                        .map(|i| i.location.clone())
                })
                .unwrap_or_default();
            if eh >= 38.0 {
                let l = layout(area, cr, &o.summary, cw - 10.0, true);
                cr.move_to(x + 5.0, y + 3.0);
                pangocairo::functions::show_layout(cr, &l);
                let second = if item_loc.is_empty() {
                    time
                } else {
                    format!("{time} · {item_loc}")
                };
                let l2 = layout(area, cr, &second, cw - 10.0, false);
                let (_, lh) = l.pixel_size();
                cr.move_to(x + 5.0, y + 3.0 + lh as f64);
                pangocairo::functions::show_layout(cr, &l2);
            } else {
                let l = layout(
                    area,
                    cr,
                    &format!("{} {}", o.start.format("%H:%M"), o.summary),
                    cw - 10.0,
                    false,
                );
                cr.move_to(x + 5.0, y + 1.0);
                pangocairo::functions::show_layout(cr, &l);
            }
            let _ = cr.restore();
            hits.push(Hit {
                x,
                y,
                w: cw - 1.0,
                h: eh,
                uid: o.uid.clone(),
                start: o.start,
                end: o.end,
                // Recurring series are edited in the dialog: dragging one occurrence would move
                // the whole series.
                movable: !o.recurring && o.kind == Kind::Event,
            });
        }
    }
    *st.hits.borrow_mut() = hits;

    // Drag preview.
    if let Some(d) = drag.filter(|d| d.moved) {
        let preview = match &d.mode {
            Mode::Create { anchor } => {
                let (a, b) = if d.cur < *anchor {
                    (d.cur, *anchor)
                } else {
                    (*anchor, d.cur)
                };
                Some((a, b + Duration::minutes(SNAP_MIN)))
            }
            Mode::Move { hit, anchor } => {
                Some((hit.start + (d.cur - *anchor), hit.end + (d.cur - *anchor)))
            }
            Mode::Resize { hit } => Some((
                hit.start,
                (d.cur + Duration::minutes(SNAP_MIN)).max(hit.start + Duration::minutes(SNAP_MIN)),
            )),
            Mode::Click { .. } => None,
        };
        if let Some((a, b)) = preview {
            let i = (a.date() - start).num_days();
            if (0..st.days).contains(&i) {
                let y0 =
                    (a - a.date().and_time(NaiveTime::MIN)).num_minutes() as f64 / 60.0 * HOUR_H;
                let mins = (b - a).num_minutes().min(24 * 60) as f64;
                let x = GUTTER + i as f64 * col + 2.0;
                set_color(cr, &sel, 0.45);
                cr.rectangle(x, y0, col - 4.0, mins / 60.0 * HOUR_H);
                let _ = cr.fill_preserve();
                set_color(cr, &sel, 1.0);
                cr.set_line_width(2.0);
                let _ = cr.stroke();
                set_color(cr, &fg, 1.0);
                let l = layout(
                    area,
                    cr,
                    &format!("{}–{}", a.format("%H:%M"), b.format("%H:%M")),
                    col - 12.0,
                    true,
                );
                cr.move_to(x + 5.0, y0 + 3.0);
                pangocairo::functions::show_layout(cr, &l);
            }
        }
    }

    // Now.
    if (0..st.days).contains(&ti) {
        let now = chrono::Local::now().naive_local().time();
        let y = (now.hour() as f64 + now.minute() as f64 / 60.0) * HOUR_H;
        set_color(cr, &err, 1.0);
        cr.rectangle(GUTTER + ti as f64 * col, y - 1.0, col, 2.0);
        let _ = cr.fill();
        cr.arc(GUTTER + ti as f64 * col, y, 4.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
}
