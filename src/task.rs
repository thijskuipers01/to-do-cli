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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatUnit {
    Day,
    Week,
    Month,
    Year,
}

/// How often a task comes back, e.g. every 2 weeks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repeat {
    pub unit: RepeatUnit,
    pub interval: u32,
}

impl Repeat {
    /// "every week", "every 2 weeks", ...
    pub fn label(self) -> String {
        let unit = match self.unit {
            RepeatUnit::Day => "day",
            RepeatUnit::Week => "week",
            RepeatUnit::Month => "month",
            RepeatUnit::Year => "year",
        };
        match self.interval {
            1 => format!("every {unit}"),
            n => format!("every {n} {unit}s"),
        }
    }
}

/// Parses a repeat interval: "week", "every week", "2 weeks", "weekly", ...
pub fn parse_repeat(input: &str) -> Result<Repeat, String> {
    let error = || {
        format!("expected something like \"week\", \"2 weeks\" or \"monthly\", got \"{input}\"")
    };
    let lower = input.trim().to_lowercase();
    let mut words: Vec<&str> = lower.split_whitespace().collect();
    if words.first() == Some(&"every") {
        words.remove(0);
    }
    let (interval, unit) = match words.as_slice() {
        [unit] => (1, *unit),
        [count, unit] => (count.parse().map_err(|_| error())?, *unit),
        _ => return Err(error()),
    };
    let unit = match unit.trim_end_matches('s') {
        "day" | "daily" => RepeatUnit::Day,
        "week" | "weekly" => RepeatUnit::Week,
        "month" | "monthly" => RepeatUnit::Month,
        "year" | "yearly" => RepeatUnit::Year,
        _ => return Err(error()),
    };
    if interval == 0 {
        return Err(error());
    }
    Ok(Repeat { unit, interval })
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
    #[serde(default)]
    pub repeat: Option<Repeat>,
    /// Name of the Reminders list the task is in.
    #[serde(default)]
    pub list: String,
}

impl Task {
    /// A new, open task with only a title. The ID is assigned when it's saved.
    pub fn new(title: &str) -> Task {
        Task {
            id: 0,
            title: title.to_string(),
            done: false,
            due: None,
            priority: Priority::None,
            tags: Vec::new(),
            notes: String::new(),
            repeat: None,
            list: String::new(),
        }
    }
}

pub enum DueStatus {
    Overdue,
    Today,
    Upcoming,
}

impl Task {
    /// Where the due date stands relative to `today`, or `None` if the task
    /// has no due date or is already done.
    /// The due date as text, with "↻" added for repeating tasks.
    pub fn due_label(&self) -> String {
        match (self.due, self.repeat) {
            (Some(due), Some(_)) => format!("{due} ↻"),
            (Some(due), None) => due.to_string(),
            (None, _) => String::new(),
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn repeat(unit: RepeatUnit, interval: u32) -> Repeat {
        Repeat { unit, interval }
    }

    // --- Priority ---------------------------------------------------------

    #[test]
    fn priority_from_eventkit_covers_the_whole_scale() {
        assert_eq!(Priority::from_eventkit(0), Priority::None);
        for value in 1..=4 {
            assert_eq!(Priority::from_eventkit(value), Priority::High);
        }
        assert_eq!(Priority::from_eventkit(5), Priority::Medium);
        for value in 6..=9 {
            assert_eq!(Priority::from_eventkit(value), Priority::Low);
        }
    }

    #[test]
    fn priority_survives_a_round_trip_through_eventkit() {
        for priority in [Priority::None, Priority::Low, Priority::Medium, Priority::High] {
            assert_eq!(Priority::from_eventkit(priority.to_eventkit()), priority);
        }
    }

    #[test]
    fn priority_labels_and_order() {
        assert_eq!(Priority::None.label(), "");
        assert_eq!(Priority::Low.label(), "low");
        assert_eq!(Priority::Medium.label(), "medium");
        assert_eq!(Priority::High.label(), "high");
        assert!(Priority::None < Priority::Low);
        assert!(Priority::Low < Priority::Medium);
        assert!(Priority::Medium < Priority::High);
    }

    // --- Repeat -----------------------------------------------------------

    #[test]
    fn repeat_labels() {
        assert_eq!(repeat(RepeatUnit::Day, 1).label(), "every day");
        assert_eq!(repeat(RepeatUnit::Week, 2).label(), "every 2 weeks");
        assert_eq!(repeat(RepeatUnit::Month, 3).label(), "every 3 months");
        assert_eq!(repeat(RepeatUnit::Year, 1).label(), "every year");
    }

    #[test]
    fn parse_repeat_accepts_common_forms() {
        let cases = [
            ("week", repeat(RepeatUnit::Week, 1)),
            ("every week", repeat(RepeatUnit::Week, 1)),
            ("weekly", repeat(RepeatUnit::Week, 1)),
            ("2 weeks", repeat(RepeatUnit::Week, 2)),
            ("every 3 days", repeat(RepeatUnit::Day, 3)),
            ("daily", repeat(RepeatUnit::Day, 1)),
            ("Monthly", repeat(RepeatUnit::Month, 1)),
            ("  every 1 year ", repeat(RepeatUnit::Year, 1)),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_repeat(input), Ok(expected), "input {input:?}");
        }
    }

    #[test]
    fn parse_repeat_rejects_nonsense() {
        for input in ["", "every", "fortnight", "0 weeks", "two weeks", "2 weeks please", "-1 day"] {
            assert!(parse_repeat(input).is_err(), "should reject {input:?}");
        }
    }

    #[test]
    fn repeat_labels_parse_back_to_the_same_repeat() {
        for r in [repeat(RepeatUnit::Day, 1), repeat(RepeatUnit::Week, 2), repeat(RepeatUnit::Year, 5)] {
            assert_eq!(parse_repeat(&r.label()), Ok(r));
        }
    }

    // --- Due dates ----------------------------------------------------------

    #[test]
    fn parse_due_accepts_dates_and_words() {
        assert_eq!(parse_due("2026-12-31"), Ok(date("2026-12-31")));
        assert_eq!(parse_due("today"), Ok(today()));
        assert_eq!(parse_due(" Tomorrow "), Ok(today() + Days::new(1)));
    }

    #[test]
    fn parse_due_weekdays_are_always_in_the_coming_week() {
        for name in ["monday", "Fri", "sunday", "SAT"] {
            let due = parse_due(name).unwrap();
            let days_ahead = (due - today()).num_days();
            assert!((1..=7).contains(&days_ahead), "{name} gave {due}");
            assert_eq!(due.weekday(), name.parse::<Weekday>().unwrap());
        }
    }

    #[test]
    fn parse_due_rejects_invalid_dates() {
        for input in ["", "2026-13-01", "2026-02-30", "31-12-2026", "someday"] {
            assert!(parse_due(input).is_err(), "should reject {input:?}");
        }
    }

    #[test]
    fn next_weekday_skips_to_next_week_on_the_same_day() {
        let saturday = date("2026-10-17");
        assert_eq!(next_weekday(saturday, Weekday::Sat), date("2026-10-24"));
        assert_eq!(next_weekday(saturday, Weekday::Sun), date("2026-10-18"));
        assert_eq!(next_weekday(saturday, Weekday::Fri), date("2026-10-23"));
        assert_eq!(next_weekday(saturday, Weekday::Mon), date("2026-10-19"));
    }

    // --- Task ---------------------------------------------------------------

    #[test]
    fn new_task_has_only_a_title() {
        let task = Task::new("Buy milk");
        assert_eq!(task.title, "Buy milk");
        assert_eq!((task.id, task.done, task.due, task.repeat), (0, false, None, None));
        assert_eq!(task.priority, Priority::None);
        assert!(task.tags.is_empty() && task.notes.is_empty() && task.list.is_empty());
    }

    #[test]
    fn due_label_marks_repeating_tasks() {
        let due = Some(date("2026-10-20"));
        assert_eq!(Task::new("a").due_label(), "");
        assert_eq!(Task { due, ..Task::new("a") }.due_label(), "2026-10-20");
        let repeating = Task { due, repeat: Some(repeat(RepeatUnit::Week, 1)), ..Task::new("a") };
        assert_eq!(repeating.due_label(), "2026-10-20 ↻");
    }

    #[test]
    fn due_status_compares_with_today() {
        let today = date("2026-10-17");
        let due = |d: &str| Task { due: Some(date(d)), ..Task::new("a") };
        assert!(matches!(due("2026-10-16").due_status(today), Some(DueStatus::Overdue)));
        assert!(matches!(due("2026-10-17").due_status(today), Some(DueStatus::Today)));
        assert!(matches!(due("2026-10-18").due_status(today), Some(DueStatus::Upcoming)));
        assert!(Task::new("a").due_status(today).is_none());
        let done = Task { done: true, ..due("2026-10-16") };
        assert!(done.due_status(today).is_none());
    }

    #[test]
    fn old_undo_snapshots_without_new_fields_still_load() {
        // Snapshots saved before `repeat` and `list` existed must still work.
        let json = r#"{"id":1,"title":"Old","done":false,"due":null,"priority":"High","tags":[],"notes":""}"#;
        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!((task.repeat, task.list.as_str()), (None, ""));
    }
}
