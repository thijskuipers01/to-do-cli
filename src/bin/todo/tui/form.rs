//! The pop-up form for adding and editing a task.

use chrono::NaiveDate;
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use todo::model::{self, Priority, Repeat, Task};

/// Label and input hint for each form field, in display order.
pub(super) const FIELDS: [(&str, &str); 5] = [
    ("Title", "What needs to be done"),
    ("Due", "YYYY-MM-DD, today, tomorrow or a weekday; empty for none"),
    ("Priority", "low, medium or high; empty for none"),
    ("Tags", "Comma separated, e.g. work, urgent"),
    ("Repeat", "day, week, 2 weeks, monthly, ...; empty for none"),
];

pub(super) struct Form {
    /// `None` when adding a new task, otherwise the task being edited.
    pub editing: Option<Task>,
    /// The text in each field, in the same order as `FIELDS`.
    pub fields: [String; 5],
    /// Index of the field that receives typed keys.
    pub focus: usize,
    pub error: Option<String>,
}

/// The form's fields after checking and converting the typed text.
pub(super) struct FormValues {
    pub title: String,
    pub due: Option<NaiveDate>,
    pub priority: Priority,
    pub tags: Vec<String>,
    pub repeat: Option<Repeat>,
}

impl Form {
    pub fn new_task() -> Form {
        Form {
            editing: None,
            fields: Default::default(),
            focus: 0,
            error: None,
        }
    }

    pub fn edit(task: Task) -> Form {
        Form {
            fields: [
                task.title.clone(),
                task.due.map(|d| d.to_string()).unwrap_or_default(),
                task.priority.label().to_string(),
                task.tags.join(", "),
                task.repeat.map(|r| r.label()).unwrap_or_default(),
            ],
            editing: Some(task),
            focus: 0,
            error: None,
        }
    }

    pub fn parse(&self) -> Result<FormValues, String> {
        let [title, due, priority, tags, repeat] = &self.fields;

        let title = title.trim();
        if title.is_empty() {
            return Err("Title can't be empty".to_string());
        }
        let due = match due.trim() {
            "" => None,
            text => Some(model::parse_due(text)?),
        };
        let priority = match priority.trim() {
            "" => Priority::None,
            text => text
                .parse()
                .map_err(|_| "Priority must be none, low, medium or high".to_string())?,
        };
        let tags = tags
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(String::from)
            .collect();
        let repeat = match repeat.trim() {
            "" => None,
            text => Some(model::parse_repeat(text)?),
        };

        Ok(FormValues { title: title.to_string(), due, priority, tags, repeat })
    }

    pub fn draw(&self, frame: &mut Frame) {
        let heading = if self.editing.is_some() { " Edit task " } else { " New task " };
        // 5 fields + blank line + hint + keys, plus 2 for the border.
        let area = centered(frame.area(), 70, 10);

        let mut lines: Vec<Line> = FIELDS
            .iter()
            .zip(&self.fields)
            .enumerate()
            .map(|(i, ((label, _), value))| {
                let focused = i == self.focus;
                let marker = if focused { "› " } else { "  " };
                let label = Span::from(format!("{marker}{label:<9}")).bold();
                let cursor = if focused { "█" } else { "" };
                Line::from(vec![label, Span::from(format!("{value}{cursor}"))])
            })
            .collect();

        lines.push(Line::default());
        lines.push(match &self.error {
            Some(error) => Line::from(error.as_str()).red(),
            None => Line::from(FIELDS[self.focus].1).dim(),
        });
        lines.push(Line::from("Tab/↑↓ switch field · Enter save · Esc cancel").dim());

        // `Clear` blanks the table underneath so the form doesn't mix with it.
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines).block(Block::bordered().title(heading)),
            area,
        );
    }
}

/// A rectangle of at most `width` x `height`, centered in `area`.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    area
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo::RepeatUnit;

    fn form(fields: [&str; 5]) -> Form {
        Form { fields: fields.map(String::from), ..Form::new_task() }
    }

    #[test]
    fn new_form_is_empty() {
        let form = Form::new_task();
        assert!(form.editing.is_none() && form.error.is_none());
        assert!(form.fields.iter().all(String::is_empty));
        assert_eq!(form.focus, 0);
    }

    #[test]
    fn edit_form_is_filled_from_the_task() {
        let task = Task {
            due: Some("2026-10-20".parse().unwrap()),
            priority: Priority::High,
            tags: vec!["a".into(), "b".into()],
            repeat: Some(Repeat { unit: RepeatUnit::Week, interval: 2 }),
            ..Task::new("Title")
        };
        let form = Form::edit(task);
        assert_eq!(form.fields, ["Title", "2026-10-20", "high", "a, b", "every 2 weeks"]);
        assert!(form.editing.is_some());
    }

    #[test]
    fn parses_valid_input() {
        let values = form([" Milk ", "2026-10-20", "HIGH", "home, , shop ", "2 weeks"]).parse().unwrap();
        assert_eq!(values.title, "Milk");
        assert_eq!(values.due, Some("2026-10-20".parse().unwrap()));
        assert_eq!(values.priority, Priority::High);
        assert_eq!(values.tags, ["home", "shop"]);
        assert_eq!(values.repeat, Some(Repeat { unit: RepeatUnit::Week, interval: 2 }));
    }

    #[test]
    fn treats_empty_optional_fields_as_unset() {
        let values = form(["Milk", "", "", "", ""]).parse().unwrap();
        assert_eq!((values.due, values.priority, values.repeat), (None, Priority::None, None));
        assert!(values.tags.is_empty());
    }

    #[test]
    fn explains_invalid_input() {
        assert_eq!(form(["  ", "", "", "", ""]).parse().err().unwrap(), "Title can't be empty");
        assert!(form(["a", "someday", "", "", ""]).parse().err().unwrap().contains("YYYY-MM-DD"));
        assert_eq!(
            form(["a", "", "urgent", "", ""]).parse().err().unwrap(),
            "Priority must be none, low, medium or high"
        );
        assert!(form(["a", "", "", "", "fortnight"]).parse().err().unwrap().contains("2 weeks"));
    }

    #[test]
    fn centered_fits_inside_the_area() {
        assert_eq!(centered(Rect::new(0, 0, 100, 40), 70, 10), Rect::new(15, 15, 70, 10));
        assert_eq!(centered(Rect::new(0, 0, 50, 5), 70, 10), Rect::new(0, 0, 50, 5));
    }
}
