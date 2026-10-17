//! The command-line options, and running each command.

use std::env;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use todo::model::{self, Priority, Repeat, Task};
use todo::{Backend, Filter, SortKey, Store};

use crate::{display, notify, tui};

/// Where the local state (short IDs and undo history) lives: `$TODO_STATE` if
/// set, otherwise `$XDG_DATA_HOME/todo/state.db`, which defaults to
/// `~/.local/share/todo/state.db`.
pub fn state_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("TODO_STATE") {
        return Ok(PathBuf::from(path));
    }
    let data_dir = match env::var_os("XDG_DATA_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".local/share"),
    };
    Ok(data_dir.join("todo").join("state.db"))
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
        /// Due date: YYYY-MM-DD, "today", "tomorrow" or a weekday ("friday")
        #[arg(short, long, value_parser = model::parse_due)]
        due: Option<NaiveDate>,
        /// How urgent the task is
        #[arg(short, long, value_enum, default_value_t = Priority::None)]
        priority: Priority,
        /// Tag to attach; repeat for more (-t work -t urgent)
        #[arg(short, long = "tag")]
        tags: Vec<String>,
        /// Repeat the task: "day", "week", "2 weeks", "monthly", ... (starts today without --due)
        #[arg(short, long, value_parser = model::parse_repeat)]
        every: Option<Repeat>,
    },
    /// Change the title, due date, priority, tags or repeat of a task
    Edit {
        /// ID of the task
        id: i64,
        /// New title
        #[arg(long)]
        title: Option<String>,
        /// New due date (same formats as `add`)
        #[arg(short, long, value_parser = model::parse_due, conflicts_with = "no_due")]
        due: Option<NaiveDate>,
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
    },
    /// List tasks
    List {
        /// Only show tasks with this tag
        #[arg(short, long)]
        tag: Option<String>,
        /// Only show tasks with this priority
        #[arg(short, long, value_enum)]
        priority: Option<Priority>,
        /// Only show tasks whose title or notes contain this text
        #[arg(long)]
        search: Option<String>,
        /// Hide completed tasks
        #[arg(long)]
        pending: bool,
        /// Order of the list
        #[arg(short, long, value_enum, default_value_t = SortKey::Id)]
        sort: SortKey,
    },
    /// Show unfinished tasks that are overdue or due today
    Today,
    /// Show all details of a task, including its notes
    Show {
        /// ID of the task
        id: i64,
    },
    /// Mark a task as done
    Done {
        /// ID of the task
        id: i64,
    },
    /// Remove a task
    Remove {
        /// ID of the task
        id: i64,
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

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

pub fn print_completions(shell: Shell, out: &mut impl Write) {
    clap_complete::generate(shell, &mut Cli::command(), "todo", out);
}

/// Runs one command. Generic over the reminders list so the tests can run
/// commands against an in-memory list instead of the real Reminders app.
/// With `json`, commands print JSON: the tasks for `list`, `today` and `show`,
/// and the changed task for `add`, `edit`, `done` and `remove`.
pub fn run<B: Backend>(command: Command, json: bool, db: &mut Store<B>) -> Result<()> {
    match command {
        Command::Add { title, due, priority, tags, every } => {
            let task = db.add(&Task { due, priority, tags, repeat: every, ..Task::new(&title) })?;
            if json {
                display::print_task_json(&task)?;
            } else {
                println!("Added task {}: {}", task.id, task.title);
            }
        }
        Command::Edit { id, title, due, no_due, priority, tags, untags, every, no_repeat } => {
            if title.is_none()
                && due.is_none()
                && !no_due
                && priority.is_none()
                && tags.is_empty()
                && untags.is_empty()
                && every.is_none()
                && !no_repeat
            {
                bail!("nothing to change; see `todo edit --help` for the options");
            }

            let mut task = db.get(id)?.with_context(|| format!("no task with id {id}"))?;
            if let Some(title) = title {
                task.title = title;
            }
            if no_due {
                task.due = None;
            } else if due.is_some() {
                task.due = due;
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

            let task = db.update(&task)?;
            if json {
                display::print_task_json(&task)?;
            } else {
                println!("Updated task {id}: {}", task.title);
            }
        }
        Command::List { tag, priority, search, pending, sort } => {
            let filter = Filter { tag, priority, search, pending_only: pending, sort, ..Filter::default() };
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
            let task = db.get(id)?.with_context(|| format!("no task with id {id}"))?;
            if json {
                display::print_task_json(&task)?;
            } else {
                display::print_task_details(&task);
            }
        }
        Command::Done { id } => {
            let task = db.set_done(id, true)?;
            if json {
                display::print_task_json(&task)?;
                return Ok(());
            }
            match (task.done, task.due) {
                (false, Some(next)) => {
                    println!("Completed task {id}: {} (repeats, next due {next})", task.title)
                }
                _ => println!("Completed task {id}: {}", task.title),
            }
        }
        Command::Remove { id } => {
            let task = db.remove(id)?;
            if json {
                display::print_task_json(&task)?;
            } else {
                println!("Removed task {id}: {}", task.title);
            }
        }
        Command::Undo => match (db.undo()?, json) {
            (what, true) => println!("{}", serde_json::json!({ "undone": what })),
            (Some(what), false) => println!("Undid {what}"),
            (None, false) => println!("Nothing to undo"),
        },
        Command::Tui => tui::run(db)?,
        Command::Notify => notify::due_tomorrow(db)?,
        Command::Completions { .. } => unreachable!("handled above"),
    }

    Ok(())
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
        let Command::Add { title, due, priority, tags, every } = command(&[
            "add", "Milk", "-d", "2026-10-20", "-p", "high", "-t", "home", "-t", "shop", "--every", "2 weeks",
        ]) else {
            panic!("not an add command");
        };
        assert_eq!(title, "Milk");
        assert_eq!(due, Some("2026-10-20".parse().unwrap()));
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
        let Command::List { tag, priority, search, pending, sort } =
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
            &["add", "x", "--due", "someday"],
            &["add", "x", "--priority", "urgent"],
            &["add", "x", "--every", "fortnight"],
            &["done", "one"],
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
            vec!["undo"],
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
