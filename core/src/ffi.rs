//! C ABI of `libopencal.so`. The declarations are in `include/opencal.h`; read that file for the
//! documentation of each function.
//!
//! Conventions: strings are UTF-8 and NUL-terminated; every `char *` returned must be released
//! with `opencal_string_free`; functions returning `int` give 0 on success and -1 on error; on
//! error, `opencal_last_error()` describes it. Structured data is exchanged as JSON.

#![allow(clippy::missing_safety_doc)]

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::ptr;

use anyhow::{Context, Result, anyhow, bail};
use chrono::NaiveDate;

use crate::model::{Calendar, Item, Kind, Store, When};
use crate::{Config, Paths, ics, merge, recur, sync};

/// Opaque handle for C.
pub struct OpencalStore(Store);

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_error(msg: String) {
    let msg = CString::new(msg.replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = msg);
}

/// Run `f`, turning errors and panics into the last error.
fn guard<T>(f: impl FnOnce() -> Result<T>) -> Option<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Some(v),
        Ok(Err(e)) => {
            set_error(format!("{e:#}"));
            None
        }
        Err(_) => {
            set_error("internal error (panic)".into());
            None
        }
    }
}

fn status(r: Option<()>) -> c_int {
    if r.is_some() { 0 } else { -1 }
}

unsafe fn arg<'a>(p: *const c_char, name: &str) -> Result<&'a str> {
    if p.is_null() {
        bail!("{name} is NULL");
    }
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .with_context(|| format!("{name} is not UTF-8"))
}

unsafe fn opt_arg<'a>(p: *const c_char, name: &str) -> Result<Option<&'a str>> {
    if p.is_null() {
        Ok(None)
    } else {
        unsafe { arg(p, name) }.map(Some)
    }
}

fn out_string(s: String) -> Result<*mut c_char> {
    Ok(CString::new(s)
        .map_err(|_| anyhow!("string contains NUL"))?
        .into_raw())
}

fn out_json<T: serde::Serialize>(v: &T) -> Result<*mut c_char> {
    out_string(serde_json::to_string(v)?)
}

unsafe fn store<'a>(p: *const OpencalStore) -> Result<&'a Store> {
    unsafe { p.as_ref() }
        .map(|s| &s.0)
        .ok_or_else(|| anyhow!("store is NULL"))
}

unsafe fn store_mut<'a>(p: *mut OpencalStore) -> Result<&'a mut Store> {
    unsafe { p.as_mut() }
        .map(|s| &mut s.0)
        .ok_or_else(|| anyhow!("store is NULL"))
}

fn boxed(s: Store) -> *mut OpencalStore {
    Box::into_raw(Box::new(OpencalStore(s)))
}

/// "2026-11-16" or "2026-11-16T10:00[:00]".
fn parse_when(s: &str) -> Result<When> {
    let s = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(When::Date(d));
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(When::DateTime(dt));
        }
    }
    bail!("bad date/time {s:?} (expected YYYY-MM-DD or YYYY-MM-DDTHH:MM[:SS])")
}

fn pick_calendar(store: &Store, cal: Option<&str>) -> Result<String> {
    match cal {
        Some(uid) if store.calendars.get(uid).is_some_and(|c| !c.is_deleted()) => {
            Ok(uid.to_string())
        }
        Some(name) => store
            .live_calendars()
            .into_iter()
            .find(|c| c.name == name)
            .map(|c| c.uid.clone())
            .ok_or_else(|| anyhow!("no calendar with UID or name {name:?}")),
        None => store
            .live_calendars()
            .first()
            .map(|c| c.uid.clone())
            .ok_or_else(|| anyhow!("no calendars")),
    }
}

// --- general -----------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn opencal_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn opencal_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_default_path(profile: *const c_char) -> *mut c_char {
    guard(|| {
        let profile = unsafe { opt_arg(profile, "profile") }?;
        let paths = Paths::new(profile.map(Path::new));
        let config = Config::load(&paths)?;
        out_string(paths.db(&config).display().to_string())
    })
    .unwrap_or(ptr::null_mut())
}

// --- store lifetime ----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn opencal_store_new() -> *mut OpencalStore {
    boxed(Store::new())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_open(path: *const c_char) -> *mut OpencalStore {
    guard(|| {
        Ok(boxed(sync::load_store(Path::new(unsafe {
            arg(path, "path")
        }?))?))
    })
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_from_ics(text: *const c_char) -> *mut OpencalStore {
    guard(|| {
        let mut s = ics::parse(unsafe { arg(text, "text") }?)?;
        s.ensure_default_calendar();
        Ok(boxed(s))
    })
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_save(
    store_: *const OpencalStore,
    path: *const c_char,
) -> c_int {
    status(guard(|| {
        crate::save_store(Path::new(unsafe { arg(path, "path") }?), unsafe {
            store(store_)
        }?)
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_to_ics(
    store_: *const OpencalStore,
    include_deleted: c_int,
) -> *mut c_char {
    guard(|| {
        let s = unsafe { store(store_) }?;
        out_string(if include_deleted != 0 {
            ics::write(s)
        } else {
            ics::write_export(s)
        })
    })
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_free(store_: *mut OpencalStore) {
    if !store_.is_null() {
        drop(unsafe { Box::from_raw(store_) });
    }
}

// --- reading -----------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_calendars_json(store_: *const OpencalStore) -> *mut c_char {
    guard(|| out_json(&unsafe { store(store_) }?.live_calendars())).unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_items_json(
    store_: *const OpencalStore,
    include_deleted: c_int,
) -> *mut c_char {
    guard(|| {
        let s = unsafe { store(store_) }?;
        let items: Vec<&Item> = s
            .items
            .values()
            .filter(|i| include_deleted != 0 || !i.is_deleted())
            .collect();
        out_json(&items)
    })
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_get_item_json(
    store_: *const OpencalStore,
    uid: *const c_char,
) -> *mut c_char {
    guard(|| {
        let uid = unsafe { arg(uid, "uid") }?;
        let item = unsafe { store(store_) }?
            .items
            .get(uid)
            .ok_or_else(|| anyhow!("no item {uid:?}"))?;
        out_json(item)
    })
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_occurrences_json(
    store_: *const OpencalStore,
    from: *const c_char,
    to: *const c_char,
) -> *mut c_char {
    guard(|| {
        let from = parse_when(unsafe { arg(from, "from") }?)?.date();
        let to = parse_when(unsafe { arg(to, "to") }?)?.date();
        out_json(&recur::occurrences(
            unsafe { store(store_) }?,
            from,
            to,
            &Default::default(),
        ))
    })
    .unwrap_or(ptr::null_mut())
}

// --- changing ----------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_add_calendar(
    store_: *mut OpencalStore,
    name: *const c_char,
    color: *const c_char,
) -> *mut c_char {
    guard(|| {
        let s = unsafe { store_mut(store_) }?;
        let color = unsafe { opt_arg(color, "color") }?.unwrap_or(
            crate::model::CALENDAR_COLORS[s.calendars.len() % crate::model::CALENDAR_COLORS.len()],
        );
        let cal = Calendar::new(unsafe { arg(name, "name") }?, color);
        let uid = cal.uid.clone();
        s.add_calendar(cal);
        out_string(uid)
    })
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_delete_calendar(
    store_: *mut OpencalStore,
    uid: *const c_char,
) -> c_int {
    status(guard(|| {
        let uid = unsafe { arg(uid, "uid") }?;
        let s = unsafe { store_mut(store_) }?;
        if !s.calendars.contains_key(uid) {
            bail!("no calendar {uid:?}");
        }
        s.delete_calendar(uid);
        s.ensure_default_calendar();
        Ok(())
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_add_event(
    store_: *mut OpencalStore,
    calendar: *const c_char,
    summary: *const c_char,
    start: *const c_char,
    end: *const c_char,
) -> *mut c_char {
    guard(|| {
        let s = unsafe { store_mut(store_) }?;
        let cal = pick_calendar(s, unsafe { opt_arg(calendar, "calendar") }?)?;
        let start = parse_when(unsafe { arg(start, "start") }?)?;
        let end = match unsafe { opt_arg(end, "end") }? {
            Some(e) => parse_when(e)?,
            None => match start {
                When::Date(d) => When::Date(d + chrono::Duration::days(1)),
                When::DateTime(dt) => When::DateTime(dt + chrono::Duration::hours(1)),
            },
        };
        if start.is_date() != end.is_date() {
            bail!("start and end must both be dates or both be date-times");
        }
        let item = Item::new_event(&cal, unsafe { arg(summary, "summary") }?, start, end);
        let uid = item.uid.clone();
        s.add_item(item);
        out_string(uid)
    })
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_add_task(
    store_: *mut OpencalStore,
    calendar: *const c_char,
    summary: *const c_char,
    due: *const c_char,
) -> *mut c_char {
    guard(|| {
        let s = unsafe { store_mut(store_) }?;
        let cal = pick_calendar(s, unsafe { opt_arg(calendar, "calendar") }?)?;
        let due = unsafe { opt_arg(due, "due") }?
            .map(parse_when)
            .transpose()?;
        let item = Item::new_task(&cal, unsafe { arg(summary, "summary") }?, due);
        let uid = item.uid.clone();
        s.add_item(item);
        out_string(uid)
    })
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_put_item_json(
    store_: *mut OpencalStore,
    json: *const c_char,
) -> *mut c_char {
    guard(|| {
        let s = unsafe { store_mut(store_) }?;
        let mut item: Item =
            serde_json::from_str(unsafe { arg(json, "json") }?).context("parsing the item JSON")?;
        if item.kind == Kind::Event && item.start.is_none() {
            bail!("an event needs a start");
        }
        if item.calendar.is_empty() || !s.calendars.contains_key(&item.calendar) {
            item.calendar = pick_calendar(s, None)?;
        }
        if let Some(old) = s.items.get(&item.uid) {
            item.created = old.created;
            item.sequence = old.sequence;
            if item.completed.is_some() != old.completed.is_some() {
                item.completion_modified = Some(crate::model::now());
            }
            item.modified = old.modified;
            item.touch();
        }
        let uid = item.uid.clone();
        s.add_item(item);
        out_string(uid)
    })
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_set_completed(
    store_: *mut OpencalStore,
    uid: *const c_char,
    done: c_int,
) -> c_int {
    status(guard(|| {
        let uid = unsafe { arg(uid, "uid") }?;
        let item = unsafe { store_mut(store_) }?
            .items
            .get_mut(uid)
            .ok_or_else(|| anyhow!("no item {uid:?}"))?;
        item.set_completed(done != 0);
        Ok(())
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_delete_item(
    store_: *mut OpencalStore,
    uid: *const c_char,
) -> c_int {
    status(guard(|| {
        let uid = unsafe { arg(uid, "uid") }?;
        if !unsafe { store_mut(store_) }?.delete_item(uid) {
            bail!("no item {uid:?}");
        }
        Ok(())
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_import_ics(
    store_: *mut OpencalStore,
    text: *const c_char,
    calendar: *const c_char,
) -> c_int {
    guard(|| {
        let s = unsafe { store_mut(store_) }?;
        let cal = pick_calendar(s, unsafe { opt_arg(calendar, "calendar") }?)?;
        Ok(ics::import(s, unsafe { arg(text, "text") }?, &cal)? as c_int)
    })
    .unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_store_merge(
    store_: *mut OpencalStore,
    other: *const OpencalStore,
) -> *mut c_char {
    guard(|| {
        let other = unsafe { store(other) }?.clone();
        let s = unsafe { store_mut(store_) }?;
        let (merged, report) = merge::merge(s, &other);
        *s = merged;
        out_json(&report)
    })
    .unwrap_or(ptr::null_mut())
}

// --- files and sync ----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_merge_file(
    db: *const c_char,
    other: *const c_char,
) -> *mut c_char {
    guard(|| {
        let report = sync::merge_file(
            Path::new(unsafe { arg(db, "db") }?),
            Path::new(unsafe { arg(other, "other") }?),
        )?;
        out_json(&report)
    })
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opencal_sync(profile: *const c_char) -> *mut c_char {
    guard(|| {
        let profile = unsafe { opt_arg(profile, "profile") }?;
        let paths = Paths::new(profile.map(Path::new));
        let config = Config::load(&paths)?;
        out_json(&sync::sync(&paths, &config))
    })
    .unwrap_or(ptr::null_mut())
}
