use std::fmt::Write;

use chrono::NaiveDate;
use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table};

use todo::model::{self, DueStatus, Priority, Task};

/// Prints tasks as a colored table. Colors are left out automatically when
/// the output is not a terminal (e.g. when piped to a file).
pub fn print_tasks(tasks: &[Task]) {
    if tasks.is_empty() {
        println!("No tasks found. Add one with: todo add \"Buy milk\"");
    } else {
        println!("{}", tasks_table(tasks, model::today()));
    }
}

/// Prints every detail of one task, including its notes.
pub fn print_task_details(task: &Task) {
    print!("{}", task_details(task, model::today()));
}

fn tasks_table(tasks: &[Task], today: NaiveDate) -> Table {
    let mut table = Table::new();
    table
        .load_style(UTF8_FULL_CONDENSED.with_rounded_corners())
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["ID", "", "Title", "Priority", "Due", "Tags"]);

    for task in tasks {
        let mut title = Cell::new(&task.title);
        if task.done {
            title = title
                .add_attribute(Attribute::CrossedOut)
                .fg(Color::DarkGrey);
        }

        let due_text = task.due_label();
        let due = match task.due_status(today) {
            Some(DueStatus::Overdue) => Cell::new(due_text)
                .fg(Color::Red)
                .add_attribute(Attribute::Bold),
            Some(DueStatus::Today) => Cell::new(due_text).fg(Color::Yellow),
            _ => Cell::new(due_text),
        };

        table.add_row(vec![
            Cell::new(task.id),
            Cell::new(if task.done { "✓" } else { "" }).fg(Color::Green),
            title,
            Cell::new(task.priority.label()).fg(priority_color(task.priority)),
            due,
            Cell::new(task.tags.join(", ")).fg(Color::Cyan),
        ]);
    }
    table
}

fn task_details(task: &Task, today: NaiveDate) -> String {
    let dash = || "-".to_string();
    let due = match task.due {
        Some(due) => format!("{due} ({})", relative_day(due, today)),
        None => dash(),
    };
    let priority = match task.priority {
        Priority::None => dash(),
        priority => priority.label().to_string(),
    };
    let tags = match task.tags.is_empty() {
        true => dash(),
        false => task.tags.join(", "),
    };

    // Writing to a String can't fail, so the results are ignored.
    let mut out = String::new();
    let _ = writeln!(out, "Task {}: {}", task.id, task.title);
    let _ = writeln!(out, "  Status    {}", if task.done { "done" } else { "open" });
    let _ = writeln!(out, "  List      {}", task.list);
    let _ = writeln!(out, "  Due       {due}");
    let _ = writeln!(out, "  Repeats   {}", task.repeat.map_or_else(dash, |r| r.label()));
    let _ = writeln!(out, "  Priority  {priority}");
    let _ = writeln!(out, "  Tags      {tags}");
    if task.notes.trim().is_empty() {
        let _ = writeln!(out, "  Notes     -");
    } else {
        let _ = writeln!(out, "  Notes");
        for line in task.notes.lines() {
            let _ = writeln!(out, "    {line}");
        }
    }
    out
}

/// "today", "tomorrow", "in 3 days", "2 days ago", ...
fn relative_day(date: NaiveDate, today: NaiveDate) -> String {
    match (date - today).num_days() {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        -1 => "yesterday".to_string(),
        days if days > 1 => format!("in {days} days"),
        days => format!("{} days ago", -days),
    }
}

fn priority_color(priority: Priority) -> Color {
    match priority {
        Priority::High => Color::Red,
        Priority::Medium => Color::Yellow,
        Priority::Low => Color::Green,
        Priority::None => Color::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo::{Repeat, RepeatUnit};

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn full_task() -> Task {
        Task {
            id: 7,
            due: Some(date("2026-10-20")),
            priority: Priority::High,
            tags: vec!["work".into(), "urgent".into()],
            notes: "Bring the report\nand slides".into(),
            repeat: Some(Repeat { unit: RepeatUnit::Week, interval: 2 }),
            list: "Todo".into(),
            ..Task::new("Weekly sync")
        }
    }

    #[test]
    fn table_has_a_row_per_task_with_its_fields() {
        let mut done = Task { id: 8, done: true, ..Task::new("Finished thing") };
        done.tags = vec!["home".into()];
        let table = tasks_table(&[full_task(), done], date("2026-10-17")).to_string();

        for text in ["ID", "Title", "Weekly sync", "high", "2026-10-20 ↻", "work, urgent"] {
            assert!(table.contains(text), "missing {text:?} in:\n{table}");
        }
        assert!(table.contains("Finished thing") && table.contains('✓'));
        assert_eq!(tasks_table(&[full_task()], date("2026-10-17")).row_count(), 1);
    }

    #[test]
    fn details_show_every_field() {
        let details = task_details(&full_task(), date("2026-10-17"));
        assert_eq!(
            details,
            "Task 7: Weekly sync\n\
             \x20 Status    open\n\
             \x20 List      Todo\n\
             \x20 Due       2026-10-20 (in 3 days)\n\
             \x20 Repeats   every 2 weeks\n\
             \x20 Priority  high\n\
             \x20 Tags      work, urgent\n\
             \x20 Notes\n\
             \x20   Bring the report\n\
             \x20   and slides\n"
        );
    }

    #[test]
    fn details_use_dashes_for_empty_fields() {
        let task = Task { id: 1, done: true, list: "Todo".into(), ..Task::new("Bare") };
        let details = task_details(&task, date("2026-10-17"));
        assert!(details.contains("Status    done"));
        for label in ["Due", "Repeats", "Priority", "Tags", "Notes"] {
            let line = details.lines().find(|l| l.trim_start().starts_with(label)).unwrap();
            assert!(line.ends_with(" -"), "{label} should be a dash: {line:?}");
        }
    }

    #[test]
    fn relative_day_names() {
        let today = date("2026-10-17");
        assert_eq!(relative_day(date("2026-10-17"), today), "today");
        assert_eq!(relative_day(date("2026-10-18"), today), "tomorrow");
        assert_eq!(relative_day(date("2026-10-16"), today), "yesterday");
        assert_eq!(relative_day(date("2026-10-27"), today), "in 10 days");
        assert_eq!(relative_day(date("2026-10-14"), today), "3 days ago");
    }

    #[test]
    fn priority_colors() {
        assert_eq!(priority_color(Priority::High), Color::Red);
        assert_eq!(priority_color(Priority::Medium), Color::Yellow);
        assert_eq!(priority_color(Priority::Low), Color::Green);
        assert_eq!(priority_color(Priority::None), Color::Reset);
    }
}
