use anyhow::Result;
use chrono::NaiveDate;
use clap::ValueEnum;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState};
use ratatui::{DefaultTerminal, Frame};

use crate::store::{Filter, Store};
use crate::task::{self, DueStatus, Priority, Repeat, Task};

const HELP: &str =
    "a add · e edit · space done · d delete · u undo · / search · s sort · h hide done · q quit";

/// Runs the interactive UI. The terminal is restored afterwards, even when
/// the app stops with an error.
pub fn run(db: &mut Store) -> Result<()> {
    let mut app = App::new(db)?;
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}

enum Mode {
    Normal,
    /// Typing a search; the list filters as you type.
    Search,
    /// The add/edit form is open. Boxed so `Mode` stays small.
    Form(Box<Form>),
}

struct App<'a> {
    db: &'a mut Store,
    filter: Filter,
    tasks: Vec<Task>,
    table_state: TableState,
    mode: Mode,
    /// Feedback shown in the footer after an action, e.g. "Removed task 3".
    message: Option<String>,
    quit: bool,
}

impl<'a> App<'a> {
    fn new(db: &'a mut Store) -> Result<Self> {
        let mut app = App {
            db,
            filter: Filter::default(),
            tasks: Vec::new(),
            table_state: TableState::default(),
            mode: Mode::Normal,
            message: None,
            quit: false,
        };
        app.reload(None)?;
        Ok(app)
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            terminal.draw(|frame| self.draw(frame))?;
            if let Event::Key(key) = event::read()? {
                // Some terminals also report key releases; only react to presses.
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                // Show database errors in the footer instead of closing the UI.
                if let Err(err) = self.handle_key(key.code) {
                    self.message = Some(format!("Error: {err:#}"));
                }
            }
        }
        Ok(())
    }

    /// Reloads tasks from the database. Selects the task `select_id` if given
    /// and still visible; otherwise keeps the cursor at the same row.
    fn reload(&mut self, select_id: Option<i64>) -> Result<()> {
        self.tasks = self.db.list(&self.filter)?;
        let position = select_id.and_then(|id| self.tasks.iter().position(|t| t.id == id));
        let index = match position {
            Some(index) => Some(index),
            None if self.tasks.is_empty() => None,
            None => Some(self.table_state.selected().unwrap_or(0).min(self.tasks.len() - 1)),
        };
        self.table_state.select(index);
        Ok(())
    }

    fn selected(&self) -> Option<&Task> {
        self.table_state.selected().and_then(|i| self.tasks.get(i))
    }

    fn handle_key(&mut self, code: KeyCode) -> Result<()> {
        match &mut self.mode {
            Mode::Form(form) => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Tab | KeyCode::Down => form.focus = (form.focus + 1) % FIELDS.len(),
                KeyCode::BackTab | KeyCode::Up => {
                    form.focus = (form.focus + FIELDS.len() - 1) % FIELDS.len();
                }
                KeyCode::Backspace => {
                    form.fields[form.focus].pop();
                }
                KeyCode::Char(c) => form.fields[form.focus].push(c),
                KeyCode::Enter => match form.parse() {
                    // Invalid input: keep the form open and show what is wrong.
                    Err(err) => form.error = Some(err),
                    Ok(values) => {
                        let editing = form.editing.take();
                        self.mode = Mode::Normal;
                        self.save_form(editing, values)?;
                    }
                },
                _ => {}
            },
            Mode::Search => self.handle_search_key(code)?,
            Mode::Normal => self.handle_normal_key(code)?,
        }
        Ok(())
    }

    fn handle_search_key(&mut self, code: KeyCode) -> Result<()> {
        let mut text = self.filter.search.take().unwrap_or_default();
        match code {
            // Enter keeps the search active; Esc clears it.
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Esc => {
                text.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char(c) => text.push(c),
            _ => {}
        }
        self.filter.search = (!text.is_empty()).then_some(text);
        let selected_id = self.selected().map(|t| t.id);
        self.reload(selected_id)
    }

    /// Adds a new task, or updates `editing` with the form's values.
    fn save_form(&mut self, editing: Option<Task>, values: FormValues) -> Result<()> {
        let is_new = editing.is_none();
        let base = editing.unwrap_or_else(|| Task::new(&values.title));
        let task = Task {
            title: values.title,
            due: values.due,
            priority: values.priority,
            tags: values.tags,
            repeat: values.repeat,
            ..base
        };
        let (task, verb) = if is_new {
            (self.db.add(&task)?, "Added")
        } else {
            (self.db.update(&task)?, "Updated")
        };
        self.message = Some(format!("{verb} task {}", task.id));
        self.reload(Some(task.id))
    }

    fn handle_normal_key(&mut self, code: KeyCode) -> Result<()> {
        self.message = None;
        let selected_id = self.selected().map(|t| t.id);

        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Down | KeyCode::Char('j') => self.table_state.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.table_state.select_previous(),
            KeyCode::Char('a') => self.mode = Mode::Form(Box::new(Form::new_task())),
            KeyCode::Char('/') => self.mode = Mode::Search,
            KeyCode::Char('e') => {
                if let Some(task) = self.selected() {
                    self.mode = Mode::Form(Box::new(Form::edit(task.clone())));
                }
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if let Some(task) = self.selected() {
                    let (id, done) = (task.id, !task.done);
                    self.db.set_done(id, done)?;
                    self.reload(Some(id))?;
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(id) = selected_id {
                    self.db.remove(id)?;
                    self.message = Some(format!("Removed task {id} (u to undo)"));
                    self.reload(None)?;
                }
            }
            KeyCode::Char('u') => {
                self.message = Some(match self.db.undo()? {
                    Some(what) => format!("Undid {what}"),
                    None => "Nothing to undo".to_string(),
                });
                self.reload(selected_id)?;
            }
            KeyCode::Char('s') => {
                self.filter.sort = self.filter.sort.next();
                self.reload(selected_id)?;
            }
            KeyCode::Char('h') => {
                self.filter.pending_only = !self.filter.pending_only;
                self.reload(selected_id)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [main, footer] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());

        let today = task::today();
        let header = Row::new(["ID", "", "Title", "Priority", "Due", "Tags"]).bold();
        let rows = self.tasks.iter().map(|t| task_row(t, today));
        let widths = [
            Constraint::Length(4),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(20),
        ];
        let mut title = format!(" {} · sort: {}", self.db.list_name(), self.filter.sort.label());
        if self.filter.pending_only {
            title.push_str(" · hiding done");
        }
        if let Some(search) = &self.filter.search {
            title.push_str(&format!(" · search: \"{search}\""));
        }
        title.push(' ');
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::bordered().title(title))
            .row_highlight_style(Style::new().reversed())
            .highlight_symbol("› ");
        frame.render_stateful_widget(table, main, &mut self.table_state);

        let footer_line = match (&self.mode, &self.message) {
            (Mode::Search, _) => {
                let text = self.filter.search.as_deref().unwrap_or("");
                Line::from(format!("Search: {text}█  (Enter keep · Esc clear)"))
            }
            (_, Some(message)) => Line::from(message.as_str()),
            (_, None) => Line::from(HELP).dim(),
        };
        frame.render_widget(Paragraph::new(footer_line), footer);

        if let Mode::Form(form) = &self.mode {
            form.draw(frame);
        }
    }
}

fn task_row(task: &Task, today: NaiveDate) -> Row<'static> {
    let priority_color = match task.priority {
        Priority::High => Color::Red,
        Priority::Medium => Color::Yellow,
        Priority::Low => Color::Green,
        Priority::None => Color::Reset,
    };
    let due_style = match task.due_status(today) {
        Some(DueStatus::Overdue) => Style::new().red().bold(),
        Some(DueStatus::Today) => Style::new().yellow(),
        _ => Style::new(),
    };

    let row = Row::new(vec![
        Cell::from(task.id.to_string()),
        Cell::from(if task.done { "✓" } else { "" }).green(),
        Cell::from(task.title.clone()),
        Cell::from(task.priority.label()).fg(priority_color),
        Cell::from(task.due_label()).style(due_style),
        Cell::from(task.tags.join(", ")).cyan(),
    ]);
    if task.done {
        row.style(Style::new().dark_gray().add_modifier(Modifier::CROSSED_OUT))
    } else {
        row
    }
}

// ---------------------------------------------------------------------------
// Add / edit form
// ---------------------------------------------------------------------------

/// Label and input hint for each form field, in display order.
const FIELDS: [(&str, &str); 5] = [
    ("Title", "What needs to be done"),
    ("Due", "YYYY-MM-DD, today, tomorrow or a weekday; empty for none"),
    ("Priority", "low, medium or high; empty for none"),
    ("Tags", "Comma separated, e.g. work, urgent"),
    ("Repeat", "day, week, 2 weeks, monthly, ...; empty for none"),
];

struct Form {
    /// `None` when adding a new task, otherwise the task being edited.
    editing: Option<Task>,
    /// The text in each field, in the same order as `FIELDS`.
    fields: [String; 5],
    /// Index of the field that receives typed keys.
    focus: usize,
    error: Option<String>,
}

/// The form's fields after checking and converting the typed text.
struct FormValues {
    title: String,
    due: Option<NaiveDate>,
    priority: Priority,
    tags: Vec<String>,
    repeat: Option<Repeat>,
}

impl Form {
    fn new_task() -> Form {
        Form {
            editing: None,
            fields: Default::default(),
            focus: 0,
            error: None,
        }
    }

    fn edit(task: Task) -> Form {
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

    fn parse(&self) -> Result<FormValues, String> {
        let [title, due, priority, tags, repeat] = &self.fields;

        let title = title.trim();
        if title.is_empty() {
            return Err("Title can't be empty".to_string());
        }
        let due = match due.trim() {
            "" => None,
            text => Some(task::parse_due(text)?),
        };
        let priority = match priority.trim() {
            "" => Priority::None,
            text => Priority::from_str(text, true)
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
            text => Some(task::parse_repeat(text)?),
        };

        Ok(FormValues { title: title.to_string(), due, priority, tags, repeat })
    }

    fn draw(&self, frame: &mut Frame) {
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
