//! The calendar data: calendars, events and tasks, with the timestamps the merge needs.
//!
//! Nothing is ever removed outright: a deletion turns the entry into a tombstone (`deleted` is
//! set) so that another copy of the file learns about it when the two are merged.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize};

/// UID of the calendar every new file starts with. All devices create it with the same UID and a
/// modification time of 0, so two fresh files merge into one "Personal" calendar instead of two.
pub const DEFAULT_CALENDAR: &str = "opencal-default-calendar";

/// The current time, rounded to whole seconds (the precision iCalendar stores).
pub fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(0)
}

/// A new random UID.
pub fn new_uid() -> String {
    format!("{}@opencal", uuid::Uuid::new_v4())
}

/// A point in time as the user entered it: a whole day or a wall-clock time. Times are
/// "floating" (iCalendar's local time without a time zone), which is what a personal calendar
/// means by "at 10:00". Times imported with a time zone are converted to local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum When {
    Date(NaiveDate),
    DateTime(NaiveDateTime),
}

impl When {
    pub fn date(&self) -> NaiveDate {
        match self {
            When::Date(d) => *d,
            When::DateTime(dt) => dt.date(),
        }
    }

    /// The start of the day for dates.
    pub fn naive(&self) -> NaiveDateTime {
        match self {
            When::Date(d) => d.and_time(NaiveTime::MIN),
            When::DateTime(dt) => *dt,
        }
    }

    pub fn is_date(&self) -> bool {
        matches!(self, When::Date(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Event,
    Task,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calendar {
    #[serde(default = "new_uid")]
    pub uid: String,
    pub name: String,
    /// `#rrggbb`
    pub color: String,
    #[serde(default = "now")]
    pub modified: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<DateTime<Utc>>,
}

impl Calendar {
    pub fn new(name: &str, color: &str) -> Self {
        Calendar {
            uid: new_uid(),
            name: name.to_string(),
            color: color.to_string(),
            modified: now(),
            deleted: None,
        }
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted.is_some()
    }
}

/// An event (VEVENT) or a task (VTODO).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    #[serde(default = "new_uid")]
    pub uid: String,
    pub kind: Kind,
    /// UID of the calendar it belongs to.
    #[serde(default)]
    pub calendar: String,
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: String,
    /// Event start (required for events), optional start date of a task.
    #[serde(default)]
    pub start: Option<When>,
    /// Event end, exclusive (an all-day event on the 5th ends on the 6th).
    #[serde(default)]
    pub end: Option<When>,
    /// Task due date.
    #[serde(default)]
    pub due: Option<When>,
    /// iCalendar RRULE value without the `RRULE:` prefix, e.g. `FREQ=WEEKLY;BYDAY=MO,WE`.
    #[serde(default)]
    pub rrule: Option<String>,
    /// Starts of the occurrences removed from a recurring event.
    #[serde(default)]
    pub exdates: Vec<When>,
    /// 0 = undefined, 1 = highest, 9 = lowest.
    #[serde(default)]
    pub priority: u8,
    /// When the task was completed; `None` while it is open.
    #[serde(default)]
    pub completed: Option<DateTime<Utc>>,
    /// When the task was last completed or reopened. The merge takes completion from the side
    /// that changed it last, independently of other edits.
    #[serde(default)]
    pub completion_modified: Option<DateTime<Utc>>,
    #[serde(default = "now")]
    pub created: DateTime<Utc>,
    #[serde(default = "now")]
    pub modified: DateTime<Utc>,
    #[serde(default)]
    pub sequence: u32,
    #[serde(default)]
    pub deleted: Option<DateTime<Utc>>,
    /// Properties and sub-components opencal does not edit (alarms, attendees, …), kept verbatim
    /// as unfolded content lines so they survive a round trip.
    #[serde(default)]
    pub extra: Vec<String>,
}

impl Item {
    fn blank(kind: Kind, calendar: &str, summary: &str) -> Self {
        let t = now();
        Item {
            uid: new_uid(),
            kind,
            calendar: calendar.to_string(),
            summary: summary.to_string(),
            description: String::new(),
            location: String::new(),
            start: None,
            end: None,
            due: None,
            rrule: None,
            exdates: Vec::new(),
            priority: 0,
            completed: None,
            completion_modified: None,
            created: t,
            modified: t,
            sequence: 0,
            deleted: None,
            extra: Vec::new(),
        }
    }

    pub fn new_event(calendar: &str, summary: &str, start: When, end: When) -> Self {
        let mut item = Item::blank(Kind::Event, calendar, summary);
        item.start = Some(start);
        item.end = Some(end);
        item
    }

    pub fn new_task(calendar: &str, summary: &str, due: Option<When>) -> Self {
        let mut item = Item::blank(Kind::Task, calendar, summary);
        item.due = due;
        item
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted.is_some()
    }

    pub fn is_completed(&self) -> bool {
        self.completed.is_some()
    }

    pub fn is_all_day(&self) -> bool {
        match self.kind {
            Kind::Event => self.start.is_some_and(|s| s.is_date()),
            Kind::Task => self.due.is_some_and(|d| d.is_date()),
        }
    }

    /// The end of an event, filling in iCalendar's defaults when DTEND is missing (one day for
    /// all-day events, zero length otherwise).
    pub fn effective_end(&self) -> Option<NaiveDateTime> {
        let start = self.start?;
        Some(match (self.end, start) {
            (Some(end), _) if end.naive() >= start.naive() => end.naive(),
            (_, When::Date(d)) => (d + Duration::days(1)).and_time(NaiveTime::MIN),
            (_, When::DateTime(dt)) => dt,
        })
    }

    /// Call after editing the content: bumps LAST-MODIFIED and SEQUENCE.
    pub fn touch(&mut self) {
        self.modified = now().max(self.modified);
        self.sequence += 1;
    }

    /// Mark a task done or open again.
    pub fn set_completed(&mut self, done: bool) {
        if done == self.is_completed() {
            return;
        }
        let t = now();
        self.completed = done.then_some(t);
        self.completion_modified = Some(t);
    }

    /// Turn the item into a tombstone.
    pub fn delete(&mut self) {
        let t = now().max(self.modified);
        self.deleted = Some(t);
        self.modified = t;
        self.sequence += 1;
    }
}

/// Everything in one calendar file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    pub calendars: BTreeMap<String, Calendar>,
    pub items: BTreeMap<String, Item>,
}

impl Store {
    /// A new file with the default "Personal" calendar.
    pub fn new() -> Self {
        let mut store = Store::default();
        store.ensure_default_calendar();
        store
    }

    pub fn ensure_default_calendar(&mut self) {
        if !self.calendars.values().any(|c| !c.is_deleted()) {
            let cal = Calendar {
                uid: DEFAULT_CALENDAR.to_string(),
                name: "Personal".to_string(),
                color: "#3584e4".to_string(),
                modified: DateTime::<Utc>::UNIX_EPOCH,
                deleted: None,
            };
            self.calendars.insert(cal.uid.clone(), cal);
        }
    }

    /// Calendars that are not deleted, sorted by name.
    pub fn live_calendars(&self) -> Vec<&Calendar> {
        let mut cals: Vec<_> = self
            .calendars
            .values()
            .filter(|c| !c.is_deleted())
            .collect();
        cals.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.uid.cmp(&b.uid))
        });
        cals
    }

    /// Items that are not deleted.
    pub fn live_items(&self) -> impl Iterator<Item = &Item> {
        self.items.values().filter(|i| !i.is_deleted())
    }

    /// The calendar an item is shown in: its own, or the first live calendar when its calendar
    /// was deleted on another device.
    pub fn calendar_of(&self, item: &Item) -> Option<&Calendar> {
        match self.calendars.get(&item.calendar) {
            Some(c) if !c.is_deleted() => Some(c),
            _ => self.live_calendars().into_iter().next(),
        }
    }

    pub fn add_calendar(&mut self, cal: Calendar) {
        self.calendars.insert(cal.uid.clone(), cal);
    }

    pub fn add_item(&mut self, item: Item) {
        self.items.insert(item.uid.clone(), item);
    }

    /// Delete a calendar and everything in it.
    pub fn delete_calendar(&mut self, uid: &str) {
        if let Some(cal) = self.calendars.get_mut(uid) {
            let t = now().max(cal.modified);
            cal.deleted = Some(t);
            cal.modified = t;
        }
        for item in self.items.values_mut() {
            if item.calendar == uid && !item.is_deleted() {
                item.delete();
            }
        }
    }

    pub fn delete_item(&mut self, uid: &str) -> bool {
        match self.items.get_mut(uid) {
            Some(item) if !item.is_deleted() => {
                item.delete();
                true
            }
            _ => false,
        }
    }

    /// Drop tombstones older than `days`. Copies that have not been synced for longer than that
    /// may bring deleted entries back.
    pub fn prune_tombstones(&mut self, days: i64) {
        if days <= 0 {
            return;
        }
        let cutoff = now() - Duration::days(days);
        self.items
            .retain(|_, i| i.deleted.is_none_or(|d| d > cutoff));
        self.calendars
            .retain(|_, c| c.deleted.is_none_or(|d| d > cutoff));
    }
}

/// Hand-picked colours offered for new calendars.
pub const CALENDAR_COLORS: &[&str] = &[
    "#3584e4", "#2ec27e", "#e5a50a", "#e66100", "#c01c28", "#9141ac", "#986a44", "#5e5c64",
];
