use std::cmp::Ordering;

use chrono::{Datelike, Days, Local, NaiveDate, Weekday};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// How urgent a task is. The derived `Ord` follows declaration order,
/// so `None < Low < Medium < High`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ValueEnum)]
pub enum Priority {
    None,
    Low,
    Medium,
    High,
}

impl Priority {
    /// Converts from EventKit's scale: 0 = none, 1–4 = high, 5 = medium, 6–9 = low.
    pub fn from_eventkit(value: usize) -> Priority {
        match value {
            0 => Priority::None,
            1..=4 => Priority::High,
            5 => Priority::Medium,
            _ => Priority::Low,
        }
    }

    /// The values the Reminders app itself uses for each level.
    pub fn to_eventkit(self) -> usize {
        match self {
            Priority::None => 0,
            Priority::High => 1,
            Priority::Medium => 5,
            Priority::Low => 9,
        }
    }

    /// Text shown in tables; empty when there is no priority.
    pub fn label(self) -> &'static str {
        match self {
            Priority::None => "",
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
        }
    }
}

/// A single to-do item, stored as a reminder in the Reminders app. Serde is
/// still used here: the undo history stores snapshots of tasks as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub done: bool,
    pub due: Option<NaiveDate>,
    pub priority: Priority,
    pub tags: Vec<String>,
    /// The reminder's notes, apart from the line of hashtags holding the tags.
    pub notes: String,
}

pub enum DueStatus {
    Overdue,
    Today,
    Upcoming,
}

impl Task {
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

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Parses a due date: `YYYY-MM-DD`, `today`, `tomorrow` or a weekday
/// (`friday`, `fri`), which means the next such day after today.
pub fn parse_due(input: &str) -> Result<NaiveDate, String> {
    let today = today();
    match input.trim().to_lowercase().as_str() {
        "today" => Ok(today),
        "tomorrow" => Ok(today + Days::new(1)),
        other => {
            if let Ok(weekday) = other.parse::<Weekday>() {
                return Ok(next_weekday(today, weekday));
            }
            NaiveDate::parse_from_str(other, "%Y-%m-%d").map_err(|_| {
                format!("expected YYYY-MM-DD, \"today\", \"tomorrow\" or a weekday, got \"{input}\"")
            })
        }
    }
}

/// The first `weekday` strictly after `from` (so "friday" on a Friday is next week).
fn next_weekday(from: NaiveDate, weekday: Weekday) -> NaiveDate {
    let ahead = (7 + weekday.num_days_from_monday() - from.weekday().num_days_from_monday()) % 7;
    let ahead = if ahead == 0 { 7 } else { ahead };
    from + Days::new(ahead.into())
}
