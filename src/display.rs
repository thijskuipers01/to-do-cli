use comfy_table::presets::UTF8_FULL_CONDENSED;
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

        let due_text = task.due.map(|d| d.to_string()).unwrap_or_default();
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

fn priority_color(priority: Priority) -> Color {
    match priority {
        Priority::High => Color::Red,
        Priority::Medium => Color::Yellow,
        Priority::Low => Color::Green,
        Priority::None => Color::Reset,
    }
}
