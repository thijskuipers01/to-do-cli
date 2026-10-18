//! The JSON that `todo --json` prints and the MCP server returns, so both
//! show tasks, lists, history and errors the same way.
//!
//! Unlike `Task`'s own serialization (used for the undo history), values use
//! the same words as the command-line options, so they can be passed straight
//! back: `"high"`, `"every 2 weeks"`. Empty fields are `null`.

use serde_json::{Value, json};

use crate::error::ErrorKind;
use crate::model::{Priority, Task};
use crate::reminders::ListSummary;
use crate::store::Change;

/// `{"id", "title", "done", "due", "due_time", "priority", "tags", "notes",
/// "repeat", "list"}`
pub fn task(task: &Task) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "done": task.done,
        "due": task.due,
        // "15:00", or null for an all-day task.
        "due_time": task.due.and(task.due_time).map(|t| t.format("%H:%M").to_string()),
        "priority": match task.priority {
            Priority::None => None,
            priority => Some(priority.label()),
        },
        "tags": task.tags,
        "notes": task.notes,
        "repeat": task.repeat.map(|r| r.label()),
        "list": task.list,
    })
}

/// An array of [`task`] objects.
pub fn tasks(tasks: &[Task]) -> Value {
    tasks.iter().map(task).collect()
}

/// `[{"name": "Todo", "open": 4}, ...]`
pub fn lists(lists: &[ListSummary]) -> Value {
    lists.iter().map(|l| json!({ "name": l.name, "open": l.open })).collect()
}

/// `[{"action", "id", "title", "at", "batch"}, ...]`, in the given order.
pub fn history(changes: &[Change]) -> Value {
    changes
        .iter()
        .map(|c| {
            json!({
                "action": c.action,
                "id": c.task_id,
                "title": c.title,
                "at": c.at.map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)),
                "batch": c.batch,
            })
        })
        .collect()
}

/// `{"undone": "done of task 3"}`, or `{"undone": null}`.
pub fn undone(what: Option<&str>) -> Value {
    json!({ "undone": what })
}

/// `{"error": "not_found", "message": "no task with id 7"}`
pub fn error(kind: ErrorKind, message: &str) -> Value {
    json!({ "error": kind.name(), "message": message })
}

#[cfg(test)]
mod tests {
    use chrono::{Local, NaiveDate, TimeZone};

    use super::*;
    use crate::model::{Repeat, RepeatUnit};

    fn full_task() -> Task {
        Task {
            id: 7,
            due: Some("2026-10-20".parse::<NaiveDate>().unwrap()),
            due_time: Some("09:30:00".parse().unwrap()),
            priority: Priority::High,
            tags: vec!["work".into(), "urgent".into()],
            notes: "Bring the report\nand slides".into(),
            repeat: Some(Repeat { unit: RepeatUnit::Week, interval: 2 }),
            list: "Todo".into(),
            ..Task::new("Weekly sync")
        }
    }

    #[test]
    fn task_uses_command_line_words() {
        assert_eq!(
            task(&full_task()),
            json!({
                "id": 7,
                "title": "Weekly sync",
                "done": false,
                "due": "2026-10-20",
                "due_time": "09:30",
                "priority": "high",
                "tags": ["work", "urgent"],
                "notes": "Bring the report\nand slides",
                "repeat": "every 2 weeks",
                "list": "Todo",
            })
        );
        assert_eq!(tasks(&[full_task()]), json!([task(&full_task())]));
    }

    #[test]
    fn task_uses_null_for_empty_fields() {
        let value = task(&Task::new("Bare"));
        for field in ["due", "due_time", "priority", "repeat"] {
            assert!(value[field].is_null(), "{field} should be null: {value}");
        }
    }

    #[test]
    fn lists_undone_and_errors() {
        let summary = ListSummary { name: "Shopping".into(), open: 3 };
        assert_eq!(lists(&[summary]), json!([{ "name": "Shopping", "open": 3 }]));
        assert_eq!(undone(Some("add of task 1")), json!({ "undone": "add of task 1" }));
        assert_eq!(undone(None), json!({ "undone": null }));
        assert_eq!(
            error(ErrorKind::NotFound, "no task with id 7").to_string(),
            r#"{"error":"not_found","message":"no task with id 7"}"#
        );
    }

    #[test]
    fn history_has_times_and_batches() {
        let at = Local.with_ymd_and_hms(2026, 10, 18, 14, 3, 0).unwrap();
        let changes = [
            Change { action: "done".into(), task_id: 3, title: "Milk".into(), at: Some(at), batch: Some(2) },
            Change { action: "add".into(), task_id: 1, title: "Old".into(), at: None, batch: None },
        ];
        let value = history(&changes);
        assert_eq!((&value[0]["action"], &value[0]["id"], &value[0]["batch"]), (&json!("done"), &json!(3), &json!(2)));
        assert!(value[0]["at"].as_str().unwrap().starts_with("2026-10-18T14:03:00"));
        assert!(value[1]["at"].is_null() && value[1]["batch"].is_null());
    }
}
