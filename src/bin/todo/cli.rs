//! The command-line options, and running each command.

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use chrono::{Days, NaiveDate};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use todo::model::{self, Due, Priority, Repeat, Task};
use todo::{Backend, Error, ErrorKind, Filter, ListSummary, SortKey, Store, json};

use crate::{display, notify, tui};

/// Where the local state lives; see [`todo::store::state_path`].
pub fn state_path() -> Result<PathBuf> {
    todo::store::state_path()
}

// ---------------------------------------------------------------------------
// Errors as JSON
// ---------------------------------------------------------------------------

/// Whether `--json` is among the arguments (before a `--`, after which
/// everything is a value).
pub fn wants_json(args: impl IntoIterator<Item = OsString>) -> bool {
    args.into_iter().take_while(|arg| arg != "--").any(|arg| arg == "--json")
}

/// The exit code for each kind of error.
pub fn exit_code(kind: ErrorKind) -> u8 {
    match kind {
        ErrorKind::Other => 1,
        ErrorKind::InvalidInput => 2,
        ErrorKind::NotFound => 3,
        ErrorKind::PermissionDenied => 4,
    }
}

/// Prints the error as JSON on stderr and returns the matching exit code.
pub fn report_json_error(kind: ErrorKind, message: &str) -> ExitCode {
    eprintln!("{}", json::error(kind, message));
    ExitCode::from(exit_code(kind))
}

/// The first line of a clap error without the "error: " in front, e.g.
/// "unexpected argument '--foo' found".
pub fn clap_message(error: &clap::Error) -> String {
    let text = error.render().to_string();
    let line = text.lines().find(|line| !line.trim().is_empty()).unwrap_or_default();
    line.trim_start_matches("error: ").trim().to_string()
}

// ---------------------------------------------------------------------------
// Command-line interface (clap builds the parser from these definitions)
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "todo", version, about = "A to-do manager backed by Apple Reminders")]
pub struct Cli {
    /// Reminders list to use (created if it doesn't exist)
    #[arg(short, long, global = true, env = "TODO_LIST", default_value = "Todo")]
    pub list: String,

    /// Print JSON instead of text, for scripts and AI assistants
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Add a new task
    Add {
        /// The task description
        title: String,
        /// Due date: YYYY-MM-DD, "today", "tomorrow" or a weekday ("friday"),
        /// optionally with a time: "friday 15:00", "tomorrow 3pm", "2026-10-23T15:00"
        #[arg(short, long, value_parser = model::parse_due_time)]
        due: Option<Due>,
        /// How urgent the task is
        #[arg(short, long, value_enum, default_value_t = Priority::None)]
        priority: Priority,
        /// Tag to attach; repeat for more (-t work -t urgent)
        #[arg(short, long = "tag")]
        tags: Vec<String>,
        /// Repeat the task: "day", "week", "2 weeks", "monthly", ... (starts today without --due)
        #[arg(short, long, value_parser = model::parse_repeat)]
        every: Option<Repeat>,
        /// Notes to store with the task
        #[arg(long)]
        notes: Option<String>,
    },
    /// Change the title, due date, priority, tags, notes or repeat of a task
    Edit {
        /// ID of the task
        id: i64,
        /// New title
        #[arg(long)]
        title: Option<String>,
        /// New due date, optionally with a time (same formats as `add`)
        #[arg(short, long, value_parser = model::parse_due_time, conflicts_with = "no_due")]
        due: Option<Due>,
        /// Remove the due date
        #[arg(long)]
        no_due: bool,
        /// New priority
        #[arg(short, long, value_enum)]
        priority: Option<Priority>,
        /// Tag to add; repeat for more
        #[arg(short, long = "tag")]
        tags: Vec<String>,
        /// Tag to remove; repeat for more
        #[arg(long = "untag")]
        untags: Vec<String>,
        /// New repeat interval (same formats as `add`)
        #[arg(short, long, value_parser = model::parse_repeat, conflicts_with = "no_repeat")]
        every: Option<Repeat>,
        /// Stop repeating the task
        #[arg(long)]
        no_repeat: bool,
        /// Replace the notes (tags are kept)
        #[arg(long, conflicts_with_all = ["no_notes", "append_notes"])]
        notes: Option<String>,
        /// Remove the notes (tags are kept)
        #[arg(long, conflicts_with = "append_notes")]
        no_notes: bool,
        /// Add a line to the end of the notes
        #[arg(long)]
        append_notes: Option<String>,
        /// Show the task as it would be after the change, without saving it
        #[arg(long)]
        dry_run: bool,
    },
    /// List tasks
    List(ListArgs),
    /// Show unfinished tasks that are overdue or due today
    Today,
    /// Show all details of a task, including its notes
    Show {
        /// ID of the task
        id: i64,
    },
    /// Mark one or more tasks as done
    Done {
        /// IDs of the tasks; nothing changes if one doesn't exist
        #[arg(required = true)]
        ids: Vec<i64>,
        /// Show what would happen, without changing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove one or more tasks
    Remove {
        /// IDs of the tasks; nothing changes if one doesn't exist
        #[arg(required = true)]
        ids: Vec<i64>,
        /// Show what would happen, without changing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Show all Reminders lists and how many unfinished tasks each has
    Lists,
    /// Show the most recent changes, newest first
    History {
        /// How many changes to show
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
    },
    /// Undo the last change (can be repeated)
    Undo,
    /// Open the interactive terminal UI
    Tui,
    /// Show a desktop notification for unfinished tasks due tomorrow
    Notify,
    /// Print a shell completion script (e.g. `todo completions zsh`)
    Completions {
        shell: Shell,
    },
}

/// The options of `todo list`.
#[derive(Args)]
pub struct ListArgs {
    /// Only show tasks with this tag
    #[arg(short, long)]
    pub tag: Option<String>,
    /// Only show tasks with this priority
    #[arg(short, long, value_enum)]
    pub priority: Option<Priority>,
    /// Only show tasks whose title or notes contain this text
    #[arg(long)]
    pub search: Option<String>,
    /// Hide completed tasks
    #[arg(long)]
    pub pending: bool,
    /// Only tasks due on or before this date (same formats as `add`, without a time)
    #[arg(long, value_parser = model::parse_due)]
    pub due_before: Option<NaiveDate>,
    /// Only tasks due on or after this date
    #[arg(long, value_parser = model::parse_due)]
    pub due_after: Option<NaiveDate>,
    /// Only unfinished tasks due before today
    #[arg(long)]
    pub overdue: bool,
    /// Only tasks without a due date
    #[arg(long, conflicts_with_all = ["due_before", "due_after", "overdue"])]
    pub no_due: bool,
    /// Order of the list
    #[arg(short, long, value_enum, default_value_t = SortKey::Id)]
    pub sort: SortKey,
}

impl ListArgs {
    /// The filter these options describe; `today` decides what's overdue.
    pub fn filter(self, today: NaiveDate) -> Filter {
        let mut filter = Filter {
            tag: self.tag,
            priority: self.priority,
            search: self.search,
            pending_only: self.pending,
            due_by: self.due_before,
            due_after: self.due_after,
            no_due: self.no_due,
            sort: self.sort,
        };
        if self.overdue {
            let yesterday = today - Days::new(1);
            filter.pending_only = true;
            filter.due_by = Some(filter.due_by.map_or(yesterday, |by| by.min(yesterday)));
        }
        filter
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

/// Prints the lists from `todo lists`.
pub fn print_lists(lists: &[ListSummary], json: bool) -> Result<()> {
    if json {
        display::print_lists_json(lists)?;
    } else {
        display::print_lists(lists);
    }
    Ok(())
}

pub fn print_completions(shell: Shell, out: &mut impl Write) {
    clap_complete::generate(shell, &mut Cli::command(), "todo", out);
}

/// Runs one command. Generic over the reminders list so the tests can run
/// commands against an in-memory list instead of the real Reminders app.
/// With `json`, commands print JSON: the tasks for `list`, `today` and `show`,
/// and the changed task for `add`, `edit`, `done` and `remove` (an array
/// when `done` or `remove` get several IDs).
pub fn run<B: Backend>(command: Command, json: bool, db: &mut Store<B>) -> Result<()> {
    match command {
        Command::Add { title, due, priority, tags, every, notes } => {
            let notes = notes.unwrap_or_default();
            let task = db.add(&Task {
                due: due.map(|d| d.date),
                due_time: due.and_then(|d| d.time),
                priority,
                tags,
                notes,
                repeat: every,
                ..Task::new(&title)
            })?;
            if json {
                display::print_task_json(&task)?;
            } else {
                println!("Added task {}: {}", task.id, task.title);
            }
        }
        Command::Edit {
            id,
            title,
            due,
            no_due,
            priority,
            tags,
            untags,
            every,
            no_repeat,
            notes,
            no_notes,
            append_notes,
            dry_run,
        } => {
            if title.is_none()
                && due.is_none()
                && !no_due
                && priority.is_none()
                && tags.is_empty()
                && untags.is_empty()
                && every.is_none()
                && !no_repeat
                && notes.is_none()
                && !no_notes
                && append_notes.is_none()
            {
                return Err(Error::invalid_input("nothing to change; see `todo edit --help` for the options").into());
            }

            let mut task = db.get(id)?.ok_or_else(|| Error::no_task(id))?;
            if let Some(title) = title {
                task.title = title;
            }
            if no_due {
                (task.due, task.due_time) = (None, None);
            } else if let Some(due) = due {
                (task.due, task.due_time) = (Some(due.date), due.time);
            }
            if let Some(priority) = priority {
                task.priority = priority;
            }
            task.tags.retain(|tag| !untags.contains(tag));
            task.tags.extend(tags);
            if no_repeat {
                task.repeat = None;
            } else if every.is_some() {
                task.repeat = every;
            }
            if no_notes {
                task.notes.clear();
            } else if let Some(notes) = notes {
                task.notes = notes;
            }
            if let Some(line) = append_notes {
                task.notes = append_line(&task.notes, &line);
            }

            if dry_run {
                let task = db.preview_update(&task)?;
                if json {
                    display::print_task_json(&task)?;
                } else {
                    println!("Would update task {id} to:");
                    display::print_task_details(&task);
                }
                return Ok(());
            }
            let task = db.update(&task)?;
            if json {
                display::print_task_json(&task)?;
            } else {
                println!("Updated task {id}: {}", task.title);
            }
        }
        Command::List(args) => {
            let filter = args.filter(model::today());
            let tasks = db.list(&filter)?;
            if json {
                display::print_tasks_json(&tasks)?;
            } else {
                display::print_tasks(&tasks);
            }
        }
        Command::Today => {
            let filter = Filter {
                pending_only: true,
                due_by: Some(model::today()),
                sort: SortKey::Due,
                ..Filter::default()
            };
            let tasks = db.list(&filter)?;
            if json {
                display::print_tasks_json(&tasks)?;
            } else if tasks.is_empty() {
                println!("Nothing overdue or due today.");
            } else {
                display::print_tasks(&tasks);
            }
        }
        Command::Show { id } => {
            let task = db.get(id)?.ok_or_else(|| Error::no_task(id))?;
            if json {
                display::print_task_json(&task)?;
            } else {
                display::print_task_details(&task);
            }
        }
        Command::Done { ids, dry_run } => {
            let tasks = match dry_run {
                true => db.preview_done(&ids)?,
                false => db.set_done_many(&ids, true)?,
            };
            if json {
                return print_changed_json(&ids, &tasks);
            }
            let verb = if dry_run { "Would complete" } else { "Completed" };
            for task in tasks {
                match (task.done, task.due) {
                    (false, Some(_)) => println!(
                        "{verb} task {}: {} (repeats, next due {})",
                        task.id,
                        task.title,
                        task.due_text()
                    ),
                    _ => println!("{verb} task {}: {}", task.id, task.title),
                }
            }
        }
        Command::Remove { ids, dry_run } => {
            let tasks = match dry_run {
                true => db.require_all(&ids)?,
                false => db.remove_many(&ids)?,
            };
            if json {
                return print_changed_json(&ids, &tasks);
            }
            let verb = if dry_run { "Would remove" } else { "Removed" };
            for task in tasks {
                println!("{verb} task {}: {}", task.id, task.title);
            }
        }
        Command::Lists => print_lists(&db.backend().lists()?, json)?,
        Command::History { limit } => {
            let changes = db.history(limit)?;
            if json {
                display::print_history_json(&changes)?;
            } else {
                display::print_history(&changes);
            }
        }
        Command::Undo => match (db.undo()?, json) {
            (what, true) => println!("{}", json::undone(what.as_deref())),
            (Some(what), false) => println!("Undid {what}"),
            (None, false) => println!("Nothing to undo"),
        },
        Command::Tui => tui::run(db)?,
        Command::Notify => notify::due_tomorrow(db)?,
        Command::Completions { .. } => unreachable!("handled above"),
    }

    Ok(())
}

/// The tasks changed by `done` or `remove`: one object for a single ID (as
/// before several IDs were possible), otherwise an array.
fn print_changed_json(ids: &[i64], tasks: &[Task]) -> Result<()> {
    match (ids, tasks) {
        ([_], [task]) => display::print_task_json(task)?,
        _ => display::print_tasks_json(tasks)?,
    }
    Ok(())
}

/// `notes` with `line` added as a new last line.
fn append_line(notes: &str, line: &str) -> String {
    match notes.trim_end() {
        "" => line.to_string(),
        notes => format!("{notes}\n{line}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo::RepeatUnit;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("todo").chain(args.iter().copied()))
    }

    fn command(args: &[&str]) -> Command {
        parse(args).unwrap().command
    }

    // --- Parsing the command line -------------------------------------------

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn add_with_every_option() {
        let Command::Add { title, due, priority, tags, every, notes } = command(&[
            "add", "Milk", "-d", "2026-10-20", "-p", "high", "-t", "home", "-t", "shop", "--every", "2 weeks",
            "--notes", "Oat milk",
        ]) else {
            panic!("not an add command");
        };
        assert_eq!((title.as_str(), notes.as_deref()), ("Milk", Some("Oat milk")));
        assert_eq!(due, Some(Due { date: "2026-10-20".parse().unwrap(), time: None }));
        assert_eq!((priority, tags), (Priority::High, vec!["home".into(), "shop".into()]));
        assert_eq!(every, Some(Repeat { unit: RepeatUnit::Week, interval: 2 }));
    }

    #[test]
    fn add_defaults() {
        let Command::Add { due, priority, tags, every, .. } = command(&["add", "Milk"]) else {
            panic!("not an add command");
        };
        assert_eq!((due, priority, every), (None, Priority::None, None));
        assert!(tags.is_empty());
    }

    #[test]
    fn list_option_works_before_and_after_the_command() {
        assert_eq!(parse(&["--list", "Shopping", "list"]).unwrap().list, "Shopping");
        assert_eq!(parse(&["list", "-l", "Shopping"]).unwrap().list, "Shopping");
    }

    #[test]
    fn list_filters() {
        let Command::List(ListArgs { tag, priority, search, pending, sort, .. }) =
            command(&["list", "-t", "work", "-p", "low", "--search", "mail", "--pending", "-s", "due"])
        else {
            panic!("not a list command");
        };
        assert_eq!(tag.as_deref(), Some("work"));
        assert_eq!(priority, Some(Priority::Low));
        assert_eq!(search.as_deref(), Some("mail"));
        assert!(pending);
        assert_eq!(sort, SortKey::Due);
    }

    #[test]
    fn contradicting_or_invalid_options_are_rejected() {
        for args in [
            &["edit", "1", "--due", "today", "--no-due"][..],
            &["edit", "1", "--every", "week", "--no-repeat"],
            &["edit", "1", "--notes", "a", "--no-notes"],
            &["edit", "1", "--notes", "a", "--append-notes", "b"],
            &["edit", "1", "--no-notes", "--append-notes", "b"],
            &["add", "x", "--due", "someday"],
            &["add", "x", "--priority", "urgent"],
            &["add", "x", "--every", "fortnight"],
            &["done", "one"],
            &["list", "--no-due", "--overdue"],
            &["list", "--no-due", "--due-before", "today"],
            &["list", "--due-after", "friday 15:00"],
            &["done"],
            &["remove"],
            &["completions", "notashell"],
        ] {
            assert!(parse(args).is_err(), "should reject {args:?}");
        }
    }

    #[test]
    fn completion_scripts_cover_the_commands() {
        for shell in [Shell::Zsh, Shell::Bash, Shell::Fish] {
            let mut out = Vec::new();
            print_completions(shell, &mut out);
            let script = String::from_utf8(out).unwrap();
            // Fish writes long options as `-l every`, so look for the bare name.
            for word in ["todo", "add", "today", "completions", "every"] {
                assert!(script.contains(word), "{shell}: missing {word:?}");
            }
        }
    }

    #[test]
    fn state_path_defaults_to_the_data_directory() {
        use std::env;
        let path = state_path().unwrap();
        match env::var_os("TODO_STATE") {
            Some(custom) => assert_eq!(path, PathBuf::from(custom)),
            None => assert!(path.ends_with("todo/state.db"), "{}", path.display()),
        }
    }

    // --- Running commands against the fake list ------------------------------

    #[test]
    fn every_command_runs() {
        let mut db = crate::test_store();
        let commands = [
            vec!["add", "Milk", "-d", "today", "-t", "shop"],
            vec!["add", "Plants", "--every", "week"],
            vec!["list"],
            vec!["list", "--search", "milk", "-s", "priority"],
            vec!["today"],
            vec!["show", "1"],
            vec!["edit", "1", "--title", "Oat milk", "-p", "low", "--untag", "shop", "-t", "store"],
            vec!["done", "1"],
            vec!["done", "2"],
            vec!["remove", "1"],
            vec!["history"],
            vec!["undo"],
            vec!["lists"],
        ];
        for args in commands {
            run(command(&args), false, &mut db).unwrap_or_else(|e| panic!("{args:?} failed: {e}"));
        }
        let milk = db.get(1).unwrap().unwrap();
        assert_eq!((milk.title.as_str(), milk.priority, milk.done), ("Oat milk", Priority::Low, true));
        assert_eq!(milk.tags, ["store"]);
    }

    #[test]
    fn every_command_runs_with_json() {
        let mut db = crate::test_store();
        for args in [
            vec!["add", "Milk", "--json"],
            vec!["--json", "list"],
            vec!["today", "--json"],
            vec!["show", "1", "--json"],
            vec!["edit", "1", "-p", "high", "--json"],
            vec!["done", "1", "--json"],
            vec!["remove", "1", "--json"],
            vec!["undo", "--json"],
            vec!["undo", "--json"],
            vec!["lists", "--json"],
            vec!["history", "-n", "3", "--json"],
        ] {
            let cli = parse(&args).unwrap();
            assert!(cli.json, "{args:?} should set --json");
            run(cli.command, cli.json, &mut db).unwrap_or_else(|e| panic!("{args:?} failed: {e}"));
        }
        // The two undos bring back the removed task, then reopen it.
        let task = db.get(1).unwrap().unwrap();
        assert_eq!((task.done, task.priority), (false, Priority::High));
    }

    #[test]
    fn edit_applies_only_the_given_changes() {
        let mut db = crate::test_store();
        run(command(&["add", "Milk", "-d", "2026-10-20", "-p", "high", "--every", "week"]), false, &mut db).unwrap();
        run(command(&["edit", "1", "--no-due", "--no-repeat"]), false, &mut db).unwrap();
        let task = db.get(1).unwrap().unwrap();
        assert_eq!((task.due, task.repeat, task.priority), (None, None, Priority::High));

        run(command(&["edit", "1", "--every", "monthly", "-d", "2026-11-01"]), false, &mut db).unwrap();
        let task = db.get(1).unwrap().unwrap();
        assert_eq!(task.repeat, Some(Repeat { unit: RepeatUnit::Month, interval: 1 }));
        assert_eq!(task.due, Some("2026-11-01".parse().unwrap()));
    }

    #[test]
    fn due_times_are_set_changed_and_cleared() {
        let mut db = crate::test_store();
        let due = |db: &Store<_>| {
            let task = db.get(1).unwrap().unwrap();
            (task.due.map(|d| d.to_string()), task.due_time.map(|t| t.to_string()))
        };
        run(command(&["add", "Call mum", "--due", "2026-10-23T15:00"]), false, &mut db).unwrap();
        assert_eq!(due(&db), (Some("2026-10-23".into()), Some("15:00:00".into())));
        run(command(&["edit", "1", "--due", "2026-10-24 9am"]), false, &mut db).unwrap();
        assert_eq!(due(&db), (Some("2026-10-24".into()), Some("09:00:00".into())));
        run(command(&["edit", "1", "--due", "2026-10-25"]), false, &mut db).unwrap();
        assert_eq!(due(&db), (Some("2026-10-25".into()), None), "a date alone makes it all-day");
        run(command(&["edit", "1", "--due", "2026-10-25 18:30"]), false, &mut db).unwrap();
        run(command(&["edit", "1", "--no-due"]), false, &mut db).unwrap();
        assert_eq!(due(&db), (None, None));
    }

    #[test]
    fn notes_can_be_set_replaced_appended_and_cleared_without_losing_tags() {
        let mut db = crate::test_store();
        let notes = |db: &Store<_>| db.get(1).unwrap().unwrap().notes;
        run(command(&["add", "Dentist", "-t", "health", "--notes", "Bring card"]), false, &mut db).unwrap();
        assert_eq!(notes(&db), "Bring card");
        assert_eq!(db.backend().data("memory-1").unwrap().notes, "Bring card\n#health");

        run(command(&["edit", "1", "--append-notes", "Booked by phone"]), false, &mut db).unwrap();
        assert_eq!(notes(&db), "Bring card\nBooked by phone");

        run(command(&["edit", "1", "--notes", "Ask about X"]), false, &mut db).unwrap();
        assert_eq!(notes(&db), "Ask about X");

        run(command(&["edit", "1", "--no-notes"]), false, &mut db).unwrap();
        assert_eq!(notes(&db), "");
        run(command(&["edit", "1", "--append-notes", "First line"]), false, &mut db).unwrap();
        assert_eq!(notes(&db), "First line");
        assert_eq!(db.get(1).unwrap().unwrap().tags, ["health"]);
        assert_eq!(db.backend().data("memory-1").unwrap().notes, "First line\n#health");
    }

    #[test]
    fn append_line_skips_the_newline_for_empty_notes() {
        assert_eq!(append_line("", "a"), "a");
        assert_eq!(append_line("a\n", "b"), "a\nb");
        assert_eq!(append_line("a", "b"), "a\nb");
    }

    #[test]
    fn errors_have_a_kind_and_an_exit_code() {
        let mut db = crate::test_store();
        let kind = |db: &mut Store<_>, args: &[&str]| ErrorKind::of(&run(command(args), true, db).unwrap_err());
        assert_eq!(kind(&mut db, &["edit", "1"]), ErrorKind::InvalidInput);
        assert_eq!(kind(&mut db, &["show", "7"]), ErrorKind::NotFound);
        assert_eq!(kind(&mut db, &["done", "7"]), ErrorKind::NotFound);
        assert_eq!(kind(&mut db, &["edit", "7", "-p", "low"]), ErrorKind::NotFound);

        let codes = [ErrorKind::Other, ErrorKind::InvalidInput, ErrorKind::NotFound, ErrorKind::PermissionDenied];
        assert_eq!(codes.map(exit_code), [1, 2, 3, 4]);
    }

    #[test]
    fn argument_errors_become_short_messages() {
        let message = |args: &[&str]| clap_message(&parse(args).err().unwrap());
        assert_eq!(message(&["add", "x", "--bogus"]), "unexpected argument '--bogus' found");
        assert!(message(&["add", "x", "--due", "someday"]).contains("expected a date (YYYY-MM-DD"));
        assert!(message(&["add", "x", "--priority", "urgent"]).contains("urgent"));
    }

    #[test]
    fn wants_json_stops_at_double_dash() {
        let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(wants_json(args(&["list", "--json"])));
        assert!(!wants_json(args(&["list"])));
        assert!(!wants_json(args(&["add", "--", "--json"])));
    }

    #[test]
    fn done_and_remove_take_several_ids_and_undo_them_together() {
        let mut db = crate::test_store();
        for title in ["a", "b", "c"] {
            run(command(&["add", title]), false, &mut db).unwrap();
        }
        let Command::Done { ids, .. } = command(&["done", "1", "3"]) else { panic!("not done") };
        assert_eq!(ids, [1, 3]);
        run(command(&["done", "1", "3", "--json"]), true, &mut db).unwrap();
        let done: Vec<i64> = db.list(&Filter::default()).unwrap().iter().filter(|t| t.done).map(|t| t.id).collect();
        assert_eq!(done, [1, 3]);

        let error = run(command(&["remove", "2", "9"]), false, &mut db).unwrap_err();
        assert_eq!(error.to_string(), "no task with id 9");
        assert!(db.get(2).unwrap().is_some(), "nothing removed");

        run(command(&["remove", "1", "2"]), false, &mut db).unwrap();
        run(command(&["undo"]), false, &mut db).unwrap();
        assert_eq!(db.list(&Filter::default()).unwrap().len(), 3);
        run(command(&["undo"]), false, &mut db).unwrap();
        assert!(db.list(&Filter::default()).unwrap().iter().all(|t| !t.done));
    }

    #[test]
    fn list_filters_by_due_date() {
        let mut db = crate::test_store();
        let today = model::today();
        let day = |offset: i64| (today + chrono::TimeDelta::days(offset)).to_string();
        for (title, due) in [("Overdue", day(-2)), ("Today", day(0)), ("Next week", day(7)), ("Done late", day(-1))] {
            run(command(&["add", title, "--due", &due]), false, &mut db).unwrap();
        }
        run(command(&["add", "Someday"]), false, &mut db).unwrap();
        run(command(&["done", "4"]), false, &mut db).unwrap();

        let ids = |db: &mut Store<_>, args: &[&str]| {
            run(command(args), true, db).unwrap();
            let Command::List(list) = command(args) else { panic!("not a list command") };
            db.list(&list.filter(today)).unwrap().iter().map(|t| t.id).collect::<Vec<_>>()
        };
        assert_eq!(ids(&mut db, &["list", "--overdue"]), [1]);
        assert_eq!(ids(&mut db, &["list", "--due-before", "today"]), [1, 2, 4]);
        assert_eq!(ids(&mut db, &["list", "--due-after", "today", "--due-before", &day(7)]), [2, 3]);
        assert_eq!(ids(&mut db, &["list", "--no-due"]), [5]);
        assert_eq!(ids(&mut db, &["list", "--overdue", "--due-before", &day(-3)]), Vec::<i64>::new());
    }

    #[test]
    fn dry_runs_never_change_tasks_or_the_undo_history() {
        let mut db = crate::test_store();
        run(command(&["add", "Dentist", "-d", "2026-10-23", "--every", "month"]), false, &mut db).unwrap();
        run(command(&["add", "Milk"]), false, &mut db).unwrap();
        let reminders = |db: &Store<todo::MemoryList>| {
            db.backend().all().unwrap().into_iter().map(|r| r.data).collect::<Vec<_>>()
        };
        let before = (reminders(&db), db.history(100).unwrap());

        for json in [false, true] {
            for args in [
                &["edit", "1", "--due", "monday 9:00", "--title", "X", "--dry-run"][..],
                &["done", "1", "2", "--dry-run"],
                &["done", "1", "--dry-run"],
                &["remove", "1", "2", "--dry-run"],
            ] {
                run(command(args), json, &mut db).unwrap_or_else(|e| panic!("{args:?} failed: {e}"));
            }
        }
        assert_eq!((reminders(&db), db.history(100).unwrap()), before);

        // A dry run still checks its input.
        let error = run(command(&["remove", "1", "9", "--dry-run"]), false, &mut db).unwrap_err();
        assert_eq!(todo::ErrorKind::of(&error), todo::ErrorKind::NotFound);
        assert!(run(command(&["edit", "1", "--dry-run"]), false, &mut db).is_err(), "nothing to change");
    }

    #[test]
    fn commands_report_problems() {
        let mut db = crate::test_store();
        let error = |db: &mut Store<_>, args: &[&str]| run(command(args), false, db).unwrap_err().to_string();
        assert!(error(&mut db, &["edit", "1"]).starts_with("nothing to change"));
        assert_eq!(error(&mut db, &["edit", "1", "-p", "low"]), "no task with id 1");
        assert_eq!(error(&mut db, &["show", "1"]), "no task with id 1");
        assert_eq!(error(&mut db, &["done", "1"]), "no task with id 1");
        assert_eq!(error(&mut db, &["remove", "1"]), "no task with id 1");
        run(command(&["undo"]), false, &mut db).unwrap(); // nothing to undo is not an error
    }
}
