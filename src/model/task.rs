use std::cmp::Ordering;

use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

use super::{Priority, Repeat};

/// A single to-do item, stored as a reminder in the Reminders app. It can be
/// serialized; the undo history stores snapshots of tasks as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Short number for the command line. 0 for a task that isn't saved yet.
    pub id: i64,
    pub title: String,
    pub done: bool,
    pub due: Option<NaiveDate>,
    /// Time of day the task is due, if any. Only meaningful with `due`.
    #[serde(default)]
    pub due_time: Option<NaiveTime>,
    pub priority: Priority,
    pub tags: Vec<String>,
    /// The reminder's notes, apart from the line of hashtags holding the tags.
    pub notes: String,
    #[serde(default)]
    pub repeat: Option<Repeat>,
    /// Name of the Reminders list the task is in.
    #[serde(default)]
    pub list: String,
}

/// Where a due date stands compared to today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueStatus {
    Overdue,
    Today,
    Upcoming,
}

impl Task {
    /// A new, open task with only a title. The ID is assigned when it's saved.
    pub fn new(title: &str) -> Task {
        Task {
            id: 0,
            title: title.to_string(),
            done: false,
            due: None,
            due_time: None,
            priority: Priority::None,
            tags: Vec::new(),
            notes: String::new(),
            repeat: None,
            list: String::new(),
        }
    }

    /// The due date and time as text ("2026-10-20 15:00"), without the time
    /// for an all-day task.
    pub fn due_text(&self) -> String {
        match (self.due, self.due_time) {
            (Some(due), Some(time)) => format!("{due} {}", time.format("%H:%M")),
            (Some(due), None) => due.to_string(),
            (None, _) => String::new(),
        }
    }

    /// The due date and time as text, with "↻" added for repeating tasks.
    pub fn due_label(&self) -> String {
        match (self.due, self.repeat) {
            (Some(_), Some(_)) => format!("{} ↻", self.due_text()),
            _ => self.due_text(),
        }
    }

    /// Where the due date stands relative to `today`, or `None` if the task
    /// has no due date or is already done.
    pub fn due_status(&self, today: NaiveDate) -> Option<DueStatus> {
        let due = self.due?;
        if self.done {
            return None;
        }
        Some(match due.cmp(&today) {
            Ordering::Less => DueStatus::Overdue,
            Ordering::Equal => DueStatus::Today,
            Ordering::Greater => DueStatus::Upcoming,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RepeatUnit;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn new_task_has_only_a_title() {
        let task = Task::new("Buy milk");
        assert_eq!(task.title, "Buy milk");
        assert_eq!((task.id, task.done, task.due, task.due_time, task.repeat), (0, false, None, None, None));
        assert_eq!(task.priority, Priority::None);
        assert!(task.tags.is_empty() && task.notes.is_empty() && task.list.is_empty());
    }

    #[test]
    fn due_label_marks_repeating_tasks() {
        let due = Some(date("2026-10-20"));
        assert_eq!(Task::new("a").due_label(), "");
        assert_eq!(Task { due, ..Task::new("a") }.due_label(), "2026-10-20");
        let weekly = Some(Repeat { unit: RepeatUnit::Week, interval: 1 });
        let repeating = Task { due, repeat: weekly, ..Task::new("a") };
        assert_eq!(repeating.due_label(), "2026-10-20 ↻");
        let timed = Task { due_time: Some("15:00:00".parse().unwrap()), ..repeating };
        assert_eq!(timed.due_label(), "2026-10-20 15:00 ↻");
        assert_eq!(timed.due_text(), "2026-10-20 15:00");
    }

    #[test]
    fn due_status_compares_with_today() {
        let today = date("2026-10-17");
        let due = |d: &str| Task { due: Some(date(d)), ..Task::new("a") };
        assert_eq!(due("2026-10-16").due_status(today), Some(DueStatus::Overdue));
        assert_eq!(due("2026-10-17").due_status(today), Some(DueStatus::Today));
        assert_eq!(due("2026-10-18").due_status(today), Some(DueStatus::Upcoming));
        assert_eq!(Task::new("a").due_status(today), None);
        let done = Task { done: true, ..due("2026-10-16") };
        assert_eq!(done.due_status(today), None);
    }

    #[test]
    fn old_undo_snapshots_without_new_fields_still_load() {
        // Snapshots saved before `repeat`, `list` and `due_time` existed must still work.
        let json = r#"{"id":1,"title":"Old","done":false,"due":null,"priority":"High","tags":[],"notes":""}"#;
        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!((task.repeat, task.list.as_str(), task.due_time), (None, "", None));
    }
}
