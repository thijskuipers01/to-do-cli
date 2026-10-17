use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::ValueEnum;
use rusqlite::{Connection, OptionalExtension, params};

use crate::reminders::{ReminderData, RemindersList};
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

pub struct Store {
    conn: Connection,
    reminders: RemindersList,
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
        conn.execute_batch(SCHEMA)?;
        let reminders = RemindersList::open(list_name)?;
        Ok(Store { conn, reminders })
    }

    /// Name of the Reminders list this store works on.
    pub fn list_name(&self) -> String {
        self.reminders.name()
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
