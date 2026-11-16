//! Merging two copies of the calendar file and describing what changed.
//!
//! Every entry carries its own timestamps, so no common ancestor is needed:
//!
//! * the copy with the later LAST-MODIFIED (then the higher SEQUENCE) wins;
//! * a deletion is a modification at the time of deletion, so it wins over older edits and
//!   loses to newer ones;
//! * a task's completion is merged on its own (`X-OPENCAL-COMPLETION-MODIFIED`), so completing a
//!   task on one device and renaming it on another keeps both changes.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;

use serde::Serialize;

use crate::model::{Calendar, Item, Kind, Store};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Changed,
    Deleted,
    Completed,
    Reopened,
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ChangeKind::Added => "added",
            ChangeKind::Changed => "changed",
            ChangeKind::Deleted => "deleted",
            ChangeKind::Completed => "completed",
            ChangeKind::Reopened => "reopened",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Subject {
    Calendar,
    Event,
    Task,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Change {
    pub kind: ChangeKind,
    pub subject: Subject,
    pub uid: String,
    pub summary: String,
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = match self.kind {
            ChangeKind::Added => '+',
            ChangeKind::Changed => '~',
            ChangeKind::Deleted => '-',
            ChangeKind::Completed => 'x',
            ChangeKind::Reopened => 'o',
        };
        let subject = match self.subject {
            Subject::Calendar => "calendar",
            Subject::Event => "event",
            Subject::Task => "task",
        };
        write!(f, "{sign} {subject} {}: {}", self.kind, self.summary)
    }
}

/// What a merge did.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    /// Changes the other copy brought into this one.
    pub incoming: Vec<Change>,
    /// Changes this copy has that the other one did not (what a push sends).
    pub outgoing: Vec<Change>,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.incoming.is_empty() && self.outgoing.is_empty()
    }

    /// One line per change, for logs, notifications and commit messages.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let mut section = |title: &str, list: &[Change]| {
            s.push_str(&format!("{title} ({}):\n", list.len()));
            for c in list {
                s.push_str(&format!("  {c}\n"));
            }
        };
        section("Incoming", &self.incoming);
        section("Outgoing", &self.outgoing);
        s
    }

    /// e.g. "3 in, 1 out".
    pub fn summary(&self) -> String {
        format!("{} in, {} out", self.incoming.len(), self.outgoing.len())
    }
}

fn order(a: &Item, b: &Item) -> Ordering {
    a.modified
        .cmp(&b.modified)
        .then(a.sequence.cmp(&b.sequence))
        .then(a.is_deleted().cmp(&b.is_deleted()))
        // Fully deterministic, so both devices pick the same winner.
        .then_with(|| crate::ics::write(&single(a)).cmp(&crate::ics::write(&single(b))))
}

fn single(item: &Item) -> Store {
    let mut s = Store::default();
    s.add_item(item.clone());
    s
}

fn merge_item(a: &Item, b: &Item) -> Item {
    let (winner, loser) = if order(a, b) == Ordering::Less {
        (b, a)
    } else {
        (a, b)
    };
    let mut out = winner.clone();
    if !out.is_deleted()
        && out.kind == Kind::Task
        && loser.completion_modified > out.completion_modified
    {
        out.completed = loser.completed;
        out.completion_modified = loser.completion_modified;
    }
    out
}

fn merge_calendar(a: &Calendar, b: &Calendar) -> Calendar {
    let key = |c: &Calendar| {
        (
            c.modified,
            c.deleted.is_some(),
            c.name.clone(),
            c.color.clone(),
        )
    };
    if key(a) >= key(b) {
        a.clone()
    } else {
        b.clone()
    }
}

/// Merge two copies. The result is the same whichever side is `local`.
pub fn merge(local: &Store, remote: &Store) -> (Store, Report) {
    let mut merged = Store::default();
    let uids: BTreeSet<&String> = local
        .calendars
        .keys()
        .chain(remote.calendars.keys())
        .collect();
    for uid in uids {
        let cal = match (local.calendars.get(uid), remote.calendars.get(uid)) {
            (Some(a), Some(b)) => merge_calendar(a, b),
            (Some(a), None) | (None, Some(a)) => a.clone(),
            (None, None) => unreachable!(),
        };
        merged.add_calendar(cal);
    }
    let uids: BTreeSet<&String> = local.items.keys().chain(remote.items.keys()).collect();
    for uid in uids {
        let item = match (local.items.get(uid), remote.items.get(uid)) {
            (Some(a), Some(b)) => merge_item(a, b),
            (Some(a), None) | (None, Some(a)) => a.clone(),
            (None, None) => unreachable!(),
        };
        merged.add_item(item);
    }
    merged.ensure_default_calendar();
    let report = Report {
        incoming: diff(local, &merged),
        outgoing: diff(remote, &merged),
    };
    (merged, report)
}

/// The changes that turn `old` into `new`.
pub fn diff(old: &Store, new: &Store) -> Vec<Change> {
    let mut out = Vec::new();
    for (uid, n) in &new.calendars {
        let o = old.calendars.get(uid);
        let kind = match o {
            _ if n.is_deleted() => match o {
                Some(o) if !o.is_deleted() => Some(ChangeKind::Deleted),
                _ => None,
            },
            None => Some(ChangeKind::Added),
            Some(o) if o.is_deleted() => Some(ChangeKind::Added),
            Some(o) if o.name != n.name || o.color != n.color => Some(ChangeKind::Changed),
            _ => None,
        };
        if let Some(kind) = kind {
            out.push(Change {
                kind,
                subject: Subject::Calendar,
                uid: uid.clone(),
                summary: n.name.clone(),
            });
        }
    }
    for (uid, n) in &new.items {
        let o = old.items.get(uid);
        let subject = match n.kind {
            Kind::Event => Subject::Event,
            Kind::Task => Subject::Task,
        };
        let summary = match o {
            Some(o) if n.is_deleted() => o.summary.clone(),
            _ => n.summary.clone(),
        };
        let mut push = |kind| {
            out.push(Change {
                kind,
                subject,
                uid: uid.clone(),
                summary: summary.clone(),
            })
        };
        match o {
            _ if n.is_deleted() => {
                if o.is_some_and(|o| !o.is_deleted()) {
                    push(ChangeKind::Deleted);
                }
            }
            None => push(ChangeKind::Added),
            Some(o) if o.is_deleted() => push(ChangeKind::Added),
            Some(o) => {
                if o.completed.is_some() != n.completed.is_some() {
                    push(if n.completed.is_some() {
                        ChangeKind::Completed
                    } else {
                        ChangeKind::Reopened
                    });
                }
                let content = |i: &Item| {
                    let mut i = i.clone();
                    i.completed = None;
                    i.completion_modified = None;
                    i
                };
                if content(o) != content(n) {
                    push(ChangeKind::Changed);
                }
            }
        }
    }
    out
}
