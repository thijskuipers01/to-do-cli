use anyhow::Result;
use chrono::NaiveDate;
use clap::ValueEnum;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState};
use ratatui::{DefaultTerminal, Frame};

use crate::reminders::Backend;
use crate::store::{Filter, Store};
use crate::task::{self, DueStatus, Priority, Repeat, Task};

const HELP: &str =
    "a add · e edit · space done · d delete · u undo · / search · s sort · h hide done · q quit";

/// Runs the interactive UI. The terminal is restored afterwards, even when
/// the app stops with an error.
pub fn run<B: Backend>(db: &mut Store<B>) -> Result<()> {
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

struct App<'a, B: Backend> {
    db: &'a mut Store<B>,
    filter: Filter,
    tasks: Vec<Task>,
    table_state: TableState,
    mode: Mode,
    /// Feedback shown in the footer after an action, e.g. "Removed task 3".
    message: Option<String>,
    quit: bool,
}

impl<'a, B: Backend> App<'a, B> {
    fn new(db: &'a mut Store<B>) -> Result<Self> {
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

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::task::RepeatUnit;
    use crate::testing::{self, FakeList};

    fn press(app: &mut App<FakeList>, code: KeyCode) {
        app.handle_key(code).unwrap();
    }

    fn type_text(app: &mut App<FakeList>, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn titles(app: &App<FakeList>) -> Vec<String> {
        app.tasks.iter().map(|t| t.title.clone()).collect()
    }

    /// Everything on screen as one string, row after row.
    fn screen(app: &mut App<FakeList>) -> String {
        let mut terminal = Terminal::new(TestBackend::new(110, 20)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect()
    }

    fn form(fields: [&str; 5]) -> Form {
        Form { fields: fields.map(String::from), ..Form::new_task() }
    }

    // --- Form ---------------------------------------------------------------

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
    fn form_parses_valid_input() {
        let values = form([" Milk ", "2026-10-20", "HIGH", "home, , shop ", "2 weeks"]).parse().unwrap();
        assert_eq!(values.title, "Milk");
        assert_eq!(values.due, Some("2026-10-20".parse().unwrap()));
        assert_eq!(values.priority, Priority::High);
        assert_eq!(values.tags, ["home", "shop"]);
        assert_eq!(values.repeat, Some(Repeat { unit: RepeatUnit::Week, interval: 2 }));
    }

    #[test]
    fn form_treats_empty_optional_fields_as_unset() {
        let values = form(["Milk", "", "", "", ""]).parse().unwrap();
        assert_eq!((values.due, values.priority, values.repeat), (None, Priority::None, None));
        assert!(values.tags.is_empty());
    }

    #[test]
    fn form_explains_invalid_input() {
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

    // --- App ----------------------------------------------------------------

    #[test]
    fn adding_through_the_form_saves_every_field() {
        let mut db = testing::store();
        let mut app = App::new(&mut db).unwrap();
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "Milk");
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "2026-10-20");
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "high");
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "home, shop");
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "week");
        press(&mut app, KeyCode::Enter);

        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.message.as_deref(), Some("Added task 1"));
        let task = app.selected().unwrap().clone();
        assert_eq!(task.title, "Milk");
        assert_eq!(task.due_label(), "2026-10-20 ↻");
        assert_eq!((task.priority, task.tags), (Priority::High, vec!["home".into(), "shop".into()]));
    }

    #[test]
    fn form_keys_move_focus_and_edit_text() {
        let mut db = testing::store();
        let mut app = App::new(&mut db).unwrap();
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "Mikl");
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        type_text(&mut app, "lk");
        press(&mut app, KeyCode::BackTab);
        let Mode::Form(form) = &app.mode else { panic!("form closed") };
        assert_eq!((form.fields[0].as_str(), form.focus), ("Milk", 4), "BackTab wraps to the last field");
        press(&mut app, KeyCode::Down);
        let Mode::Form(form) = &app.mode else { panic!("form closed") };
        assert_eq!(form.focus, 0);
    }

    #[test]
    fn invalid_form_stays_open_with_an_error() {
        let mut db = testing::store();
        let mut app = App::new(&mut db).unwrap();
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Enter);
        let Mode::Form(form) = &app.mode else { panic!("form closed") };
        assert_eq!(form.error.as_deref(), Some("Title can't be empty"));
        assert!(app.tasks.is_empty());
    }

    #[test]
    fn escape_cancels_the_form() {
        let mut db = testing::store();
        let mut app = App::new(&mut db).unwrap();
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "Never mind");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.tasks.is_empty());
    }

    #[test]
    fn editing_changes_the_selected_task() {
        let mut db = testing::store();
        db.add(&Task { notes: "keep".into(), ..Task::new("Old") }).unwrap();
        let mut app = App::new(&mut db).unwrap();
        press(&mut app, KeyCode::Char('e'));
        type_text(&mut app, " name");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.message.as_deref(), Some("Updated task 1"));
        let task = app.selected().unwrap();
        assert_eq!((task.title.as_str(), task.notes.as_str()), ("Old name", "keep"));
    }

    #[test]
    fn space_toggles_done() {
        let mut db = testing::store();
        db.add(&Task::new("a")).unwrap();
        let mut app = App::new(&mut db).unwrap();
        press(&mut app, KeyCode::Char(' '));
        assert!(app.selected().unwrap().done);
        press(&mut app, KeyCode::Enter);
        assert!(!app.selected().unwrap().done);
    }

    #[test]
    fn delete_then_undo() {
        let mut db = testing::store();
        db.add(&Task::new("a")).unwrap();
        let mut app = App::new(&mut db).unwrap();
        press(&mut app, KeyCode::Char('d'));
        assert!(app.tasks.is_empty());
        assert_eq!(app.message.as_deref(), Some("Removed task 1 (u to undo)"));
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(titles(&app), ["a"]);
        assert_eq!(app.message.as_deref(), Some("Undid remove of task 1"));
    }

    #[test]
    fn undo_with_nothing_to_undo_says_so() {
        let mut db = testing::store();
        let mut app = App::new(&mut db).unwrap();
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.message.as_deref(), Some("Nothing to undo"));
    }

    #[test]
    fn arrow_keys_and_j_k_move_the_selection() {
        let mut db = testing::store();
        for title in ["a", "b", "c"] {
            db.add(&Task::new(title)).unwrap();
        }
        let mut app = App::new(&mut db).unwrap();
        assert_eq!(app.table_state.selected(), Some(0));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected().unwrap().title, "c");
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected().unwrap().title, "a");
    }

    #[test]
    fn search_filters_while_typing() {
        let mut db = testing::store();
        for title in ["Buy milk", "Call mom", "Milk the cow"] {
            db.add(&Task::new(title)).unwrap();
        }
        let mut app = App::new(&mut db).unwrap();
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "milkx");
        assert!(app.tasks.is_empty());
        press(&mut app, KeyCode::Backspace);
        assert_eq!(titles(&app), ["Buy milk", "Milk the cow"]);

        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.tasks.len(), 2, "Enter keeps the search");

        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.tasks.len(), 3, "Esc clears the search");
        assert!(app.filter.search.is_none());
    }

    #[test]
    fn s_cycles_sorting_and_h_hides_done() {
        let mut db = testing::store();
        db.add(&Task { priority: Priority::Low, ..Task::new("low") }).unwrap();
        db.add(&Task { priority: Priority::High, ..Task::new("high") }).unwrap();
        let done = db.add(&Task::new("finished")).unwrap();
        db.set_done(done.id, true).unwrap();
        let mut app = App::new(&mut db).unwrap();

        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.filter.sort, crate::store::SortKey::Priority);
        assert_eq!(titles(&app), ["high", "low", "finished"]);

        press(&mut app, KeyCode::Char('h'));
        assert_eq!(titles(&app), ["high", "low"]);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.tasks.len(), 3);
    }

    #[test]
    fn q_and_escape_quit() {
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            let mut db = testing::store();
            let mut app = App::new(&mut db).unwrap();
            press(&mut app, key);
            assert!(app.quit);
        }
    }

    #[test]
    fn screen_shows_list_tasks_and_help() {
        let mut db = testing::store();
        let task = db.add(&Task { tags: vec!["work".into()], ..Task::new("Write report") }).unwrap();
        db.set_done(task.id, true).unwrap();
        let mut app = App::new(&mut db).unwrap();
        let text = screen(&mut app);
        for expected in ["Todo · sort: id", "Write report", "work", "✓", "a add", "q quit"] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
    }

    #[test]
    fn screen_shows_search_and_form() {
        let mut db = testing::store();
        let mut app = App::new(&mut db).unwrap();
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "abc");
        let text = screen(&mut app);
        assert!(text.contains("Search: abc") && text.contains("search: \"abc\""));

        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Enter);
        let text = screen(&mut app);
        for expected in ["New task", "Title", "Repeat", "Title can't be empty"] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
    }
}
