use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::ValueEnum;
use rusqlite::{Connection, OptionalExtension, params};

use crate::reminders::{Backend, ReminderData, RemindersList};
use crate::task::{self, Priority, Task};

/// Tasks themselves live in the Reminders app. This local database only
/// keeps what Reminders can't: short IDs and the undo history.
const SCHEMA: &str = "
-- Reminders identifies items by long strings like '3A365278-64BD-...'.
-- This table gives each one a short number for the command line.
-- AUTOINCREMENT guarantees numbers are never reused.
CREATE TABLE IF NOT EXISTS ids (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    reminder_id TEXT    NOT NULL UNIQUE
);

-- Undo stack: each row holds a task as it was *before* a change, as JSON.
-- `before` is NULL when the change created the task (undoing it deletes the task).
CREATE TABLE IF NOT EXISTS history (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    action  TEXT    NOT NULL,
    task_id INTEGER NOT NULL,
    before  TEXT
);
";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum SortKey {
    #[default]
    Id,
    Due,
    Priority,
}

impl SortKey {
    fn sort(self, tasks: &mut [Task]) {
        // `(due.is_none(), due)` puts tasks without a due date last.
        match self {
            SortKey::Id => tasks.sort_by_key(|t| t.id),
            SortKey::Due => {
                tasks.sort_by_key(|t| (t.due.is_none(), t.due, Reverse(t.priority), t.id))
            }
            SortKey::Priority => {
                tasks.sort_by_key(|t| (Reverse(t.priority), t.due.is_none(), t.due, t.id))
            }
        }
    }

    /// The next sort key, for cycling through them in the TUI.
    pub fn next(self) -> SortKey {
        match self {
            SortKey::Id => SortKey::Due,
            SortKey::Due => SortKey::Priority,
            SortKey::Priority => SortKey::Id,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Id => "id",
            SortKey::Due => "due date",
            SortKey::Priority => "priority",
        }
    }
}

/// Which tasks `Store::list` returns, and in what order.
#[derive(Debug, Default)]
pub struct Filter {
    pub tag: Option<String>,
    pub priority: Option<Priority>,
    pub pending_only: bool,
    /// Text that must appear in the title or notes (upper/lower case ignored).
    pub search: Option<String>,
    /// Only tasks due on or before this date.
    pub due_by: Option<NaiveDate>,
    pub sort: SortKey,
}

impl Filter {
    fn matches(&self, task: &Task) -> bool {
        self.tag.as_ref().is_none_or(|tag| task.tags.contains(tag))
            && self.priority.is_none_or(|priority| task.priority == priority)
            && !(self.pending_only && task.done)
            && self.search.as_ref().is_none_or(|text| {
                let text = text.to_lowercase();
                task.title.to_lowercase().contains(&text)
                    || task.notes.to_lowercase().contains(&text)
            })
            && self.due_by.is_none_or(|by| task.due.is_some_and(|due| due <= by))
    }
}

/// Tasks, stored in a reminders list `B`: the real Reminders app by default,
/// or an in-memory fake in tests.
pub struct Store<B = RemindersList> {
    conn: Connection,
    reminders: B,
}

impl Store {
    /// Opens the local state database at `state_path` and the Reminders list `list_name`.
    pub fn open(state_path: &Path, list_name: &str) -> Result<Store> {
        if let Some(dir) = state_path.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("could not create {}", dir.display()))?;
        }
        let conn = Connection::open(state_path)
            .with_context(|| format!("could not open {}", state_path.display()))?;
        Store::with_backend(conn, RemindersList::open(list_name)?)
    }
}

impl<B: Backend> Store<B> {
    /// Builds a store from an open state database and a reminders list.
    pub fn with_backend(conn: Connection, reminders: B) -> Result<Store<B>> {
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn, reminders })
    }

    /// Name of the Reminders list this store works on.
    pub fn list_name(&self) -> String {
        self.reminders.name()
    }

    /// The underlying reminders list, so tests can inspect what was written.
    #[cfg(test)]
    pub fn backend(&self) -> &B {
        &self.reminders
    }

    pub fn list(&self, filter: &Filter) -> Result<Vec<Task>> {
        let mut reminders = self.reminders.all()?;
        // Reminders seen for the first time get the next free numbers, oldest first.
        reminders.sort_by(|a, b| a.created.total_cmp(&b.created));

        let mut known: HashMap<String, i64> = self
            .conn
            .prepare("SELECT reminder_id, id FROM ids")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;

        let mut tasks = Vec::new();
        for reminder in reminders {
            let id = match known.get(&reminder.id) {
                Some(&id) => id,
                None => {
                    let id = self.assign_id(&reminder.id)?;
                    known.insert(reminder.id, id);
                    id
                }
            };
            let task = to_task(id, reminder.list, reminder.data);
            if filter.matches(&task) {
                tasks.push(task);
            }
        }
        filter.sort.sort(&mut tasks);
        Ok(tasks)
    }

    pub fn get(&self, id: i64) -> Result<Option<Task>> {
        let Some(reminder_id) = self.reminder_id(id)? else {
            return Ok(None);
        };
        Ok(self.reminders.get(&reminder_id).map(|r| to_task(id, r.list, r.data)))
    }

    /// Saves `task` as a new reminder in this store's list and returns it
    /// with its assigned ID. (`task.id` and `task.list` are ignored.)
    pub fn add(&mut self, task: &Task) -> Result<Task> {
        let reminder_id = self.reminders.create(&to_data(task), None)?;
        let id = self.assign_id(&reminder_id)?;
        self.record("add", id, None)?;
        self.get(id)?.context("the new reminder disappeared")
    }

    /// Saves all fields of `task` over the stored task with the same ID.
    pub fn update(&mut self, task: &Task) -> Result<Task> {
        let before = self.require(task.id)?;
        self.write(task)?;
        self.record("edit", task.id, Some(&before))?;
        self.get(task.id)?.context("the reminder disappeared")
    }

    pub fn set_done(&mut self, id: i64, done: bool) -> Result<Task> {
        let before = self.require(id)?;
        self.write(&Task { done, ..before.clone() })?;
        self.record(if done { "done" } else { "undone" }, id, Some(&before))?;
        // Read it back: completing a repeating task moves it to its next date
        // (Reminders keeps a completed copy as a separate reminder).
        self.require(id)
    }

    pub fn remove(&mut self, id: i64) -> Result<Task> {
        let before = self.require(id)?;
        if let Some(reminder_id) = self.reminder_id(id)? {
            self.reminders.delete(&reminder_id)?;
        }
        self.record("remove", id, Some(&before))?;
        Ok(before)
    }

    /// Reverts the most recent change. Returns a description of what was
    /// undone, or `None` if there is nothing left to undo.
    pub fn undo(&mut self) -> Result<Option<String>> {
        let last = self
            .conn
            .query_row(
                "SELECT id, action, task_id, before FROM history ORDER BY id DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((history_id, action, task_id, before)) = last else {
            return Ok(None);
        };

        // The reminder as it exists right now, if it still does.
        let current = self
            .reminder_id(task_id)?
            .filter(|reminder_id| self.reminders.get(reminder_id).is_some());

        match before {
            // The change created the task: delete it again.
            None => {
                if let Some(reminder_id) = current {
                    self.reminders.delete(&reminder_id)?;
                }
            }
            // Put the task back as it was. If it was deleted, recreate it in
            // its original list and point its short ID at the new reminder.
            Some(json) => {
                let task: Task = serde_json::from_str(&json).context("corrupt undo history")?;
                match current {
                    Some(reminder_id) => self.reminders.update(&reminder_id, &to_data(&task))?,
                    None => {
                        let list = (!task.list.is_empty()).then_some(task.list.as_str());
                        let reminder_id = self.reminders.create(&to_data(&task), list)?;
                        self.conn.execute(
                            "INSERT OR REPLACE INTO ids (id, reminder_id) VALUES (?1, ?2)",
                            params![task_id, reminder_id],
                        )?;
                    }
                }
            }
        }
        self.conn.execute("DELETE FROM history WHERE id = ?1", [history_id])?;
        Ok(Some(format!("{action} of task {task_id}")))
    }

    fn require(&self, id: i64) -> Result<Task> {
        self.get(id)?.with_context(|| format!("no task with id {id}"))
    }

    fn write(&self, task: &Task) -> Result<()> {
        let reminder_id = self
            .reminder_id(task.id)?
            .with_context(|| format!("no task with id {}", task.id))?;
        self.reminders.update(&reminder_id, &to_data(task))
    }

    fn reminder_id(&self, id: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT reminder_id FROM ids WHERE id = ?1", [id], |row| row.get(0))
            .optional()?)
    }

    fn assign_id(&self, reminder_id: &str) -> Result<i64> {
        self.conn
            .execute("INSERT INTO ids (reminder_id) VALUES (?1)", [reminder_id])?;
        Ok(self.conn.last_insert_rowid())
    }

    fn record(&self, action: &str, task_id: i64, before: Option<&Task>) -> Result<()> {
        let before = before.map(serde_json::to_string).transpose()?;
        self.conn.execute(
            "INSERT INTO history (action, task_id, before) VALUES (?1, ?2, ?3)",
            params![action, task_id, before],
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Converting between tasks and reminders.
//
// EventKit can't read or write Reminders' own tags, so tags are stored as a
// line of hashtags at the end of the reminder's notes: "#work #urgent".
// Any other text in the notes is kept in `Task::notes` and left untouched.
// ---------------------------------------------------------------------------

fn to_task(id: i64, list: String, data: ReminderData) -> Task {
    let (notes, tags) = split_notes(&data.notes);
    Task {
        id,
        title: data.title,
        done: data.done,
        due: data.due,
        priority: Priority::from_eventkit(data.priority),
        tags,
        notes,
        repeat: data.repeat,
        list,
    }
}

fn to_data(task: &Task) -> ReminderData {
    ReminderData {
        title: task.title.clone(),
        notes: join_notes(&task.notes, &task.tags),
        done: task.done,
        // A repeating reminder needs a date to repeat from; start today.
        due: task.due.or(task.repeat.map(|_| task::today())),
        priority: task.priority.to_eventkit(),
        repeat: task.repeat,
    }
}

/// Splits notes into the free text and the tags on its last line, if that
/// line consists only of hashtags.
fn split_notes(notes: &str) -> (String, Vec<String>) {
    let (rest, last_line) = notes.rsplit_once('\n').unwrap_or(("", notes));
    let words: Vec<&str> = last_line.split_whitespace().collect();
    let all_hashtags = words.iter().all(|w| w.len() > 1 && w.starts_with('#'));
    if words.is_empty() || !all_hashtags {
        return (notes.to_string(), Vec::new());
    }
    let tags = words.iter().map(|w| w[1..].to_string()).collect();
    (rest.to_string(), tags)
}

fn join_notes(rest: &str, tags: &[String]) -> String {
    let mut hashtags: Vec<String> = Vec::new();
    for tag in tags {
        // A hashtag can't contain spaces, so "deep work" becomes "#deep-work".
        let tag = tag.trim().trim_start_matches('#').split_whitespace().collect::<Vec<_>>();
        let hashtag = format!("#{}", tag.join("-"));
        if hashtag.len() > 1 && !hashtags.contains(&hashtag) {
            hashtags.push(hashtag);
        }
    }
    let line = hashtags.join(" ");
    match (rest.is_empty(), line.is_empty()) {
        (_, true) => rest.to_string(),
        (true, false) => line,
        (false, false) => format!("{rest}\n{line}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Repeat, RepeatUnit};
    use crate::testing::{self, data};

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|t| t.to_string()).collect()
    }

    fn titles(tasks: &[Task]) -> Vec<&str> {
        tasks.iter().map(|t| t.title.as_str()).collect()
    }

    fn all() -> Filter {
        Filter::default()
    }

    // --- Notes and tags -----------------------------------------------------

    #[test]
    fn split_notes_finds_the_hashtag_line() {
        assert_eq!(split_notes(""), (String::new(), vec![]));
        assert_eq!(split_notes("#work #urgent"), (String::new(), tags(&["work", "urgent"])));
        assert_eq!(
            split_notes("Ask for the blue one\n#shop"),
            ("Ask for the blue one".to_string(), tags(&["shop"]))
        );
    }

    #[test]
    fn split_notes_leaves_ordinary_text_alone() {
        for notes in ["Just text", "Call #2 tomorrow", "Line one\nline two", "#", "# heading"] {
            assert_eq!(split_notes(notes), (notes.to_string(), vec![]), "notes {notes:?}");
        }
    }

    #[test]
    fn join_notes_cleans_and_deduplicates_tags() {
        assert_eq!(join_notes("", &[]), "");
        assert_eq!(join_notes("Text", &[]), "Text");
        assert_eq!(join_notes("", &tags(&["work"])), "#work");
        assert_eq!(join_notes("Text", &tags(&["a", "b"])), "Text\n#a #b");
        assert_eq!(join_notes("", &tags(&["deep work", "#home", " ", "a", "a"])), "#deep-work #home #a");
    }

    #[test]
    fn notes_and_tags_survive_a_round_trip() {
        let notes = join_notes("Line one\nLine two", &tags(&["x", "y"]));
        assert_eq!(split_notes(&notes), ("Line one\nLine two".to_string(), tags(&["x", "y"])));
    }

    // --- Converting between tasks and reminders -----------------------------

    #[test]
    fn task_and_reminder_data_convert_both_ways() {
        let task = Task {
            id: 3,
            due: Some(date("2026-10-20")),
            priority: Priority::High,
            tags: tags(&["work"]),
            notes: "Bring slides".into(),
            repeat: Some(Repeat { unit: RepeatUnit::Week, interval: 1 }),
            list: "Todo".into(),
            ..Task::new("Weekly sync")
        };
        let data = to_data(&task);
        assert_eq!(data.notes, "Bring slides\n#work");
        assert_eq!(data.priority, 1);

        let back = to_task(3, "Todo".into(), data);
        assert_eq!(serde_json::to_value(&back).unwrap(), serde_json::to_value(&task).unwrap());
    }

    #[test]
    fn repeating_task_without_due_date_starts_today() {
        let task = Task { repeat: Some(Repeat { unit: RepeatUnit::Day, interval: 1 }), ..Task::new("a") };
        assert_eq!(to_data(&task).due, Some(task::today()));
        assert_eq!(to_data(&Task::new("a")).due, None);
    }

    // --- Filter and sorting -------------------------------------------------

    fn sample_tasks() -> Vec<Task> {
        vec![
            Task { id: 1, due: Some(date("2026-10-20")), priority: Priority::Low, ..Task::new("Buy milk") },
            Task { id: 2, priority: Priority::High, tags: tags(&["work"]), notes: "about the PLANTS".into(), ..Task::new("Email boss") },
            Task { id: 3, due: Some(date("2026-10-18")), priority: Priority::Low, done: true, ..Task::new("Old thing") },
            Task { id: 4, due: Some(date("2026-10-18")), priority: Priority::High, tags: tags(&["work", "home"]), ..Task::new("Water plants") },
        ]
    }

    fn matching(filter: &Filter) -> Vec<i64> {
        sample_tasks().iter().filter(|t| filter.matches(t)).map(|t| t.id).collect()
    }

    #[test]
    fn empty_filter_matches_everything() {
        assert_eq!(matching(&all()), [1, 2, 3, 4]);
    }

    #[test]
    fn filter_by_tag_priority_and_pending() {
        assert_eq!(matching(&Filter { tag: Some("work".into()), ..all() }), [2, 4]);
        assert_eq!(matching(&Filter { priority: Some(Priority::Low), ..all() }), [1, 3]);
        assert_eq!(matching(&Filter { pending_only: true, ..all() }), [1, 2, 4]);
    }

    #[test]
    fn search_looks_in_title_and_notes_ignoring_case() {
        assert_eq!(matching(&Filter { search: Some("plant".into()), ..all() }), [2, 4]);
        assert_eq!(matching(&Filter { search: Some("MILK".into()), ..all() }), [1]);
        assert_eq!(matching(&Filter { search: Some("nothing".into()), ..all() }), Vec::<i64>::new());
    }

    #[test]
    fn due_by_needs_a_due_date_on_or_before_the_day() {
        assert_eq!(matching(&Filter { due_by: Some(date("2026-10-18")), ..all() }), [3, 4]);
        assert_eq!(matching(&Filter { due_by: Some(date("2026-10-17")), ..all() }), Vec::<i64>::new());
    }

    #[test]
    fn filters_combine() {
        let filter = Filter { tag: Some("work".into()), search: Some("water".into()), ..all() };
        assert_eq!(matching(&filter), [4]);
    }

    fn sorted(key: SortKey) -> Vec<i64> {
        let mut tasks = sample_tasks();
        tasks.reverse();
        key.sort(&mut tasks);
        tasks.iter().map(|t| t.id).collect()
    }

    #[test]
    fn sort_by_id() {
        assert_eq!(sorted(SortKey::Id), [1, 2, 3, 4]);
    }

    #[test]
    fn sort_by_due_puts_undated_last_and_breaks_ties_by_priority() {
        assert_eq!(sorted(SortKey::Due), [4, 3, 1, 2]);
    }

    #[test]
    fn sort_by_priority_then_due() {
        assert_eq!(sorted(SortKey::Priority), [4, 2, 3, 1]);
    }

    #[test]
    fn sort_keys_cycle_and_have_labels() {
        assert_eq!(SortKey::Id.next(), SortKey::Due);
        assert_eq!(SortKey::Due.next(), SortKey::Priority);
        assert_eq!(SortKey::Priority.next(), SortKey::Id);
        assert_eq!(SortKey::Id.label(), "id");
        assert_eq!(SortKey::Due.label(), "due date");
        assert_eq!(SortKey::Priority.label(), "priority");
    }

    // --- Store --------------------------------------------------------------

    #[test]
    fn list_name_comes_from_the_backend() {
        assert_eq!(testing::store().list_name(), "Todo");
    }

    #[test]
    fn add_assigns_increasing_ids_and_writes_the_reminder() {
        let mut db = testing::store();
        let first = db.add(&Task { tags: tags(&["work"]), ..Task::new("First") }).unwrap();
        let second = db.add(&Task::new("Second")).unwrap();
        assert_eq!((first.id, second.id), (1, 2));
        assert_eq!((first.list.as_str(), first.tags.clone()), ("Todo", tags(&["work"])));
        assert_eq!(db.backend().data("fake-1").unwrap().notes, "#work");
    }

    #[test]
    fn reminders_made_elsewhere_get_numbers_oldest_first_and_keep_them() {
        let mut db = testing::store();
        db.backend().push(data("Made first on iPhone"), "Todo");
        db.backend().push(data("Made second on iPhone"), "Todo");
        db.backend().push(data("In another list"), "Shopping");

        let tasks = db.list(&all()).unwrap();
        assert_eq!(titles(&tasks), ["Made first on iPhone", "Made second on iPhone"]);
        assert_eq!(tasks.iter().map(|t| t.id).collect::<Vec<_>>(), [1, 2]);

        let added = db.add(&Task::new("From the CLI")).unwrap();
        assert_eq!(added.id, 3);
        let again = db.list(&all()).unwrap();
        assert_eq!(again.iter().map(|t| t.id).collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[test]
    fn ids_are_never_reused() {
        let mut db = testing::store();
        db.add(&Task::new("a")).unwrap();
        let b = db.add(&Task::new("b")).unwrap();
        db.remove(b.id).unwrap();
        assert_eq!(db.add(&Task::new("c")).unwrap().id, 3);
    }

    #[test]
    fn get_returns_none_for_unknown_or_deleted_tasks() {
        let mut db = testing::store();
        assert!(db.get(42).unwrap().is_none());
        let task = db.add(&Task::new("a")).unwrap();
        db.backend().delete("fake-1").unwrap();
        assert!(db.get(task.id).unwrap().is_none());
    }

    #[test]
    fn list_applies_filter_and_sort() {
        let mut db = testing::store();
        db.add(&Task { due: Some(date("2026-10-20")), ..Task::new("Later") }).unwrap();
        db.add(&Task { due: Some(date("2026-10-18")), ..Task::new("Sooner") }).unwrap();
        db.add(&Task::new("Whenever")).unwrap();
        let filter = Filter { search: Some("e".into()), sort: SortKey::Due, ..all() };
        assert_eq!(titles(&db.list(&filter).unwrap()), ["Sooner", "Later", "Whenever"]);
    }

    #[test]
    fn update_changes_fields_and_keeps_other_notes() {
        let mut db = testing::store();
        let reminder_id = db.backend().push(
            ReminderData { notes: "Ask for the blue one\n#shop".into(), ..data("Paint") },
            "Todo",
        );
        let mut task = db.list(&all()).unwrap().remove(0);
        task.title = "Buy paint".into();
        task.tags.push("diy".into());
        task.priority = Priority::Medium;

        let updated = db.update(&task).unwrap();
        assert_eq!(updated.title, "Buy paint");
        let written = db.backend().data(&reminder_id).unwrap();
        assert_eq!(written.notes, "Ask for the blue one\n#shop #diy");
        assert_eq!(written.priority, 5);
    }

    #[test]
    fn changing_a_missing_task_is_an_error() {
        let mut db = testing::store();
        let error = |r: Result<Task>| r.unwrap_err().to_string();
        assert_eq!(error(db.set_done(9, true)), "no task with id 9");
        assert_eq!(error(db.remove(9)), "no task with id 9");
        assert_eq!(error(db.update(&Task { id: 9, ..Task::new("x") })), "no task with id 9");
    }

    #[test]
    fn set_done_marks_and_unmarks() {
        let mut db = testing::store();
        let task = db.add(&Task::new("a")).unwrap();
        assert!(db.set_done(task.id, true).unwrap().done);
        assert!(db.backend().data("fake-1").unwrap().done);
        assert!(!db.set_done(task.id, false).unwrap().done);
    }

    #[test]
    fn remove_deletes_the_reminder() {
        let mut db = testing::store();
        let task = db.add(&Task::new("a")).unwrap();
        assert_eq!(db.remove(task.id).unwrap().title, "a");
        assert!(db.backend().ids().is_empty());
    }

    // --- Undo ---------------------------------------------------------------

    #[test]
    fn undo_with_empty_history_does_nothing() {
        assert_eq!(testing::store().undo().unwrap(), None);
    }

    #[test]
    fn undo_add_deletes_the_task() {
        let mut db = testing::store();
        let task = db.add(&Task::new("a")).unwrap();
        assert_eq!(db.undo().unwrap().as_deref(), Some("add of task 1"));
        assert!(db.get(task.id).unwrap().is_none());
        assert!(db.backend().ids().is_empty());
    }

    #[test]
    fn undo_edit_restores_every_field() {
        let mut db = testing::store();
        let original = db.add(&Task { tags: tags(&["a"]), ..Task::new("Original") }).unwrap();
        db.update(&Task { title: "Changed".into(), tags: vec![], priority: Priority::High, ..original.clone() }).unwrap();
        assert_eq!(db.undo().unwrap().as_deref(), Some("edit of task 1"));
        let restored = db.get(original.id).unwrap().unwrap();
        assert_eq!((restored.title.as_str(), restored.tags.clone(), restored.priority), ("Original", tags(&["a"]), Priority::None));
    }

    #[test]
    fn undo_done_reopens_the_task() {
        let mut db = testing::store();
        let task = db.add(&Task::new("a")).unwrap();
        db.set_done(task.id, true).unwrap();
        assert_eq!(db.undo().unwrap().as_deref(), Some("done of task 1"));
        assert!(!db.get(task.id).unwrap().unwrap().done);
    }

    #[test]
    fn undo_remove_recreates_the_task_under_the_same_id() {
        let mut db = testing::store();
        let task = db.add(&Task { notes: "keep me".into(), ..Task::new("a") }).unwrap();
        db.remove(task.id).unwrap();
        assert_eq!(db.undo().unwrap().as_deref(), Some("remove of task 1"));

        let back = db.get(task.id).unwrap().unwrap();
        assert_eq!((back.title.as_str(), back.notes.as_str()), ("a", "keep me"));
        assert_eq!(db.backend().ids(), ["fake-2"], "a new reminder was created");
    }

    #[test]
    fn undo_remove_puts_the_task_back_in_its_own_list() {
        let mut db = testing::store();
        db.backend().switch_to("Shopping");
        let milk = db.add(&Task::new("Milk")).unwrap();
        db.remove(milk.id).unwrap();

        db.backend().switch_to("Todo");
        db.undo().unwrap();
        assert!(db.list(&all()).unwrap().is_empty(), "not recreated in Todo");
        assert_eq!(db.get(milk.id).unwrap().unwrap().list, "Shopping");
    }

    #[test]
    fn undo_recreates_a_task_deleted_on_another_device() {
        let mut db = testing::store();
        let task = db.add(&Task::new("Original")).unwrap();
        db.update(&Task { title: "Edited".into(), ..task.clone() }).unwrap();
        db.backend().delete("fake-1").unwrap();

        db.undo().unwrap();
        assert_eq!(db.get(task.id).unwrap().unwrap().title, "Original");
    }

    #[test]
    fn undo_steps_back_through_several_changes() {
        let mut db = testing::store();
        let task = db.add(&Task::new("a")).unwrap();
        db.set_done(task.id, true).unwrap();
        db.remove(task.id).unwrap();

        let undone: Vec<String> = (0..4).filter_map(|_| db.undo().unwrap()).collect();
        assert_eq!(undone, ["remove of task 1", "done of task 1", "add of task 1"]);
        assert!(db.backend().ids().is_empty());
    }
}
