//! Expanding events (and recurring events) into the occurrences shown in a date range.

use std::collections::HashSet;

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use rrule::{RRule, Tz, Unvalidated};
use serde::Serialize;

use crate::model::{Item, Kind, Store, When};

/// One appearance of an event or task in the calendar.
#[derive(Clone, Debug, Serialize)]
pub struct Occurrence {
    pub uid: String,
    pub kind: Kind,
    pub calendar: String,
    pub summary: String,
    pub start: NaiveDateTime,
    /// Exclusive. Tasks get `start` (a point in time).
    pub end: NaiveDateTime,
    pub all_day: bool,
    pub completed: bool,
    pub recurring: bool,
}

impl Occurrence {
    /// Whether it touches the given day.
    pub fn on_day(&self, day: NaiveDate) -> bool {
        let d0 = day.and_time(NaiveTime::MIN);
        let d1 = d0 + Duration::days(1);
        if self.end <= self.start {
            return self.start >= d0 && self.start < d1;
        }
        self.start < d1 && self.end > d0
    }
}

/// Floating times are expanded as if they were UTC: that is plain wall-clock arithmetic, with no
/// daylight-saving shifts, which is what a floating time means.
fn to_tz(dt: NaiveDateTime) -> chrono::DateTime<Tz> {
    Tz::UTC.from_utc_datetime(&dt)
}

/// Start times of a recurring item between `from` and `to`. `None` when the rule can't be used.
pub fn expand(
    item: &Item,
    from: NaiveDateTime,
    to: NaiveDateTime,
    limit: u16,
) -> Option<Vec<NaiveDateTime>> {
    let rule = item.rrule.as_deref()?;
    let start = item.start?.naive();
    // A date-only UNTIL is common in the wild; the rrule crate wants a date-time.
    let rule = normalize_until(rule);
    let parsed: RRule<Unvalidated> = rule.parse().ok()?;
    let mut set = parsed.build(to_tz(start)).ok()?;
    for ex in &item.exdates {
        let ex = match ex {
            When::Date(d) => d.and_time(start.time()),
            When::DateTime(dt) => *dt,
        };
        set = set.exdate(to_tz(ex));
    }
    let set = set.after(to_tz(from)).before(to_tz(to));
    Some(
        set.all(limit)
            .dates
            .into_iter()
            .map(|d| d.naive_utc())
            .collect(),
    )
}

fn normalize_until(rule: &str) -> String {
    rule.split(';')
        .map(|part| match part.split_once('=') {
            Some((k, v)) if k.eq_ignore_ascii_case("UNTIL") => {
                let v = v.trim_end_matches('Z');
                if v.len() == 8 {
                    format!("UNTIL={v}T235959Z")
                } else {
                    format!("UNTIL={v}Z")
                }
            }
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// All occurrences overlapping `[from, to)` (dates), skipping the calendars in `hidden`.
/// Tasks appear on their due date.
pub fn occurrences(
    store: &Store,
    from: NaiveDate,
    to: NaiveDate,
    hidden: &HashSet<String>,
) -> Vec<Occurrence> {
    let from_dt = from.and_time(NaiveTime::MIN);
    let to_dt = to.and_time(NaiveTime::MIN);
    let mut out = Vec::new();
    for item in store.live_items() {
        let calendar = store
            .calendar_of(item)
            .map(|c| c.uid.clone())
            .unwrap_or_default();
        if hidden.contains(&calendar) {
            continue;
        }
        let make = |start: NaiveDateTime, end: NaiveDateTime, all_day: bool| Occurrence {
            uid: item.uid.clone(),
            kind: item.kind,
            calendar: calendar.clone(),
            summary: item.summary.clone(),
            start,
            end,
            all_day,
            completed: item.is_completed(),
            recurring: item.rrule.is_some(),
        };
        match item.kind {
            Kind::Task => {
                if let Some(due) = item.due {
                    let t = due.naive();
                    if t >= from_dt && t < to_dt {
                        out.push(make(t, t, due.is_date()));
                    }
                }
            }
            Kind::Event => {
                let (Some(start), Some(end)) = (item.start, item.effective_end()) else {
                    continue;
                };
                let len = end - start.naive();
                let starts = match item.rrule {
                    Some(_) => expand(item, from_dt - len - Duration::seconds(1), to_dt, 2000)
                        .unwrap_or_else(|| vec![start.naive()]),
                    None => vec![start.naive()],
                };
                for s in starts {
                    let e = s + len;
                    let overlaps = if e <= s {
                        s >= from_dt && s < to_dt
                    } else {
                        s < to_dt && e > from_dt
                    };
                    if overlaps {
                        out.push(make(s, e, start.is_date()));
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| {
        b.all_day
            .cmp(&a.all_day)
            .then(a.start.cmp(&b.start))
            .then(a.summary.cmp(&b.summary))
    });
    out
}
