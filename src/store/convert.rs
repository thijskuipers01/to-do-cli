//! Converting between tasks and reminders.
//!
//! EventKit can't read or write Reminders' own tags, so tags are stored as a
//! line of hashtags at the end of the reminder's notes: "#work #urgent".
//! Any other text in the notes is kept in `Task::notes` and left untouched.

use crate::model::{self, Priority, Task};
use crate::reminders::ReminderData;

pub(super) fn to_task(id: i64, list: String, data: ReminderData) -> Task {
    let (notes, tags) = split_notes(&data.notes);
    Task {
        id,
        title: data.title,
        done: data.done,
        due: data.due,
        due_time: data.due.and(data.due_time),
        priority: Priority::from_eventkit(data.priority),
        tags,
        notes,
        repeat: data.repeat,
        list,
    }
}

pub(super) fn to_data(task: &Task) -> ReminderData {
    ReminderData {
        title: task.title.clone(),
        notes: join_notes(&task.notes, &task.tags),
        done: task.done,
        // A repeating reminder needs a date to repeat from; start today.
        due: task.due.or(task.repeat.map(|_| model::today())),
        due_time: task.due.and(task.due_time),
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
    use chrono::NaiveDate;

    use super::*;
    use crate::model::{Repeat, RepeatUnit};

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|t| t.to_string()).collect()
    }

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

    #[test]
    fn task_and_reminder_data_convert_both_ways() {
        let task = Task {
            id: 3,
            due: Some(date("2026-10-20")),
            due_time: Some("15:30:00".parse().unwrap()),
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
    fn a_time_without_a_date_is_dropped() {
        let task = Task { due_time: Some("15:00:00".parse().unwrap()), ..Task::new("a") };
        assert_eq!(to_data(&task).due_time, None);
    }

    #[test]
    fn repeating_task_without_due_date_starts_today() {
        let task = Task { repeat: Some(Repeat { unit: RepeatUnit::Day, interval: 1 }), ..Task::new("a") };
        assert_eq!(to_data(&task).due, Some(model::today()));
        assert_eq!(to_data(&Task::new("a")).due, None);
    }
}
