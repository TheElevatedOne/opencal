use chrono::{Duration, NaiveDate};

use crate::merge::{ChangeKind, merge};
use crate::model::*;
use crate::{ics, recur};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn at(y: i32, m: u32, day: u32, h: u32) -> When {
    When::DateTime(d(y, m, day).and_hms_opt(h, 0, 0).unwrap())
}

fn sample() -> Store {
    let mut s = Store::new();
    let mut e = Item::new_event(
        DEFAULT_CALENDAR,
        "Dentist; bring card, ok\\yes",
        at(2026, 11, 16, 10),
        at(2026, 11, 16, 11),
    );
    e.description = "Line one\nLine two with a very long text that will certainly need folding because it is long ✓ ✓ ✓ ✓".into();
    e.extra.push("BEGIN:VALARM".into());
    e.extra.push("ACTION:DISPLAY".into());
    e.extra.push("TRIGGER:-PT15M".into());
    e.extra.push("END:VALARM".into());
    s.add_item(e);
    let mut t = Item::new_task(
        DEFAULT_CALENDAR,
        "Buy milk",
        Some(When::Date(d(2026, 11, 17))),
    );
    t.set_completed(true);
    s.add_item(t);
    let mut w = Item::new_event(
        DEFAULT_CALENDAR,
        "Gym",
        at(2026, 11, 2, 18),
        at(2026, 11, 2, 19),
    );
    w.rrule = Some("FREQ=WEEKLY;BYDAY=MO,TH".into());
    w.exdates.push(at(2026, 11, 9, 18));
    s.add_item(w);
    s
}

#[test]
fn roundtrip() {
    let s = sample();
    let text = ics::write(&s);
    assert!(text.lines().all(|l| l.len() <= 76), "lines are folded");
    let back = ics::parse(&text).unwrap();
    assert_eq!(s, back);
    assert_eq!(text, ics::write(&back), "writing is stable");
}

#[test]
fn tombstones_roundtrip_and_export() {
    let mut s = sample();
    let uid = s
        .items
        .values()
        .find(|i| i.summary == "Buy milk")
        .unwrap()
        .uid
        .clone();
    s.delete_item(&uid);
    let back = ics::parse(&ics::write(&s)).unwrap();
    assert!(back.items[&uid].is_deleted());
    assert!(!ics::write_export(&s).contains(&uid));
}

#[test]
fn merge_is_symmetric_and_reports() {
    let base = sample();
    let mut a = base.clone();
    let mut b = base.clone();
    let task = a
        .items
        .values()
        .find(|i| i.kind == Kind::Task)
        .unwrap()
        .uid
        .clone();
    let dentist = a
        .items
        .values()
        .find(|i| i.summary.starts_with("Dentist"))
        .unwrap()
        .uid
        .clone();

    // A renames the task (later), B reopens it and deletes the dentist.
    let later = now() + Duration::seconds(10);
    {
        let t = a.items.get_mut(&task).unwrap();
        t.summary = "Buy oat milk".into();
        t.modified = later;
    }
    {
        let t = b.items.get_mut(&task).unwrap();
        t.completed = None;
        t.completion_modified = Some(later + Duration::seconds(5));
    }
    b.delete_item(&dentist);
    b.add_item(Item::new_task(DEFAULT_CALENDAR, "New on B", None));

    let (m1, r1) = merge(&a, &b);
    let (m2, _) = merge(&b, &a);
    assert_eq!(m1, m2);
    let t = &m1.items[&task];
    assert_eq!(t.summary, "Buy oat milk");
    assert!(!t.is_completed(), "completion merged separately");
    assert!(m1.items[&dentist].is_deleted());
    let kinds: Vec<_> = r1.incoming.iter().map(|c| c.kind).collect();
    assert!(kinds.contains(&ChangeKind::Deleted));
    assert!(kinds.contains(&ChangeKind::Added));
    assert!(kinds.contains(&ChangeKind::Reopened));
    assert!(
        r1.outgoing
            .iter()
            .any(|c| c.kind == ChangeKind::Changed && c.uid == task)
    );
}

#[test]
fn edit_after_delete_wins() {
    let base = sample();
    let uid = base
        .items
        .values()
        .find(|i| i.summary == "Gym")
        .unwrap()
        .uid
        .clone();
    let mut a = base.clone();
    let mut b = base.clone();
    a.delete_item(&uid);
    let g = b.items.get_mut(&uid).unwrap();
    g.summary = "Gym (moved)".into();
    g.modified = a.items[&uid].modified + Duration::seconds(1);
    let (m, _) = merge(&a, &b);
    assert!(!m.items[&uid].is_deleted());
}

#[test]
fn two_fresh_files_share_the_default_calendar() {
    let (m, r) = merge(&Store::new(), &Store::new());
    assert_eq!(m.live_calendars().len(), 1);
    assert!(r.is_empty());
}

#[test]
fn recurrence() {
    let s = sample();
    let occ = recur::occurrences(&s, d(2026, 11, 1), d(2026, 12, 1), &Default::default());
    let gym: Vec<_> = occ
        .iter()
        .filter(|o| o.summary == "Gym")
        .map(|o| o.start.date())
        .collect();
    // Mondays and Thursdays from 2 Nov, without 9 Nov.
    assert_eq!(gym.first(), Some(&d(2026, 11, 2)));
    assert!(!gym.contains(&d(2026, 11, 9)));
    assert!(gym.contains(&d(2026, 11, 12)));
    assert_eq!(gym.len(), 8);
    assert!(occ.iter().any(|o| o.summary == "Buy milk" && o.all_day));
}

#[test]
fn import_foreign_ics() {
    let text = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/Prague\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:x1\r\nDTSTAMP:20260101T000000Z\r\nDTSTART;TZID=Europe/Prague:20260310T090000\r\n\
DURATION:PT90M\r\nSUMMARY:Meet\r\n ing\r\nATTENDEE;CN=\"A: B\":mailto:a@b\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:x2\r\nDTSTART;VALUE=DATE:20260311\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let mut s = Store::new();
    let n = ics::import(&mut s, text, DEFAULT_CALENDAR).unwrap();
    assert_eq!(n, 2);
    let m = &s.items["x1"];
    assert_eq!(m.summary, "Meeting");
    assert_eq!(m.calendar, DEFAULT_CALENDAR);
    assert_eq!(
        m.effective_end().unwrap() - m.start.unwrap().naive(),
        Duration::minutes(90)
    );
    assert!(m.extra.iter().any(|l| l.starts_with("ATTENDEE")));
    assert_eq!(
        s.items["x2"].effective_end().unwrap().date(),
        d(2026, 3, 12)
    );
}

#[test]
fn duration_parse() {
    assert_eq!(
        ics::parse_duration("P1W2DT3H4M5S"),
        Some(Duration::seconds(9 * 86400 + 3 * 3600 + 4 * 60 + 5))
    );
    assert_eq!(ics::parse_duration("-PT15M"), Some(Duration::minutes(-15)));
}

/// Two profiles syncing through a folder with shell commands, like a user would set it up.
#[test]
fn sync_through_a_folder() {
    use crate::{Config, Paths, sync};
    let base = std::env::temp_dir().join(format!("opencal-sync-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let remote = base.join("remote");
    std::fs::create_dir_all(&remote).unwrap();
    let setup = |name: &str| {
        let paths = Paths::new(Some(&base.join(name)));
        let mut config = Config::default();
        let r = remote.display();
        config.sync.pull = format!("[ ! -e {r}/c.ics ] || cp {r}/c.ics \"$CAL_DB_PULL\"");
        config.sync.push = format!("cp \"$CAL_DB\" {r}/c.ics && cat \"$CAL_SYNC_REPORT\" > {r}/last-report");
        (paths, config)
    };
    let (pa, ca) = setup("a");
    let (pb, cb) = setup("b");

    // A creates a task and syncs first: no remote yet, so it pushes.
    let db_a = pa.db(&ca);
    let mut a = sync::load_store(&db_a).unwrap();
    let task = Item::new_task(DEFAULT_CALENDAR, "From A", None);
    let uid = task.uid.clone();
    a.add_item(task);
    crate::save_store(&db_a, &a).unwrap();
    let r = sync::sync(&pa, &ca);
    assert!(r.ok(), "{:?}", r.error);
    assert!(!r.remote_existed && r.pushed);

    // B gets it.
    let r = sync::sync(&pb, &cb);
    assert!(r.ok(), "{:?}", r.error);
    assert!(r.report.incoming.iter().any(|c| c.uid == uid));
    let db_b = pb.db(&cb);
    assert!(!sync::load_store(&db_b).unwrap().items[&uid].is_deleted());

    // B deletes it; A learns about the deletion.
    let mut b = sync::load_store(&db_b).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    b.delete_item(&uid);
    crate::save_store(&db_b, &b).unwrap();
    assert!(sync::sync(&pb, &cb).ok());
    assert!(std::fs::read_to_string(remote.join("last-report")).unwrap().contains("From A"));
    let r = sync::sync(&pa, &ca);
    assert!(r.ok());
    assert!(sync::load_store(&db_a).unwrap().items[&uid].is_deleted());

    // The remote copy vanishes (a broken script would look the same): no push over it.
    std::fs::remove_file(remote.join("c.ics")).unwrap();
    let r = sync::sync(&pa, &ca);
    assert!(!r.ok() && !r.pushed, "must refuse to push");
    assert!(!remote.join("c.ics").exists());

    // A timed-out command is stopped, including what it started.
    let mut slow = ca.clone();
    slow.sync.pull = "sleep 30 & sleep 30".into();
    slow.sync.timeout_seconds = 1;
    let t = std::time::Instant::now();
    let r = sync::sync(&pa, &slow);
    assert!(!r.ok() && t.elapsed() < std::time::Duration::from_secs(10));

    let _ = std::fs::remove_dir_all(&base);
}
