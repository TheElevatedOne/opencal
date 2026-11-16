//! The main window: toolbar, sidebar (month calendar and calendar list), the views, and the
//! glue between them and the calendar file.

pub mod agenda;
pub mod editor;
pub mod month;
pub mod settings;
pub mod tasks;
pub mod timegrid;
pub mod widgets;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use chrono::{
    Datelike, Duration as CDuration, Local, Months, NaiveDate, NaiveDateTime, NaiveTime, Timelike,
};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use opencal::{Calendar, Config, Item, Kind, Paths, Store, When, ics, sync};

use widgets::{contrast, parse_hex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Day,
    Week,
    Month,
    Agenda,
    Tasks,
}

impl View {
    pub const ALL: [View; 5] = [
        View::Day,
        View::Week,
        View::Month,
        View::Agenda,
        View::Tasks,
    ];

    pub fn name(self) -> &'static str {
        match self {
            View::Day => "day",
            View::Week => "week",
            View::Month => "month",
            View::Agenda => "agenda",
            View::Tasks => "tasks",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            View::Day => "Day",
            View::Week => "Week",
            View::Month => "Month",
            View::Agenda => "Agenda",
            View::Tasks => "Tasks",
        }
    }

    pub fn from_name(s: &str) -> Option<View> {
        View::ALL.into_iter().find(|v| v.name() == s)
    }
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

pub struct App {
    pub window: gtk::ApplicationWindow,
    pub paths: Paths,
    pub config: RefCell<Config>,
    pub store: RefCell<Store>,
    db: RefCell<PathBuf>,
    last_written: RefCell<String>,
    /// Set when the file could not be read: saving is refused so it isn't overwritten.
    read_only: Cell<bool>,
    pub date: Cell<NaiveDate>,
    pub view: Cell<View>,
    title: gtk::Label,
    stack: gtk::Stack,
    view_buttons: RefCell<Vec<(View, gtk::ToggleButton)>>,
    mini: gtk::Calendar,
    cal_list: gtk::ListBox,
    sidebar: gtk::Box,
    content: gtk::Box,
    status: gtk::Label,
    spinner: gtk::Spinner,
    colors_css: gtk::CssProvider,
    color_classes: RefCell<HashMap<String, String>>,
    month: RefCell<Option<month::MonthView>>,
    week: RefCell<Option<timegrid::TimeGrid>>,
    day: RefCell<Option<timegrid::TimeGrid>>,
    agenda: RefCell<Option<agenda::AgendaView>>,
    tasks: RefCell<Option<tasks::TasksView>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    reload_pending: Cell<bool>,
    refresh_pending: Cell<bool>,
    syncing: Cell<bool>,
    sync_timer: RefCell<Option<glib::SourceId>>,
    pub open_editors: RefCell<HashMap<String, gtk::Window>>,
    me: RefCell<Weak<App>>,
}

impl App {
    pub fn new(application: &gtk::Application, paths: Paths) -> Rc<App> {
        let (config, config_error) = match Config::load(&paths) {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(format!("{e:#}"))),
        };
        let db = paths.db(&config);

        let window = gtk::ApplicationWindow::builder()
            .application(application)
            .title("opencal")
            .default_width(config.view.width.max(400))
            .default_height(config.view.height.max(300))
            .build();
        window.add_css_class("opencal");
        // No client-side title bar: the window manager (Hyprland) handles the window.
        window.set_decorated(false);

        let colors_css = gtk::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            #[allow(deprecated)]
            gtk::style_context_add_provider_for_display(
                &display,
                &colors_css,
                gtk::STYLE_PROVIDER_PRIORITY_USER + 2,
            );
        }

        let app = Rc::new(App {
            window,
            paths,
            date: Cell::new(today()),
            view: Cell::new(View::from_name(&config.view.view).unwrap_or(View::Month)),
            config: RefCell::new(config),
            store: RefCell::new(Store::new()),
            db: RefCell::new(db),
            last_written: RefCell::new(String::new()),
            read_only: Cell::new(false),
            title: gtk::Label::new(None),
            stack: gtk::Stack::new(),
            view_buttons: RefCell::new(Vec::new()),
            mini: gtk::Calendar::new(),
            cal_list: gtk::ListBox::new(),
            sidebar: gtk::Box::new(gtk::Orientation::Vertical, 8),
            content: gtk::Box::new(gtk::Orientation::Vertical, 0),
            status: gtk::Label::new(None),
            spinner: gtk::Spinner::new(),
            colors_css,
            color_classes: RefCell::new(HashMap::new()),
            month: RefCell::new(None),
            week: RefCell::new(None),
            day: RefCell::new(None),
            agenda: RefCell::new(None),
            tasks: RefCell::new(None),
            monitor: RefCell::new(None),
            reload_pending: Cell::new(false),
            refresh_pending: Cell::new(false),
            syncing: Cell::new(false),
            sync_timer: RefCell::new(None),
            open_editors: RefCell::new(HashMap::new()),
            me: RefCell::new(Weak::new()),
        });
        *app.me.borrow_mut() = Rc::downgrade(&app);
        app.build_ui();
        app.setup_actions(application);
        let loaded = app.load_from_disk();
        if let Err(e) = loaded {
            app.set_status(&format!("Could not read the calendar file: {e:#}"));
            widgets::error(
                &app.window,
                "Could not read the calendar file",
                &format!("{e:#}\n\nopencal will not overwrite it until it can be read."),
            );
        }
        if let Some(e) = config_error {
            app.set_status(&format!("Settings not loaded: {e}"));
        }
        app.watch_file();
        app.set_view(app.view.get());
        app.restart_sync_timer();
        // The window owns the app: every signal handler only holds a Weak<App>, so without this
        // strong reference the app would be freed as soon as the caller drops its Rc, and every
        // button would silently stop working.
        {
            let keep = RefCell::new(Some(app.clone()));
            app.window.connect_destroy(move |_| {
                keep.borrow_mut().take();
            });
        }
        app.window.connect_close_request({
            let app = app.weak();
            move |w| {
                if let Some(app) = app.upgrade() {
                    let mut c = app.config.borrow_mut();
                    c.view.width = w.default_width();
                    c.view.height = w.default_height();
                    c.view.view = app.view.get().name().to_string();
                    let _ = c.save(&app.paths);
                }
                glib::Propagation::Proceed
            }
        });
        app
    }

    pub fn weak(&self) -> Weak<App> {
        self.me.borrow().clone()
    }

    pub fn rc(&self) -> Rc<App> {
        self.me.borrow().upgrade().expect("app alive")
    }

    // ------------------------------------------------------------------------------------------
    // Layout
    // ------------------------------------------------------------------------------------------

    fn build_ui(self: &Rc<Self>) {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // Toolbar
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.add_css_class("toolbar");
        let sidebar_toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Show sidebar (F9)")
            .active(self.config.borrow().view.sidebar)
            .action_name("win.toggle-sidebar")
            .build();
        let new_menu = gio::Menu::new();
        new_menu.append(Some("Event"), Some("win.new-event"));
        new_menu.append(Some("Task"), Some("win.new-task"));
        let new_button = gtk::MenuButton::builder()
            .label("New")
            .menu_model(&new_menu)
            .tooltip_text("New event (Ctrl+N) or task (Ctrl+T)")
            .build();
        let nav = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        nav.add_css_class("linked");
        nav.append(
            &gtk::Button::builder()
                .icon_name("go-previous-symbolic")
                .action_name("win.prev")
                .tooltip_text("Previous (Page Up)")
                .build(),
        );
        nav.append(
            &gtk::Button::builder()
                .label("Today")
                .action_name("win.today")
                .tooltip_text("Today (T)")
                .build(),
        );
        nav.append(
            &gtk::Button::builder()
                .icon_name("go-next-symbolic")
                .action_name("win.next")
                .tooltip_text("Next (Page Down)")
                .build(),
        );
        self.title.add_css_class("view-title");
        self.title.set_hexpand(true);
        self.title.set_width_chars(14);
        self.title.set_xalign(0.0);
        self.title.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let views = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        views.add_css_class("linked");
        let mut buttons = Vec::new();
        let mut group: Option<gtk::ToggleButton> = None;
        for (i, v) in View::ALL.into_iter().enumerate() {
            let b = gtk::ToggleButton::builder()
                .label(v.label())
                .tooltip_text(format!("{} (Ctrl+{})", v.label(), i + 1))
                .build();
            if let Some(g) = &group {
                b.set_group(Some(g));
            } else {
                group = Some(b.clone());
            }
            let app = self.weak();
            b.connect_toggled(move |b| {
                if b.is_active()
                    && let Some(app) = app.upgrade()
                    && (app.view.get() != v
                        || app.stack.visible_child_name().as_deref() != Some(v.name()))
                {
                    app.set_view(v);
                }
            });
            views.append(&b);
            buttons.push((v, b));
        }
        *self.view_buttons.borrow_mut() = buttons;

        let sync_button = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Sync (F5)")
            .action_name("win.sync")
            .build();
        let menu = gio::Menu::new();
        let file_section = gio::Menu::new();
        file_section.append(Some("Import iCalendar…"), Some("win.import"));
        file_section.append(Some("Export iCalendar…"), Some("win.export"));
        file_section.append(Some("Merge with Calendar File…"), Some("win.merge-file"));
        menu.append_section(None, &file_section);
        let sync_section = gio::Menu::new();
        sync_section.append(Some("Sync Now"), Some("win.sync"));
        sync_section.append(Some("Sync Settings…"), Some("win.sync-settings"));
        menu.append_section(None, &sync_section);
        let cal_section = gio::Menu::new();
        cal_section.append(Some("New Calendar…"), Some("win.new-calendar"));
        cal_section.append(Some("Week Starts on Monday"), Some("win.week-monday"));
        menu.append_section(None, &cal_section);
        let app_section = gio::Menu::new();
        app_section.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
        app_section.append(Some("About opencal"), Some("win.about"));
        app_section.append(Some("Quit"), Some("app.quit"));
        menu.append_section(None, &app_section);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Menu (F10)")
            .build();
        menu_button.set_primary(true);

        bar.append(&sidebar_toggle);
        bar.append(&new_button);
        bar.append(&nav);
        bar.append(&self.title);
        bar.append(&self.spinner);
        bar.append(&views);
        bar.append(&sync_button);
        bar.append(&menu_button);
        root.append(&bar);

        // Sidebar
        self.sidebar.add_css_class("sidebar");
        self.sidebar.set_width_request(200);
        // Set explicitly so expanding children (labels, lists) don't make the sidebar take half
        // the window.
        self.sidebar.set_hexpand(false);
        self.mini.add_css_class("mini-calendar");
        {
            let app = self.weak();
            self.mini.connect_day_selected(move |cal| {
                if let Some(app) = app.upgrade() {
                    let d = widgets::from_glib_date(&cal.date());
                    if d != app.date.get() {
                        app.date.set(d);
                        app.refresh();
                    }
                }
            });
        }
        // Moving the month in the mini calendar doesn't select a day; mark its events anyway.
        for signal in ["next-month", "prev-month", "next-year", "prev-year"] {
            let app = self.weak();
            self.mini.connect_local(signal, false, move |_| {
                if let Some(app) = app.upgrade() {
                    app.mark_mini();
                }
                None
            });
        }
        self.sidebar.append(&self.mini);
        let cal_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let cal_title = widgets::label("Calendars");
        cal_title.add_css_class("section-title");
        cal_title.set_hexpand(true);
        cal_header.append(&cal_title);
        cal_header.append(
            &gtk::Button::builder()
                .icon_name("list-add-symbolic")
                .action_name("win.new-calendar")
                .tooltip_text("New calendar")
                .css_classes(["flat"])
                .build(),
        );
        self.sidebar.append(&cal_header);
        self.cal_list.add_css_class("calendar-list");
        self.cal_list.set_selection_mode(gtk::SelectionMode::None);
        let cal_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&self.cal_list)
            .build();
        self.sidebar.append(&cal_scroll);
        self.sidebar.set_visible(self.config.borrow().view.sidebar);

        // Views
        let month = month::MonthView::new(self);
        let week = timegrid::TimeGrid::new(self, 7);
        let day = timegrid::TimeGrid::new(self, 1);
        let agenda = agenda::AgendaView::new(self);
        let tasks = tasks::TasksView::new(self);
        self.stack.add_named(&day.widget, Some(View::Day.name()));
        self.stack.add_named(&week.widget, Some(View::Week.name()));
        self.stack
            .add_named(&month.widget, Some(View::Month.name()));
        self.stack
            .add_named(&agenda.widget, Some(View::Agenda.name()));
        self.stack
            .add_named(&tasks.widget, Some(View::Tasks.name()));
        self.stack.set_hexpand(true);
        self.stack.set_vexpand(true);
        *self.month.borrow_mut() = Some(month);
        *self.week.borrow_mut() = Some(week);
        *self.day.borrow_mut() = Some(day);
        *self.agenda.borrow_mut() = Some(agenda);
        *self.tasks.borrow_mut() = Some(tasks);

        // Drag the divider to resize the sidebar.
        let body = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&self.sidebar)
            .end_child(&self.stack)
            .resize_start_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .position(self.config.borrow().view.sidebar_width.max(200))
            .vexpand(true)
            .build();
        {
            let app = self.weak();
            body.connect_position_notify(move |p| {
                if let Some(app) = app.upgrade()
                    && app.sidebar.is_visible()
                    && p.position() > 0
                {
                    app.config.borrow_mut().view.sidebar_width = p.position();
                }
            });
        }
        self.content.append(&body);
        self.content.set_vexpand(true);
        root.append(&self.content);

        let statusbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        statusbar.add_css_class("statusbar");
        self.status.set_xalign(0.0);
        self.status.set_hexpand(true);
        self.status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        statusbar.append(&self.status);
        root.append(&statusbar);

        self.window.set_child(Some(&root));
    }

    fn setup_actions(self: &Rc<Self>, application: &gtk::Application) {
        let win = &self.window;
        let add = |name: &str, f: Box<dyn Fn(&Rc<App>)>| {
            let action = gio::SimpleAction::new(name, None);
            let app = self.weak();
            action.connect_activate(move |_, _| {
                if let Some(app) = app.upgrade() {
                    f(&app);
                }
            });
            win.add_action(&action);
        };
        add("new-event", Box::new(|app| app.new_event_at(None)));
        add(
            "new-task",
            Box::new(|app| editor::open_task(app, None, None)),
        );
        add("today", Box::new(|app| app.go_to(today())));
        add("prev", Box::new(|app| app.step(-1)));
        add("next", Box::new(|app| app.step(1)));
        add("sync", Box::new(|app| app.sync_now(true)));
        add("sync-settings", Box::new(settings::open_sync_settings));
        add("import", Box::new(|app| app.import_dialog()));
        add("export", Box::new(|app| app.export_dialog()));
        add("merge-file", Box::new(|app| app.merge_dialog()));
        add(
            "new-calendar",
            Box::new(|app| settings::open_calendar(app, None)),
        );
        add("about", Box::new(|app| app.about()));
        add("shortcuts", Box::new(|app| app.shortcuts()));
        // Stateful, so the toolbar toggle button and F9 stay in step.
        let sidebar = gio::SimpleAction::new_stateful(
            "toggle-sidebar",
            None,
            &self.config.borrow().view.sidebar.to_variant(),
        );
        {
            let app = self.weak();
            sidebar.connect_activate(move |a, _| {
                let show = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
                a.set_state(&show.to_variant());
                if let Some(app) = app.upgrade() {
                    app.sidebar.set_visible(show);
                    app.config.borrow_mut().view.sidebar = show;
                    let _ = app.config.borrow().save(&app.paths);
                }
            });
        }
        win.add_action(&sidebar);
        for (i, v) in View::ALL.into_iter().enumerate() {
            add(
                &format!("view-{}", v.name()),
                Box::new(move |app| app.set_view(v)),
            );
            application.set_accels_for_action(
                &format!("win.view-{}", v.name()),
                &[&format!("<Control>{}", i + 1)],
            );
        }
        let monday = gio::SimpleAction::new_stateful(
            "week-monday",
            None,
            &self.config.borrow().view.week_starts_monday.to_variant(),
        );
        {
            let app = self.weak();
            monday.connect_activate(move |a, _| {
                let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
                a.set_state(&on.to_variant());
                if let Some(app) = app.upgrade() {
                    app.config.borrow_mut().view.week_starts_monday = on;
                    let _ = app.config.borrow().save(&app.paths);
                    app.refresh();
                }
            });
        }
        win.add_action(&monday);

        let quit = gio::SimpleAction::new("quit", None);
        {
            let app = self.weak();
            quit.connect_activate(move |_, _| {
                if let Some(app) = app.upgrade() {
                    app.window.close();
                }
            });
        }
        application.add_action(&quit);

        for (action, accels) in [
            ("win.new-event", &["<Control>n"][..]),
            ("win.new-task", &["<Control>t"]),
            ("win.today", &["t", "Home"]),
            ("win.prev", &["Page_Up", "<Alt>Left"]),
            ("win.next", &["Page_Down", "<Alt>Right"]),
            ("win.sync", &["F5", "<Control>r"]),
            ("win.import", &["<Control>i"]),
            ("win.export", &["<Control>e"]),
            ("win.toggle-sidebar", &["F9"]),
            ("win.shortcuts", &["<Control>question"]),
            ("app.quit", &["<Control>q"]),
        ] {
            application.set_accels_for_action(action, accels);
        }
    }

    // ------------------------------------------------------------------------------------------
    // Views and navigation
    // ------------------------------------------------------------------------------------------

    pub fn set_view(&self, v: View) {
        self.view.set(v);
        self.stack.set_visible_child_name(v.name());
        for (bv, b) in self.view_buttons.borrow().iter() {
            if *bv == v && !b.is_active() {
                b.set_active(true);
            }
        }
        self.config.borrow_mut().view.view = v.name().to_string();
        self.refresh();
    }

    pub fn go_to(&self, d: NaiveDate) {
        self.date.set(d);
        self.refresh();
    }

    pub fn show_day(&self, d: NaiveDate) {
        self.date.set(d);
        self.set_view(View::Day);
    }

    fn step(&self, dir: i32) {
        let d = self.date.get();
        let n = match self.view.get() {
            View::Day => d + CDuration::days(dir as i64),
            View::Week => d + CDuration::days(7 * dir as i64),
            View::Month | View::Tasks => if dir > 0 {
                d.checked_add_months(Months::new(1))
            } else {
                d.checked_sub_months(Months::new(1))
            }
            .unwrap_or(d),
            View::Agenda => d + CDuration::days(30 * dir as i64),
        };
        self.go_to(n);
    }

    pub fn week_start(&self, d: NaiveDate) -> NaiveDate {
        let monday = self.config.borrow().view.week_starts_monday;
        let offset = if monday {
            d.weekday().num_days_from_monday()
        } else {
            d.weekday().num_days_from_sunday()
        };
        d - CDuration::days(offset as i64)
    }

    pub fn hidden_calendars(&self) -> HashSet<String> {
        self.config
            .borrow()
            .view
            .hidden_calendars
            .iter()
            .cloned()
            .collect()
    }

    /// Redraw everything soon (coalesces several changes into one refresh).
    pub fn queue_refresh(&self) {
        if self.refresh_pending.replace(true) {
            return;
        }
        let app = self.weak();
        glib::idle_add_local_once(move || {
            if let Some(app) = app.upgrade() {
                app.refresh();
            }
        });
    }

    pub fn refresh(&self) {
        self.refresh_pending.set(false);
        let d = self.date.get();
        let title = match self.view.get() {
            View::Day => d.format("%A %-d %B %Y").to_string(),
            View::Week => {
                let s = self.week_start(d);
                let e = s + CDuration::days(6);
                if s.month() == e.month() {
                    format!("{} – {}", s.format("%-d"), e.format("%-d %B %Y"))
                } else if s.year() == e.year() {
                    format!("{} – {}", s.format("%-d %b"), e.format("%-d %b %Y"))
                } else {
                    format!("{} – {}", s.format("%-d %b %Y"), e.format("%-d %b %Y"))
                }
            }
            View::Month => d.format("%B %Y").to_string(),
            View::Agenda => format!("From {}", d.format("%-d %B %Y")),
            View::Tasks => "Tasks".to_string(),
        };
        self.title.set_label(&title);
        if widgets::from_glib_date(&self.mini.date()) != d {
            self.mini.select_day(&widgets::glib_date(d));
        }
        self.mark_mini();
        match self.view.get() {
            View::Month => self.month.borrow().as_ref().unwrap().refresh(self),
            View::Week => self.week.borrow().as_ref().unwrap().refresh(self),
            View::Day => self.day.borrow().as_ref().unwrap().refresh(self),
            View::Agenda => self.agenda.borrow().as_ref().unwrap().refresh(self),
            View::Tasks => self.tasks.borrow().as_ref().unwrap().refresh(self),
        }
    }

    /// Show the selected date in the sidebar calendar (without a full refresh).
    pub fn sync_mini(&self) {
        let d = self.date.get();
        if widgets::from_glib_date(&self.mini.date()) != d {
            self.mini.select_day(&widgets::glib_date(d));
        }
    }

    /// Bold the days with something on them in the sidebar calendar.
    fn mark_mini(&self) {
        self.mini.clear_marks();
        let shown = widgets::from_glib_date(&self.mini.date());
        let first = shown.with_day(1).unwrap_or(shown);
        let last = first.checked_add_months(Months::new(1)).unwrap_or(first);
        let occ = opencal::recur::occurrences(
            &self.store.borrow(),
            first,
            last,
            &self.hidden_calendars(),
        );
        let mut day = first;
        while day < last {
            if occ.iter().any(|o| o.on_day(day)) {
                self.mini.mark_day(day.day());
            }
            day += CDuration::days(1);
        }
    }

    // ------------------------------------------------------------------------------------------
    // Calendars and colours
    // ------------------------------------------------------------------------------------------

    /// CSS class for a calendar's colour (`cal-<n>`).
    pub fn color_class(&self, calendar: &str) -> String {
        self.color_classes
            .borrow()
            .get(calendar)
            .cloned()
            .unwrap_or_else(|| "cal-none".into())
    }

    pub fn calendar_rgba(&self, calendar: &str) -> gdk::RGBA {
        let store = self.store.borrow();
        let hex = store
            .calendars
            .get(calendar)
            .map(|c| c.color.clone())
            .unwrap_or_else(|| "#3584e4".into());
        parse_hex(&hex)
    }

    /// Rebuild the colour CSS and the sidebar list of calendars.
    fn refresh_calendars(&self) {
        let store = self.store.borrow();
        let mut css = String::new();
        let mut classes = HashMap::new();
        for (i, cal) in store.calendars.values().enumerate() {
            let class = format!("cal-{i}");
            let bg = parse_hex(&cal.color);
            let fg = widgets::to_hex(&contrast(&bg));
            let c = widgets::to_hex(&bg);
            css.push_str(&format!(
                ".{class}.swatch, .{class}.event-pill {{ background-color: {c}; color: {fg}; }}\n\
                 .{class}.event-dot .dot {{ color: {c}; }}\n\
                 .{class}.colored-check check:checked {{ background-color: {c}; border-color: {c}; color: {fg}; }}\n"
            ));
            classes.insert(cal.uid.clone(), class);
        }
        self.colors_css.load_from_string(&css);
        *self.color_classes.borrow_mut() = classes;

        while let Some(row) = self.cal_list.first_child() {
            self.cal_list.remove(&row);
        }
        let hidden = self.hidden_calendars();
        for cal in store.live_calendars() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("calendar-row");
            let check = gtk::CheckButton::builder()
                .active(!hidden.contains(&cal.uid))
                .tooltip_text("Show this calendar")
                .build();
            check.add_css_class("colored-check");
            check.add_css_class(&self.color_class(&cal.uid));
            let name = widgets::label(&cal.name);
            name.set_hexpand(true);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let edit = gtk::Button::builder()
                .icon_name("document-edit-symbolic")
                .tooltip_text("Edit calendar")
                .css_classes(["flat"])
                .build();
            row.append(&check);
            row.append(&name);
            row.append(&edit);
            let uid = cal.uid.clone();
            let app = self.weak();
            check.connect_toggled(move |c| {
                if let Some(app) = app.upgrade() {
                    {
                        let mut cfg = app.config.borrow_mut();
                        cfg.view.hidden_calendars.retain(|u| u != &uid);
                        if !c.is_active() {
                            cfg.view.hidden_calendars.push(uid.clone());
                        }
                        let _ = cfg.save(&app.paths);
                    }
                    app.queue_refresh();
                }
            });
            let uid = cal.uid.clone();
            let app = self.weak();
            edit.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    settings::open_calendar(&app, Some(&uid));
                }
            });
            self.cal_list.append(&row);
        }
    }

    /// The calendar new entries go to: the first visible one.
    pub fn default_calendar(&self) -> String {
        let store = self.store.borrow();
        let hidden = self.hidden_calendars();
        let cals = store.live_calendars();
        cals.iter()
            .find(|c| !hidden.contains(&c.uid))
            .or(cals.first())
            .map(|c| c.uid.clone())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------------------------------------
    // Changing the calendar
    // ------------------------------------------------------------------------------------------

    /// Apply a change to the store, save the file and redraw.
    pub fn change(&self, f: impl FnOnce(&mut Store)) {
        f(&mut self.store.borrow_mut());
        self.store.borrow_mut().ensure_default_calendar();
        self.save();
        self.refresh_calendars();
        self.queue_refresh();
    }

    pub fn save(&self) {
        if self.read_only.get() {
            self.set_status(
                "Not saved: the calendar file could not be read (fix it, then restart opencal)",
            );
            return;
        }
        let text = ics::write(&self.store.borrow());
        if *self.last_written.borrow() == text {
            return;
        }
        let db = self.db.borrow().clone();
        match opencal::write_atomic(&db, text.as_bytes()) {
            Ok(()) => {
                *self.last_written.borrow_mut() = text;
            }
            Err(e) => {
                self.set_status(&format!("Saving failed: {e:#}"));
                widgets::error(
                    &self.window,
                    "Could not save the calendar",
                    &format!("{e:#}"),
                );
            }
        }
    }

    /// Read the calendar file again. Returns an error and keeps the current data when it can't
    /// be read.
    pub fn load_from_disk(&self) -> anyhow::Result<()> {
        let db = self.db.borrow().clone();
        let text = match std::fs::read_to_string(&db) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                *self.store.borrow_mut() = Store::new();
                self.save();
                self.refresh_calendars();
                self.queue_refresh();
                return Ok(());
            }
            Err(e) => {
                self.read_only.set(self.last_written.borrow().is_empty());
                return Err(e.into());
            }
        };
        if *self.last_written.borrow() == text {
            return Ok(());
        }
        let mut store = match ics::parse(&text) {
            Ok(s) => s,
            Err(e) => {
                // Only refuse to save when nothing was ever loaded; a bad file written by a
                // sync later is replaced by the good copy in memory.
                self.read_only.set(self.last_written.borrow().is_empty());
                return Err(e);
            }
        };
        self.read_only.set(false);
        store.ensure_default_calendar();
        *self.store.borrow_mut() = store;
        *self.last_written.borrow_mut() = text;
        self.refresh_calendars();
        self.queue_refresh();
        Ok(())
    }

    /// Reload when another program (a sync script, `opencal --sync`, the C library) changes the
    /// file.
    fn watch_file(&self) {
        let db = self.db.borrow().clone();
        let Some(dir) = db.parent() else { return };
        let name = db.file_name().map(|n| n.to_owned());
        let Ok(monitor) = gio::File::for_path(dir)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        else {
            return;
        };
        let app = self.weak();
        monitor.connect_changed(move |_, file, other, _| {
            let hits = |f: Option<&gio::File>| {
                f.and_then(|f| f.basename())
                    .map(|n| Some(n.into_os_string()) == name)
                    .unwrap_or(false)
            };
            if !(hits(Some(file)) || hits(other)) {
                return;
            }
            let Some(app) = app.upgrade() else { return };
            if app.syncing.get() || app.reload_pending.replace(true) {
                return;
            }
            let weak = app.weak();
            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                if let Some(app) = weak.upgrade() {
                    app.reload_pending.set(false);
                    let before = app.last_written.borrow().clone();
                    match app.load_from_disk() {
                        Ok(()) if *app.last_written.borrow() != before => app
                            .set_status("Reloaded: the calendar file was changed outside opencal"),
                        Ok(()) => {}
                        Err(e) => app.set_status(&format!(
                            "The calendar file changed but could not be read: {e:#}"
                        )),
                    }
                }
            });
        });
        *self.monitor.borrow_mut() = Some(monitor);
    }

    pub fn set_status(&self, text: &str) {
        let time = Local::now().format("%H:%M");
        self.status.set_label(&format!("{time}  {text}"));
    }

    // ------------------------------------------------------------------------------------------
    // Events and tasks
    // ------------------------------------------------------------------------------------------

    /// New event dialog. `start` = None uses the selected day at the next full hour.
    pub fn new_event_at(&self, range: Option<(When, When)>) {
        let (start, end) = range.unwrap_or_else(|| {
            let now = Local::now().naive_local();
            let d = self.date.get();
            let hour = if d == today() {
                (now.hour() + 1).min(23)
            } else {
                9
            };
            let s = d.and_time(NaiveTime::from_hms_opt(hour, 0, 0).unwrap());
            (When::DateTime(s), When::DateTime(s + CDuration::hours(1)))
        });
        let item = Item::new_event(&self.default_calendar(), "", start, end);
        editor::open_event(&self.rc(), None, Some(item), None);
    }

    /// Open the editor for an item; `occurrence` is the start of the clicked occurrence.
    pub fn open_item(&self, uid: &str, occurrence: Option<NaiveDateTime>) {
        if let Some(w) = self.open_editors.borrow().get(uid) {
            w.present();
            return;
        }
        let Some(item) = self.store.borrow().items.get(uid).cloned() else {
            return;
        };
        match item.kind {
            Kind::Event => editor::open_event(&self.rc(), Some(item), None, occurrence),
            Kind::Task => editor::open_task(&self.rc(), Some(item), None),
        }
    }

    pub fn set_task_done(&self, uid: &str, done: bool) {
        self.change(|s| {
            if let Some(i) = s.items.get_mut(uid) {
                i.set_completed(done);
            }
        });
    }

    /// Move (and/or resize) an event by dragging.
    pub fn shift_event(&self, uid: &str, start_by: CDuration, end_by: CDuration) {
        self.change(|s| {
            if let Some(i) = s.items.get_mut(uid) {
                let shift = |w: When, by: CDuration| match w {
                    When::Date(d) => When::Date(d + CDuration::days(by.num_days())),
                    When::DateTime(dt) => When::DateTime(dt + by),
                };
                let end = i.effective_end().map(|e| {
                    if i.is_all_day() {
                        When::Date(e.date())
                    } else {
                        When::DateTime(e)
                    }
                });
                i.start = i.start.map(|w| shift(w, start_by));
                i.end = end.map(|w| shift(w, end_by));
                i.touch();
            }
        });
    }

    // ------------------------------------------------------------------------------------------
    // Sync, import, export
    // ------------------------------------------------------------------------------------------

    pub fn restart_sync_timer(&self) {
        if let Some(id) = self.sync_timer.borrow_mut().take() {
            id.remove();
        }
        let minutes = self.config.borrow().sync.interval_minutes;
        if minutes > 0 {
            let app = self.weak();
            let id = glib::timeout_add_seconds_local(minutes * 60, move || {
                if let Some(app) = app.upgrade() {
                    app.sync_now(false);
                }
                glib::ControlFlow::Continue
            });
            *self.sync_timer.borrow_mut() = Some(id);
        }
    }

    pub fn sync_configured(&self) -> bool {
        let c = self.config.borrow();
        !c.sync.pull.trim().is_empty() || !c.sync.push.trim().is_empty()
    }

    /// Run the sync commands in the background. `interactive`: show the result dialog
    /// (otherwise only errors open it).
    pub fn sync_now(&self, interactive: bool) {
        if self.syncing.get() {
            return;
        }
        if !self.sync_configured() {
            if interactive {
                settings::open_sync_settings(&self.rc());
            }
            return;
        }
        self.save();
        self.syncing.set(true);
        self.spinner.set_spinning(true);
        self.spinner.set_visible(true);
        self.content.set_sensitive(false);
        self.set_status("Syncing…");
        let paths = self.paths.clone();
        let config = self.config.borrow().clone();
        let app = self.weak();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || sync::sync(&paths, &config))
                .await
                .unwrap_or_else(|_| sync::SyncResult {
                    error: Some("the sync thread crashed".into()),
                    ..Default::default()
                });
            let Some(app) = app.upgrade() else { return };
            app.syncing.set(false);
            app.spinner.set_spinning(false);
            app.spinner.set_visible(false);
            app.content.set_sensitive(true);
            if let Err(e) = app.load_from_disk() {
                app.set_status(&format!("Could not read the merged file: {e:#}"));
            }
            app.set_status(&result.status_line());
            if interactive || !result.ok() {
                settings::show_sync_result(&app, "Sync", &result);
            }
        });
    }

    fn pick_file(
        &self,
        title: &str,
        save: Option<&str>,
        done: impl FnOnce(&App, PathBuf) + 'static,
    ) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("iCalendar files"));
        filter.add_pattern("*.ics");
        filter.add_pattern("*.ical");
        filter.add_mime_type("text/calendar");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(title)
            .modal(true)
            .filters(&filters)
            .build();
        let app = self.weak();
        let cb = move |res: Result<gio::File, glib::Error>| {
            if let (Ok(file), Some(app)) = (res, app.upgrade())
                && let Some(path) = file.path()
            {
                done(&app, path);
            }
        };
        match save {
            Some(name) => {
                dialog.set_initial_name(Some(name));
                dialog.save(Some(&self.window), gio::Cancellable::NONE, cb);
            }
            None => dialog.open(Some(&self.window), gio::Cancellable::NONE, cb),
        }
    }

    fn import_dialog(&self) {
        self.pick_file("Import iCalendar", None, |app, path| {
            match app.import_file(&path) {
                Ok(msg) => app.set_status(&msg),
                Err(e) => widgets::error(&app.window, "Import failed", &format!("{e:#}")),
            }
        });
    }

    /// Import a file: opencal files are merged as they are; other .ics files go to a new
    /// calendar named after the file.
    pub fn import_file(&self, path: &Path) -> anyhow::Result<String> {
        let text = std::fs::read_to_string(path)?;
        let parsed = ics::parse(&text)?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Imported".into());
        let mut count = 0;
        let mut err = None;
        self.change(|store| {
            let target = if parsed.calendars.is_empty() {
                let n = store.calendars.len();
                let cal = Calendar::new(
                    &name,
                    opencal::model::CALENDAR_COLORS[n % opencal::model::CALENDAR_COLORS.len()],
                );
                let uid = cal.uid.clone();
                store.add_calendar(cal);
                uid
            } else {
                store
                    .live_calendars()
                    .first()
                    .map(|c| c.uid.clone())
                    .unwrap_or_default()
            };
            match ics::import(store, &text, &target) {
                Ok(n) => count = n,
                Err(e) => err = Some(e),
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
        Ok(format!("Imported {count} entries from {}", path.display()))
    }

    fn export_dialog(&self) {
        self.pick_file("Export iCalendar", Some("calendar.ics"), |app, path| {
            let text = ics::write_export(&app.store.borrow());
            match opencal::write_atomic(&path, text.as_bytes()) {
                Ok(()) => app.set_status(&format!("Exported to {}", path.display())),
                Err(e) => widgets::error(&app.window, "Export failed", &format!("{e:#}")),
            }
        });
    }

    fn merge_dialog(&self) {
        self.pick_file("Merge with Calendar File", None, |app, path| {
            app.save();
            let db = app.db.borrow().clone();
            match sync::merge_file(&db, &path) {
                Ok(report) => {
                    let _ = app.load_from_disk();
                    app.set_status(&format!("Merged {}: {}", path.display(), report.summary()));
                    let result = sync::SyncResult {
                        report,
                        remote_existed: true,
                        ..Default::default()
                    };
                    settings::show_sync_result(&app.rc(), "Merge", &result);
                }
                Err(e) => widgets::error(&app.window, "Merge failed", &format!("{e:#}")),
            }
        });
    }

    fn about(&self) {
        let about = gtk::AboutDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .program_name("opencal")
            .version(opencal::VERSION)
            .comments("A local calendar and task manager.\nYour calendar is a plain iCalendar file; sync it with your own commands.")
            .website("https://github.com/TheElevatedOne/opencal")
            .license_type(gtk::License::Gpl30)
            .logo_icon_name("org.opencal.OpenCal")
            .build();
        about.add_css_class("opencal");
        about.set_decorated(false);
        about.present();
    }

    fn shortcuts(&self) {
        let w = widgets::dialog_window(&self.window, "Keyboard Shortcuts");
        let grid = gtk::Grid::builder()
            .row_spacing(6)
            .column_spacing(24)
            .build();
        grid.add_css_class("dialog-content");
        let rows = [
            ("Ctrl+N", "New event"),
            ("Ctrl+T", "New task"),
            ("T / Home", "Go to today"),
            ("Page Up / Alt+Left", "Previous day, week or month"),
            ("Page Down / Alt+Right", "Next day, week or month"),
            ("Ctrl+1 … Ctrl+5", "Day, Week, Month, Agenda, Tasks"),
            ("F5 / Ctrl+R", "Sync"),
            ("Ctrl+I / Ctrl+E", "Import / export iCalendar"),
            ("F9", "Show or hide the sidebar"),
            ("F10", "Menu"),
            ("Ctrl+Q", "Quit"),
            ("Double-click", "New event at that day or time"),
            ("Drag (day/week)", "Create, move or resize events"),
        ];
        for (i, (k, d)) in rows.iter().enumerate() {
            let key = widgets::label(k);
            key.add_css_class("monospace");
            grid.attach(&key, 0, i as i32, 1, 1);
            grid.attach(&widgets::label(d), 1, i as i32, 1, 1);
        }
        w.set_child(Some(&widgets::titled(&w, &grid)));
        w.present();
    }

    pub fn present(&self) {
        self.window.present();
        if self.config.borrow().sync.on_start && self.sync_configured() {
            self.sync_now(false);
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.db.borrow().clone()
    }

    /// Fill the store with example entries around today (for screenshots).
    pub fn load_demo(&self) {
        let t = today();
        let ws = self.week_start(t);
        let at = |d: NaiveDate, h: u32, m: u32| When::DateTime(d.and_hms_opt(h, m, 0).unwrap());
        self.change(|s| {
            let personal = opencal::model::DEFAULT_CALENDAR.to_string();
            let mut work = Calendar::new("Work", "#2ec27e");
            work.uid = "demo-work".into();
            let mut family = Calendar::new("Family", "#e66100");
            family.uid = "demo-family".into();
            s.add_calendar(work);
            s.add_calendar(family);
            let mut standup =
                Item::new_event("demo-work", "Stand-up", at(ws, 9, 30), at(ws, 9, 45));
            standup.rrule = Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".into());
            s.add_item(standup);
            let mut review = Item::new_event(
                "demo-work",
                "Design review",
                at(ws + CDuration::days(1), 13, 0),
                at(ws + CDuration::days(1), 14, 30),
            );
            review.location = "Room 4".into();
            s.add_item(review);
            s.add_item(Item::new_event(
                "demo-work",
                "Release planning",
                at(ws + CDuration::days(3), 10, 0),
                at(ws + CDuration::days(3), 12, 0),
            ));
            s.add_item(Item::new_event(
                &personal,
                "Dentist",
                at(ws + CDuration::days(2), 16, 0),
                at(ws + CDuration::days(2), 17, 0),
            ));
            let mut gym = Item::new_event(&personal, "Gym", at(ws, 18, 0), at(ws, 19, 30));
            gym.rrule = Some("FREQ=WEEKLY;BYDAY=MO,TH".into());
            s.add_item(gym);
            s.add_item(Item::new_event(
                "demo-family",
                "Grandma's birthday",
                When::Date(ws + CDuration::days(5)),
                When::Date(ws + CDuration::days(6)),
            ));
            s.add_item(Item::new_event(
                "demo-family",
                "Trip to the mountains",
                When::Date(ws + CDuration::days(12)),
                When::Date(ws + CDuration::days(15)),
            ));
            s.add_item(Item::new_task(
                &personal,
                "Renew passport",
                Some(When::Date(t + CDuration::days(2))),
            ));
            s.add_item(Item::new_task(
                "demo-work",
                "Send the quarterly report",
                Some(at(t, 17, 0)),
            ));
            let mut done = Item::new_task(&personal, "Buy a new bike lock", None);
            done.set_completed(true);
            s.add_item(done);
            s.add_item(Item::new_task("demo-family", "Book the cabin", None));
        });
    }
}
