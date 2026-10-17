use comfy_table::presets::UTF8_FULL_CONDENSED;
use chrono::NaiveDate;
use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table};

use crate::task::{self, DueStatus, Priority, Task};

/// Prints tasks as a colored table. Colors are left out automatically when
/// the output is not a terminal (e.g. when piped to a file).
pub fn print_tasks(tasks: &[Task]) {
    if tasks.is_empty() {
        println!("No tasks found. Add one with: todo add \"Buy milk\"");
        return;
    }

    let today = task::today();
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

    println!("{table}");
}

/// Prints every detail of one task, including its notes.
pub fn print_task_details(task: &Task) {
    let today = task::today();
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

    println!("Task {}: {}", task.id, task.title);
    println!("  Status    {}", if task.done { "done" } else { "open" });
    println!("  List      {}", task.list);
    println!("  Due       {due}");
    println!("  Repeats   {}", task.repeat.map_or_else(dash, |r| r.label()));
    println!("  Priority  {priority}");
    println!("  Tags      {tags}");
    if task.notes.trim().is_empty() {
        println!("  Notes     -");
    } else {
        println!("  Notes");
        for line in task.notes.lines() {
            println!("    {line}");
        }
    }
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
