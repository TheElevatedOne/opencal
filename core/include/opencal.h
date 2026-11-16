/*
 * opencal.h - C/C++ interface of libopencal, the calendar store behind opencal.
 *
 * Build:  cc app.c $(pkg-config --cflags --libs opencal)
 *
 * Conventions
 *   - Strings are UTF-8 and NUL-terminated.
 *   - Every `char *` a function returns is owned by the caller: release it with
 *     opencal_string_free(). NULL means an error.
 *   - Functions returning int give 0 (or a count) on success and -1 on error.
 *   - After an error, opencal_last_error() describes it (per thread).
 *   - Dates are "YYYY-MM-DD"; times are local wall-clock "YYYY-MM-DDTHH:MM[:SS]".
 *   - Lists and records are JSON. An item looks like:
 *       {"uid":"…","kind":"event"|"task","calendar":"<calendar uid>","summary":"…",
 *        "description":"","location":"","start":"2026-11-16T10:00:00","end":"…",
 *        "due":null,"rrule":"FREQ=WEEKLY;BYDAY=MO","exdates":[],"priority":0,
 *        "completed":null,"completion_modified":null,"created":"…Z","modified":"…Z",
 *        "sequence":0,"deleted":null,"extra":[]}
 *   - Changes stay in memory until opencal_store_save(). Save to the path from
 *     opencal_default_path() to change the calendar the opencal app shows; a
 *     running opencal reloads the file when it changes.
 *   - A store handle must not be used from two threads at the same time.
 */
#ifndef OPENCAL_H
#define OPENCAL_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct OpencalStore opencal_store;

/* Library version, e.g. "0.1.0". Static string, do not free. */
const char *opencal_version(void);
/* Description of the last error on this thread. Static until the next error. */
const char *opencal_last_error(void);
void opencal_string_free(char *s);

/* Path of the calendar file opencal uses. profile: NULL for the normal
 * location, or a profile directory (like `opencal --profile DIR`). */
char *opencal_default_path(const char *profile);

/* --- Stores ---------------------------------------------------------------- */

/* A new calendar with the default "Personal" calendar. */
opencal_store *opencal_store_new(void);
/* Load a calendar file. A missing file gives a new calendar. */
opencal_store *opencal_store_open(const char *path);
/* Parse iCalendar text. */
opencal_store *opencal_store_from_ics(const char *text);
/* Write the calendar file (atomically). */
int opencal_store_save(const opencal_store *store, const char *path);
/* iCalendar text. include_deleted = 1 keeps tombstones (the opencal file
 * format); 0 gives a plain export for other programs. */
char *opencal_store_to_ics(const opencal_store *store, int include_deleted);
void opencal_store_free(opencal_store *store);

/* --- Reading --------------------------------------------------------------- */

/* JSON array of calendars: [{"uid","name","color","modified"}]. */
char *opencal_store_calendars_json(const opencal_store *store);
/* JSON array of items (see above). */
char *opencal_store_items_json(const opencal_store *store, int include_deleted);
/* JSON object of one item, NULL if there is none. */
char *opencal_store_get_item_json(const opencal_store *store, const char *uid);
/* JSON array of occurrences in [from, to) with recurring events expanded and
 * tasks on their due dates: [{"uid","kind","calendar","summary","start","end",
 * "all_day","completed","recurring"}]. */
char *opencal_store_occurrences_json(const opencal_store *store, const char *from, const char *to);

/* --- Changing -------------------------------------------------------------- */

/* Add a calendar; color "#rrggbb" or NULL for the next default colour.
 * Returns its UID. */
char *opencal_store_add_calendar(opencal_store *store, const char *name, const char *color);
/* Delete a calendar and everything in it. */
int opencal_store_delete_calendar(opencal_store *store, const char *uid);
/* Add an event. calendar: UID or name, NULL for the first calendar. start/end
 * are both dates (all-day, end exclusive) or both times; end NULL = 1 hour or
 * 1 day. Returns the UID. */
char *opencal_store_add_event(opencal_store *store, const char *calendar, const char *summary,
                              const char *start, const char *end);
/* Add a task; due may be NULL. Returns the UID. */
char *opencal_store_add_task(opencal_store *store, const char *calendar, const char *summary,
                             const char *due);
/* Add or replace an item from JSON (missing fields get defaults; a missing
 * uid creates a new item). Modification time and sequence are updated.
 * Returns the UID. */
char *opencal_store_put_item_json(opencal_store *store, const char *json);
int opencal_store_set_completed(opencal_store *store, const char *uid, int done);
/* Delete an item (it stays as a tombstone so syncing spreads the deletion). */
int opencal_store_delete_item(opencal_store *store, const char *uid);
/* Import events and tasks from iCalendar text into calendar (UID or name, NULL
 * = first). Returns the number of new or changed items. */
int opencal_store_import_ics(opencal_store *store, const char *text, const char *calendar);

/* --- Merging and sync ------------------------------------------------------ */

/* Merge other into store. Returns the report as JSON:
 * {"incoming":[change…],"outgoing":[change…]} with
 * change = {"kind":"added|changed|deleted|completed|reopened",
 *           "subject":"calendar|event|task","uid","summary"}. */
char *opencal_store_merge(opencal_store *store, const opencal_store *other);
/* Merge the calendar file `other` into the file `db` and save it. Report JSON. */
char *opencal_merge_file(const char *db, const char *other);
/* Run the sync configured in opencal's settings (pull command, merge, push
 * command); blocks until the commands finish. profile as for
 * opencal_default_path(). Returns {"report":…,"remote_existed","pushed",
 * "logs":[{"stage","command","status","output"}],"error":null|"…"}. */
char *opencal_sync(const char *profile);

#ifdef __cplusplus
}
#endif

#endif /* OPENCAL_H */
