mod display;
mod notify;
mod reminders;
mod store;
mod task;
mod tui;

use std::env;
use std::io;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use store::{Filter, SortKey, Store};
use task::{Priority, Repeat, Task};

/// Where the local state (short IDs and undo history) lives: `$TODO_STATE` if
/// set, otherwise `$XDG_DATA_HOME/todo/state.db`, which defaults to
/// `~/.local/share/todo/state.db`.
fn state_path() -> Result<PathBuf> {
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
struct Cli {
    /// Reminders list to use (created if it doesn't exist)
    #[arg(short, long, global = true, env = "TODO_LIST", default_value = "Todo")]
    list: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a new task
    Add {
        /// The task description
        title: String,
        /// Due date: YYYY-MM-DD, "today", "tomorrow" or a weekday ("friday")
        #[arg(short, long, value_parser = task::parse_due)]
        due: Option<NaiveDate>,
        /// How urgent the task is
        #[arg(short, long, value_enum, default_value_t = Priority::None)]
        priority: Priority,
        /// Tag to attach; repeat for more (-t work -t urgent)
        #[arg(short, long = "tag")]
        tags: Vec<String>,
        /// Repeat the task: "day", "week", "2 weeks", "monthly", ... (starts today without --due)
        #[arg(short, long, value_parser = task::parse_repeat)]
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
        #[arg(short, long, value_parser = task::parse_due, conflicts_with = "no_due")]
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
        #[arg(short, long, value_parser = task::parse_repeat, conflicts_with = "no_repeat")]
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Completions don't need Reminders, so handle them before asking for access.
    if let Command::Completions { shell } = cli.command {
        clap_complete::generate(shell, &mut Cli::command(), "todo", &mut io::stdout());
        return Ok(());
    }

    let mut db = Store::open(&state_path()?, &cli.list)?;

    match cli.command {
        Command::Add { title, due, priority, tags, every } => {
            let task = db.add(&Task { due, priority, tags, repeat: every, ..Task::new(&title) })?;
            println!("Added task {}: {}", task.id, task.title);
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
            println!("Updated task {id}: {}", task.title);
        }
        Command::List { tag, priority, search, pending, sort } => {
            let filter = Filter { tag, priority, search, pending_only: pending, sort, ..Filter::default() };
            display::print_tasks(&db.list(&filter)?);
        }
        Command::Today => {
            let filter = Filter {
                pending_only: true,
                due_by: Some(task::today()),
                sort: SortKey::Due,
                ..Filter::default()
            };
            let tasks = db.list(&filter)?;
            if tasks.is_empty() {
                println!("Nothing overdue or due today.");
            } else {
                display::print_tasks(&tasks);
            }
        }
        Command::Show { id } => {
            let task = db.get(id)?.with_context(|| format!("no task with id {id}"))?;
            display::print_task_details(&task);
        }
        Command::Done { id } => {
            let task = db.set_done(id, true)?;
            match (task.done, task.due) {
                (false, Some(next)) => {
                    println!("Completed task {id}: {} (repeats, next due {next})", task.title)
                }
                _ => println!("Completed task {id}: {}", task.title),
            }
        }
        Command::Remove { id } => {
            let task = db.remove(id)?;
            println!("Removed task {id}: {}", task.title);
        }
        Command::Undo => match db.undo()? {
            Some(what) => println!("Undid {what}"),
            None => println!("Nothing to undo"),
        },
        Command::Tui => tui::run(&mut db)?,
        Command::Notify => notify::due_tomorrow(&db)?,
        Command::Completions { .. } => unreachable!("handled above"),
    }

    Ok(())
}
