use std::process::Command;

use anyhow::{Context, Result, bail};
use chrono::Days;

use crate::store::{Filter, Store};
use crate::task;

/// Shows a desktop notification listing the unfinished tasks due tomorrow.
/// Does nothing when there are none.
pub fn due_tomorrow(db: &Store) -> Result<()> {
    let tomorrow = task::today() + Days::new(1);
    let filter = Filter { pending_only: true, ..Filter::default() };
    let titles: Vec<String> = db
        .list(&filter)?
        .into_iter()
        .filter(|t| t.due == Some(tomorrow))
        .map(|t| t.title)
        .collect();

    if titles.is_empty() {
        println!("No tasks due tomorrow");
        return Ok(());
    }

    let heading = match titles.len() {
        1 => "1 task due tomorrow".to_string(),
        n => format!("{n} tasks due tomorrow"),
    };
    let body = titles.join(", ");
    println!("{heading}: {body}");
    show(&heading, &body)
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
