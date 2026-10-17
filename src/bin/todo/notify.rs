use std::process::Command;

use anyhow::{Context, Result, bail};
use chrono::{Days, NaiveDate};

use todo::model;
use todo::{Backend, Filter, Store};

/// Shows a desktop notification listing the unfinished tasks due tomorrow.
/// Does nothing when there are none.
pub fn due_tomorrow<B: Backend>(db: &Store<B>) -> Result<()> {
    let titles = titles_due_on(db, model::today() + Days::new(1))?;
    match notification(&titles) {
        None => println!("No tasks due tomorrow"),
        Some((heading, body)) => {
            println!("{heading}: {body}");
            show(&heading, &body)?;
        }
    }
    Ok(())
}

/// Titles of the unfinished tasks due on `date`.
fn titles_due_on<B: Backend>(db: &Store<B>, date: NaiveDate) -> Result<Vec<String>> {
    let filter = Filter { pending_only: true, ..Filter::default() };
    Ok(db
        .list(&filter)?
        .into_iter()
        .filter(|t| t.due == Some(date))
        .map(|t| t.title)
        .collect())
}

/// The notification's heading and body, or `None` when there's nothing to say.
fn notification(titles: &[String]) -> Option<(String, String)> {
    let heading = match titles.len() {
        0 => return None,
        1 => "1 task due tomorrow".to_string(),
        n => format!("{n} tasks due tomorrow"),
    };
    Some((heading, titles.join(", ")))
}

/// Shows a macOS notification using `osascript`. The text is passed as
/// arguments instead of being pasted into the script, so quotes in a task
/// title can't break the script.
fn show(heading: &str, body: &str) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("desktop notifications are only supported on macOS");
    }
    let status = Command::new("osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            heading,
            body,
        ])
        .status()
        .context("could not run osascript")?;
    if !status.success() {
        bail!("osascript failed ({status})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo::Task;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn titles_due_on_skips_other_dates_and_finished_tasks() {
        let mut db = crate::test_store();
        let due = Some(date("2026-10-18"));
        db.add(&Task { due, ..Task::new("Dentist") }).unwrap();
        db.add(&Task { due: Some(date("2026-10-19")), ..Task::new("Later") }).unwrap();
        db.add(&Task::new("No date")).unwrap();
        let done = db.add(&Task { due, ..Task::new("Already done") }).unwrap();
        db.set_done(done.id, true).unwrap();

        assert_eq!(titles_due_on(&db, date("2026-10-18")).unwrap(), ["Dentist"]);
    }

    #[test]
    fn notification_is_empty_without_tasks() {
        assert_eq!(notification(&[]), None);
    }

    #[test]
    fn notification_text_for_one_task() {
        let (heading, body) = notification(&["Dentist".to_string()]).unwrap();
        assert_eq!(heading, "1 task due tomorrow");
        assert_eq!(body, "Dentist");
    }

    #[test]
    fn notification_text_for_several_tasks() {
        let titles = ["Dentist".to_string(), "Pay rent".to_string()];
        let (heading, body) = notification(&titles).unwrap();
        assert_eq!(heading, "2 tasks due tomorrow");
        assert_eq!(body, "Dentist, Pay rent");
    }
}
