mod display;
mod notify;
mod reminders;
mod store;
mod task;
mod tui;

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};

use store::{Filter, SortKey, Store};
use task::Priority;

/// The Reminders list that holds the tasks: `$TODO_LIST` if set, otherwise "Todo".
fn list_name() -> String {
    env::var("TODO_LIST").unwrap_or_else(|_| "Todo".to_string())
}

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
#[command(name = "todo", version, about = "A simple command-line to-do manager")]
struct Cli {
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
    },
    /// Change the title, due date, priority or tags of a task
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
    },
    /// List tasks
    List {
        /// Only show tasks with this tag
        #[arg(short, long)]
        tag: Option<String>,
        /// Only show tasks with this priority
        #[arg(short, long, value_enum)]
        priority: Option<Priority>,
        /// Hide completed tasks
        #[arg(long)]
        pending: bool,
        /// Order of the list
        #[arg(short, long, value_enum, default_value_t = SortKey::Id)]
        sort: SortKey,
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
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut db = Store::open(&state_path()?, &list_name())?;

    match cli.command {
        Command::Add { title, due, priority, tags } => {
            let task = db.add(&title, due, priority, &tags)?;
            println!("Added task {}: {}", task.id, task.title);
        }
        Command::Edit { id, title, due, no_due, priority, tags, untags } => {
            if title.is_none()
                && due.is_none()
                && !no_due
                && priority.is_none()
                && tags.is_empty()
                && untags.is_empty()
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

            let task = db.update(&task)?;
            println!("Updated task {id}: {}", task.title);
        }
        Command::List { tag, priority, pending, sort } => {
            let filter = Filter { tag, priority, pending_only: pending, sort };
            display::print_tasks(&db.list(&filter)?);
        }
        Command::Done { id } => {
            let task = db.set_done(id, true)?;
            println!("Completed task {id}: {}", task.title);
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
    }

    Ok(())
}
