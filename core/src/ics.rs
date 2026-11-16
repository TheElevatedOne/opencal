//! Reading and writing the calendar file (iCalendar, RFC 5545).
//!
//! The file is a normal `.ics` that other calendar programs can import. opencal adds a few
//! `X-` extensions so that two copies of the file can be merged:
//!
//! * `BEGIN:X-OPENCAL-CALENDAR` components hold the calendars (UID, NAME, COLOR, LAST-MODIFIED).
//! * `X-OPENCAL-CALENDAR:<uid>` on events and tasks names their calendar.
//! * `X-OPENCAL-DELETED:<utc>` marks a deleted entry (a tombstone, also `STATUS:CANCELLED`).
//! * `X-OPENCAL-COMPLETION-MODIFIED:<utc>` is when a task was last completed or reopened.
//!
//! Entries are written sorted by UID with one property per line, so a text diff of two versions
//! (e.g. in git) shows exactly what changed.

use anyhow::{Result, bail};
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

use crate::model::{Calendar, Item, Kind, Store, When};

pub const PRODID: &str = concat!("-//opencal//opencal ", env!("CARGO_PKG_VERSION"), "//EN");

// ---------------------------------------------------------------------------------------------
// Content lines
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Line {
    name: String,
    params: Vec<(String, String)>,
    value: String,
    raw: String,
}

impl Line {
    fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Undo line folding (a line starting with a space or tab continues the previous one).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = lines.last_mut()
        {
            last.push_str(rest);
            continue;
        }
        if !line.is_empty() {
            lines.push(line.to_string());
        }
    }
    lines
}

fn parse_line(raw: &str) -> Option<Line> {
    // The value starts after the first colon that is not inside a quoted parameter value.
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in raw.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let head = &raw[..colon];
    let value = raw[colon + 1..].to_string();
    let mut parts = Vec::new();
    let mut cur = String::new();
    in_quotes = false;
    for c in head.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            ';' if !in_quotes => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    let name = parts.remove(0).to_ascii_uppercase();
    let params = parts
        .into_iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.to_ascii_uppercase(), v.trim_matches('"').to_string()))
        })
        .collect();
    Some(Line {
        name,
        params,
        value,
        raw: raw.to_string(),
    })
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

pub fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Fold a content line to at most 75 octets per line, never splitting a UTF-8 character.
fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if width + len > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += len;
    }
    out.push_str("\r\n");
}

// ---------------------------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------------------------

fn parse_naive(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y%m%d").ok()
}

/// A DTSTART/DTEND/DUE/EXDATE value as local floating time.
fn parse_when(value: &str, line: &Line) -> Option<When> {
    let value = value.trim();
    if line.param("VALUE") == Some("DATE") || value.len() == 8 {
        return parse_date(value).map(When::Date);
    }
    if let Some(utc) = value.strip_suffix('Z') {
        let dt = Utc.from_utc_datetime(&parse_naive(utc)?);
        return Some(When::DateTime(dt.with_timezone(&Local).naive_local()));
    }
    let naive = parse_naive(value)?;
    if let Some(tz) = line
        .param("TZID")
        .and_then(|t| t.trim_start_matches('/').parse::<chrono_tz::Tz>().ok())
        && let Some(dt) = tz.from_local_datetime(&naive).earliest()
    {
        return Some(When::DateTime(dt.with_timezone(&Local).naive_local()));
    }
    Some(When::DateTime(naive))
}

/// A UTC timestamp (CREATED, LAST-MODIFIED, COMPLETED, …).
fn parse_utc(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    let v = value.strip_suffix('Z').unwrap_or(value);
    if let Some(n) = parse_naive(v) {
        return Some(Utc.from_utc_datetime(&n));
    }
    parse_date(v).map(|d| Utc.from_utc_datetime(&d.and_time(chrono::NaiveTime::MIN)))
}

/// An iCalendar DURATION such as `PT1H30M`, `P1D` or `-P2W`.
pub fn parse_duration(value: &str) -> Option<Duration> {
    let mut s = value.trim();
    let mut sign = 1;
    if let Some(r) = s.strip_prefix('-') {
        sign = -1;
        s = r;
    } else if let Some(r) = s.strip_prefix('+') {
        s = r;
    }
    let s = s.strip_prefix('P')?;
    let mut total = 0i64;
    let mut num = String::new();
    let mut in_time = false;
    for c in s.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            _ => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += n * match (c, in_time) {
                    ('W', _) => 7 * 86400,
                    ('D', _) => 86400,
                    ('H', true) => 3600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    Some(Duration::seconds(sign * total))
}

pub fn format_utc(t: &DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

fn format_when(name: &str, w: &When) -> String {
    match w {
        When::Date(d) => format!("{name};VALUE=DATE:{}", d.format("%Y%m%d")),
        When::DateTime(dt) => format!("{name}:{}", dt.format("%Y%m%dT%H%M%S")),
    }
}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

/// Parse a calendar file. Events and tasks without `X-OPENCAL-CALENDAR` get an empty calendar UID
/// (see [`import`] to assign one). Components other than events, tasks and opencal calendars
/// (time zones, journals, free/busy) are skipped.
pub fn parse(text: &str) -> Result<Store> {
    let lines: Vec<Line> = unfold(text).iter().filter_map(|l| parse_line(l)).collect();
    if !lines
        .iter()
        .any(|l| l.name == "BEGIN" && l.value.eq_ignore_ascii_case("VCALENDAR"))
    {
        bail!("not an iCalendar file (no BEGIN:VCALENDAR)");
    }
    let mut store = Store::default();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if line.name == "BEGIN" {
            let comp = line.value.to_ascii_uppercase();
            let end = find_end(&lines, i);
            match comp.as_str() {
                "VEVENT" | "VTODO" => {
                    let kind = if comp == "VEVENT" {
                        Kind::Event
                    } else {
                        Kind::Task
                    };
                    if let Some(item) = parse_item(kind, &lines[i + 1..end]) {
                        insert_newer(&mut store, item);
                    }
                    i = end + 1;
                    continue;
                }
                "X-OPENCAL-CALENDAR" => {
                    if let Some(cal) = parse_calendar(&lines[i + 1..end]) {
                        store.calendars.insert(cal.uid.clone(), cal);
                    }
                    i = end + 1;
                    continue;
                }
                "VCALENDAR" => {}
                _ => {
                    i = end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    Ok(store)
}

/// Index of the END line matching the BEGIN at `start` (or the last line).
fn find_end(lines: &[Line], start: usize) -> usize {
    let mut depth = 0;
    for (j, l) in lines.iter().enumerate().skip(start) {
        if l.name == "BEGIN" {
            depth += 1;
        } else if l.name == "END" {
            depth -= 1;
            if depth == 0 {
                return j;
            }
        }
    }
    lines.len() - 1
}

/// Recurrence overrides share the UID of their series; keep the series (no RECURRENCE-ID).
fn insert_newer(store: &mut Store, item: Item) {
    store.items.insert(item.uid.clone(), item);
}

fn parse_calendar(lines: &[Line]) -> Option<Calendar> {
    let mut cal = Calendar {
        uid: String::new(),
        name: String::new(),
        color: "#3584e4".into(),
        modified: DateTime::<Utc>::UNIX_EPOCH,
        deleted: None,
    };
    for l in lines {
        match l.name.as_str() {
            "UID" => cal.uid = l.value.clone(),
            "NAME" => cal.name = unescape(&l.value),
            "COLOR" => cal.color = l.value.clone(),
            "LAST-MODIFIED" => cal.modified = parse_utc(&l.value).unwrap_or(cal.modified),
            "X-OPENCAL-DELETED" => cal.deleted = parse_utc(&l.value),
            _ => {}
        }
    }
    (!cal.uid.is_empty()).then_some(cal)
}

fn parse_item(kind: Kind, lines: &[Line]) -> Option<Item> {
    let mut item = Item::new_task("", "", None);
    item.kind = kind;
    item.uid = String::new();
    let epoch = DateTime::<Utc>::UNIX_EPOCH;
    item.created = epoch;
    item.modified = epoch;
    let mut dtstamp = None;
    let mut modified = None;
    let mut duration = None;
    let mut status = None;
    let mut recurrence_id = false;
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        if l.name == "BEGIN" {
            // Nested component (VALARM, …): keep it verbatim.
            let end = find_end(lines, i);
            item.extra
                .extend(lines[i..=end].iter().map(|l| l.raw.clone()));
            i = end + 1;
            continue;
        }
        match l.name.as_str() {
            "UID" => item.uid = l.value.clone(),
            "SUMMARY" => item.summary = unescape(&l.value),
            "DESCRIPTION" => item.description = unescape(&l.value),
            "LOCATION" => item.location = unescape(&l.value),
            "DTSTART" => item.start = parse_when(&l.value, l),
            "DTEND" => item.end = parse_when(&l.value, l),
            "DUE" => item.due = parse_when(&l.value, l),
            "DURATION" => duration = parse_duration(&l.value),
            "RRULE" => item.rrule = Some(l.value.clone()),
            "EXDATE" => item
                .exdates
                .extend(l.value.split(',').filter_map(|v| parse_when(v, l))),
            "PRIORITY" => item.priority = l.value.trim().parse().unwrap_or(0),
            "COMPLETED" => item.completed = parse_utc(&l.value),
            "CREATED" => item.created = parse_utc(&l.value).unwrap_or(epoch),
            "LAST-MODIFIED" => modified = parse_utc(&l.value),
            "DTSTAMP" => dtstamp = parse_utc(&l.value),
            "SEQUENCE" => item.sequence = l.value.trim().parse().unwrap_or(0),
            "X-OPENCAL-CALENDAR" => item.calendar = l.value.clone(),
            "X-OPENCAL-DELETED" => item.deleted = parse_utc(&l.value),
            "X-OPENCAL-COMPLETION-MODIFIED" => item.completion_modified = parse_utc(&l.value),
            "RECURRENCE-ID" => recurrence_id = true,
            "PERCENT-COMPLETE" => {}
            "STATUS" if kind == Kind::Task => status = Some(l.value.to_ascii_uppercase()),
            "STATUS" if l.value.eq_ignore_ascii_case("CANCELLED") => {
                status = Some("CANCELLED".into())
            }
            _ => item.extra.push(l.raw.clone()),
        }
        i += 1;
    }
    if recurrence_id {
        // Modified single occurrences of a series are not supported; the series itself is kept.
        return None;
    }
    if item.uid.is_empty() {
        item.uid = crate::model::new_uid();
    }
    item.modified = modified.or(dtstamp).unwrap_or(item.created);
    if item.created == epoch {
        item.created = item.modified;
    }
    if let (None, Some(d), Some(s)) = (item.end, duration, item.start) {
        item.end = Some(match s {
            When::Date(date) => When::Date(date + Duration::days(d.num_days().max(1))),
            When::DateTime(dt) => When::DateTime(dt + d),
        });
    }
    match (kind, status.as_deref()) {
        (Kind::Task, Some("COMPLETED")) if item.completed.is_none() => {
            item.completed = Some(item.modified)
        }
        (Kind::Task, Some(s)) if s != "COMPLETED" => item.completed = None,
        (Kind::Event, Some("CANCELLED")) if item.deleted.is_none() => {
            item.extra.push("STATUS:CANCELLED".into())
        }
        _ => {}
    }
    if kind == Kind::Event && item.start.is_none() {
        return None;
    }
    Some(item)
}

// ---------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------

/// The whole store, tombstones included (this is the format of the calendar file).
pub fn write(store: &Store) -> String {
    write_impl(store, true)
}

/// Only live entries, for handing to other calendar programs.
pub fn write_export(store: &Store) -> String {
    write_impl(store, false)
}

fn write_impl(store: &Store, tombstones: bool) -> String {
    let mut out = String::new();
    let mut put = |line: String| fold(&line, &mut out);
    put("BEGIN:VCALENDAR".into());
    put("VERSION:2.0".into());
    put(format!("PRODID:{PRODID}"));
    put("CALSCALE:GREGORIAN".into());
    for cal in store.calendars.values() {
        if cal.is_deleted() && !tombstones {
            continue;
        }
        put("BEGIN:X-OPENCAL-CALENDAR".into());
        put(format!("UID:{}", cal.uid));
        put(format!("NAME:{}", escape(&cal.name)));
        put(format!("COLOR:{}", cal.color));
        put(format!("LAST-MODIFIED:{}", format_utc(&cal.modified)));
        if let Some(d) = cal.deleted {
            put(format!("X-OPENCAL-DELETED:{}", format_utc(&d)));
        }
        put("END:X-OPENCAL-CALENDAR".into());
    }
    for item in store.items.values() {
        if item.is_deleted() && !tombstones {
            continue;
        }
        for line in item_lines(item) {
            put(line);
        }
    }
    put("END:VCALENDAR".into());
    out
}

fn item_lines(item: &Item) -> Vec<String> {
    let comp = match item.kind {
        Kind::Event => "VEVENT",
        Kind::Task => "VTODO",
    };
    let mut l = vec![format!("BEGIN:{comp}"), format!("UID:{}", item.uid)];
    l.push(format!("DTSTAMP:{}", format_utc(&item.modified)));
    l.push(format!("CREATED:{}", format_utc(&item.created)));
    l.push(format!("LAST-MODIFIED:{}", format_utc(&item.modified)));
    l.push(format!("SEQUENCE:{}", item.sequence));
    l.push(format!("X-OPENCAL-CALENDAR:{}", item.calendar));
    if let Some(d) = item.deleted {
        // Tombstone: just enough to stay a valid component.
        l.push(format!("X-OPENCAL-DELETED:{}", format_utc(&d)));
        l.push("STATUS:CANCELLED".into());
        if let Some(s) = &item.start {
            l.push(format_when("DTSTART", s));
        }
        l.push(format!("END:{comp}"));
        return l;
    }
    l.push(format!("SUMMARY:{}", escape(&item.summary)));
    if !item.location.is_empty() {
        l.push(format!("LOCATION:{}", escape(&item.location)));
    }
    if !item.description.is_empty() {
        l.push(format!("DESCRIPTION:{}", escape(&item.description)));
    }
    if let Some(s) = &item.start {
        l.push(format_when("DTSTART", s));
    }
    if item.kind == Kind::Event
        && let Some(e) = &item.end
    {
        l.push(format_when("DTEND", e));
    }
    if let Some(d) = &item.due {
        l.push(format_when("DUE", d));
    }
    if let Some(r) = &item.rrule {
        l.push(format!("RRULE:{r}"));
    }
    for ex in &item.exdates {
        l.push(format_when("EXDATE", ex));
    }
    if item.priority > 0 {
        l.push(format!("PRIORITY:{}", item.priority));
    }
    if item.kind == Kind::Task {
        match &item.completed {
            Some(c) => {
                l.push("STATUS:COMPLETED".into());
                l.push(format!("COMPLETED:{}", format_utc(c)));
                l.push("PERCENT-COMPLETE:100".into());
            }
            None => l.push("STATUS:NEEDS-ACTION".into()),
        }
        if let Some(c) = &item.completion_modified {
            l.push(format!("X-OPENCAL-COMPLETION-MODIFIED:{}", format_utc(c)));
        }
    }
    l.extend(item.extra.iter().cloned());
    l.push(format!("END:{comp}"));
    l
}

/// Add the events and tasks of another `.ics` file. Entries without an opencal calendar (or with
/// one this file doesn't have) go to `calendar`. Entries that already exist are merged like a
/// sync. Returns the number of new or changed entries.
pub fn import(store: &mut Store, text: &str, calendar: &str) -> Result<usize> {
    let mut other = parse(text)?;
    for item in other.items.values_mut() {
        let known = store.calendars.contains_key(&item.calendar)
            || other.calendars.contains_key(&item.calendar);
        if item.calendar.is_empty() || !known {
            item.calendar = calendar.to_string();
        }
    }
    let (merged, report) = crate::merge::merge(store, &other);
    *store = merged;
    Ok(report.incoming.len())
}
