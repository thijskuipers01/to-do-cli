//! The [`Store`]: tasks kept in a reminders list, plus short IDs and an undo
//! history in a small local SQLite database.

mod convert;
mod filter;

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::model::Task;
use crate::reminders::{Backend, RemindersList};
use convert::{to_data, to_task};
pub use filter::{Filter, SortKey};

/// Tasks themselves live in the reminders list. This local database only
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

/// Tasks, stored in a reminders list `B`: the real Reminders app by default,
/// or any other [`Backend`] such as [`MemoryList`](crate::MemoryList).
pub struct Store<B = RemindersList> {
    conn: Connection,
    reminders: B,
}

impl Store {
    /// Opens the local state database at `state_path` and the Reminders list
    /// `list_name`. The first time, macOS asks for access to Reminders.
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

    /// A store whose state lives only in memory, e.g. for tests.
    pub fn in_memory(reminders: B) -> Result<Store<B>> {
        Store::with_backend(Connection::open_in_memory()?, reminders)
    }

    /// Name of the reminders list this store works on.
    pub fn list_name(&self) -> String {
        self.reminders.name()
    }

    /// The underlying reminders list.
    pub fn backend(&self) -> &B {
        &self.reminders
    }

    /// The tasks in the list that match `filter`, in its sort order.
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

    /// The task with this ID, or `None` if there is none (anymore).
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

    /// Marks a task as done or not done and returns it as it is now.
    pub fn set_done(&mut self, id: i64, done: bool) -> Result<Task> {
        let before = self.require(id)?;
        self.write(&Task { done, ..before.clone() })?;
        self.record(if done { "done" } else { "undone" }, id, Some(&before))?;
        // Read it back: completing a repeating task moves it to its next date
        // (Reminders keeps a completed copy as a separate reminder).
        self.require(id)
    }

    /// Deletes a task and returns it as it was.
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

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::model::Priority;
    use crate::reminders::{MemoryList, ReminderData};

    fn store() -> Store<MemoryList> {
        Store::in_memory(MemoryList::new("Todo")).unwrap()
    }

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

    #[test]
    fn list_name_comes_from_the_backend() {
        assert_eq!(store().list_name(), "Todo");
    }

    #[test]
    fn add_assigns_increasing_ids_and_writes_the_reminder() {
        let mut db = store();
        let first = db.add(&Task { tags: tags(&["work"]), ..Task::new("First") }).unwrap();
        let second = db.add(&Task::new("Second")).unwrap();
        assert_eq!((first.id, second.id), (1, 2));
        assert_eq!((first.list.as_str(), first.tags.clone()), ("Todo", tags(&["work"])));
        assert_eq!(db.backend().data("memory-1").unwrap().notes, "#work");
    }

    #[test]
    fn reminders_made_elsewhere_get_numbers_oldest_first_and_keep_them() {
        let mut db = store();
        db.backend().push(ReminderData::new("Made first on iPhone"), "Todo");
        db.backend().push(ReminderData::new("Made second on iPhone"), "Todo");
        db.backend().push(ReminderData::new("In another list"), "Shopping");

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
        let mut db = store();
        db.add(&Task::new("a")).unwrap();
        let b = db.add(&Task::new("b")).unwrap();
        db.remove(b.id).unwrap();
        assert_eq!(db.add(&Task::new("c")).unwrap().id, 3);
    }

    #[test]
    fn get_returns_none_for_unknown_or_deleted_tasks() {
        let mut db = store();
        assert!(db.get(42).unwrap().is_none());
        let task = db.add(&Task::new("a")).unwrap();
        db.backend().delete("memory-1").unwrap();
        assert!(db.get(task.id).unwrap().is_none());
    }

    #[test]
    fn list_applies_filter_and_sort() {
        let mut db = store();
        db.add(&Task { due: Some(date("2026-10-20")), ..Task::new("Later") }).unwrap();
        db.add(&Task { due: Some(date("2026-10-18")), ..Task::new("Sooner") }).unwrap();
        db.add(&Task::new("Whenever")).unwrap();
        let filter = Filter { search: Some("e".into()), sort: SortKey::Due, ..all() };
        assert_eq!(titles(&db.list(&filter).unwrap()), ["Sooner", "Later", "Whenever"]);
    }

    #[test]
    fn update_changes_fields_and_keeps_other_notes() {
        let mut db = store();
        let reminder_id = db.backend().push(
            ReminderData { notes: "Ask for the blue one\n#shop".into(), ..ReminderData::new("Paint") },
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
        let mut db = store();
        let error = |r: Result<Task>| r.unwrap_err().to_string();
        assert_eq!(error(db.set_done(9, true)), "no task with id 9");
        assert_eq!(error(db.remove(9)), "no task with id 9");
        assert_eq!(error(db.update(&Task { id: 9, ..Task::new("x") })), "no task with id 9");
    }

    #[test]
    fn set_done_marks_and_unmarks() {
        let mut db = store();
        let task = db.add(&Task::new("a")).unwrap();
        assert!(db.set_done(task.id, true).unwrap().done);
        assert!(db.backend().data("memory-1").unwrap().done);
        assert!(!db.set_done(task.id, false).unwrap().done);
    }

    #[test]
    fn remove_deletes_the_reminder() {
        let mut db = store();
        let task = db.add(&Task::new("a")).unwrap();
        assert_eq!(db.remove(task.id).unwrap().title, "a");
        assert!(db.backend().ids().is_empty());
    }

    // --- Undo ---------------------------------------------------------------

    #[test]
    fn undo_with_empty_history_does_nothing() {
        assert_eq!(store().undo().unwrap(), None);
    }

    #[test]
    fn undo_add_deletes_the_task() {
        let mut db = store();
        let task = db.add(&Task::new("a")).unwrap();
        assert_eq!(db.undo().unwrap().as_deref(), Some("add of task 1"));
        assert!(db.get(task.id).unwrap().is_none());
        assert!(db.backend().ids().is_empty());
    }

    #[test]
    fn undo_edit_restores_every_field() {
        let mut db = store();
        let original = db.add(&Task { tags: tags(&["a"]), ..Task::new("Original") }).unwrap();
        db.update(&Task { title: "Changed".into(), tags: vec![], priority: Priority::High, ..original.clone() }).unwrap();
        assert_eq!(db.undo().unwrap().as_deref(), Some("edit of task 1"));
        let restored = db.get(original.id).unwrap().unwrap();
        assert_eq!((restored.title.as_str(), restored.tags.clone(), restored.priority), ("Original", tags(&["a"]), Priority::None));
    }

    #[test]
    fn undo_done_reopens_the_task() {
        let mut db = store();
        let task = db.add(&Task::new("a")).unwrap();
        db.set_done(task.id, true).unwrap();
        assert_eq!(db.undo().unwrap().as_deref(), Some("done of task 1"));
        assert!(!db.get(task.id).unwrap().unwrap().done);
    }

    #[test]
    fn undo_remove_recreates_the_task_under_the_same_id() {
        let mut db = store();
        let task = db.add(&Task { notes: "keep me".into(), ..Task::new("a") }).unwrap();
        db.remove(task.id).unwrap();
        assert_eq!(db.undo().unwrap().as_deref(), Some("remove of task 1"));

        let back = db.get(task.id).unwrap().unwrap();
        assert_eq!((back.title.as_str(), back.notes.as_str()), ("a", "keep me"));
        assert_eq!(db.backend().ids(), ["memory-2"], "a new reminder was created");
    }

    #[test]
    fn undo_remove_puts_the_task_back_in_its_own_list() {
        let mut db = store();
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
        let mut db = store();
        let task = db.add(&Task::new("Original")).unwrap();
        db.update(&Task { title: "Edited".into(), ..task.clone() }).unwrap();
        db.backend().delete("memory-1").unwrap();

        db.undo().unwrap();
        assert_eq!(db.get(task.id).unwrap().unwrap().title, "Original");
    }

    #[test]
    fn undo_steps_back_through_several_changes() {
        let mut db = store();
        let task = db.add(&Task::new("a")).unwrap();
        db.set_done(task.id, true).unwrap();
        db.remove(task.id).unwrap();

        let undone: Vec<String> = (0..4).filter_map(|_| db.undo().unwrap()).collect();
        assert_eq!(undone, ["remove of task 1", "done of task 1", "add of task 1"]);
        assert!(db.backend().ids().is_empty());
    }
}
