use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

/// File where tasks are stored (in the current directory for now).
const DATA_FILE: &str = "tasks.json";

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A single to-do item. `Serialize`/`Deserialize` let serde convert it
/// to and from JSON automatically.
#[derive(Debug, Serialize, Deserialize)]
struct Task {
    id: u32,
    title: String,
    done: bool,
}

/// Everything stored in the data file: the tasks plus a counter for the
/// next ID. The counter only ever goes up, so IDs are never reused, even
/// after the task with the highest ID is removed.
#[derive(Debug, Serialize, Deserialize)]
struct TodoList {
    next_id: u32,
    tasks: Vec<Task>,
}

impl Default for TodoList {
    fn default() -> Self {
        TodoList { next_id: 1, tasks: Vec::new() }
    }
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
    },
    /// List all tasks
    List,
    /// Mark a task as done
    Done {
        /// ID of the task
        id: u32,
    },
    /// Remove a task
    Remove {
        /// ID of the task
        id: u32,
    },
}

// ---------------------------------------------------------------------------
// Storage: load from / save to the JSON file
// ---------------------------------------------------------------------------

fn load_list(path: &Path) -> Result<TodoList> {
    // No file yet? Then we simply start with an empty list.
    if !path.exists() {
        return Ok(TodoList::default());
    }
    let contents = fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let list = serde_json::from_str(&contents)
        .with_context(|| format!("{} contains invalid JSON", path.display()))?;
    Ok(list)
}

fn save_list(path: &Path, list: &TodoList) -> Result<()> {
    let json = serde_json::to_string_pretty(list)?;
    fs::write(path, json).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Task logic
// ---------------------------------------------------------------------------

fn print_tasks(tasks: &[Task]) {
    if tasks.is_empty() {
        println!("No tasks yet. Add one with: todo add \"Buy milk\"");
        return;
    }
    for task in tasks {
        let mark = if task.done { "x" } else { " " };
        println!("{:>3}. [{}] {}", task.id, mark, task.title);
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = Path::new(DATA_FILE);
    let mut list = load_list(path)?;

    match cli.command {
        Command::Add { title } => {
            let id = list.next_id;
            list.next_id += 1;
            println!("Added task {id}: {title}");
            list.tasks.push(Task { id, title, done: false });
            save_list(path, &list)?;
        }
        Command::List => {
            print_tasks(&list.tasks);
        }
        Command::Done { id } => {
            // `iter_mut().find(..)` gives Option<&mut Task>: a mutable borrow
            // of the matching task inside the Vec.
            match list.tasks.iter_mut().find(|t| t.id == id) {
                Some(task) => {
                    task.done = true;
                    println!("Completed task {id}: {}", task.title);
                    save_list(path, &list)?;
                }
                None => bail!("no task with id {id}"),
            }
        }
        Command::Remove { id } => {
            // `position` returns the index of the first match, if any.
            match list.tasks.iter().position(|t| t.id == id) {
                Some(index) => {
                    let removed = list.tasks.remove(index);
                    println!("Removed task {id}: {}", removed.title);
                    save_list(path, &list)?;
                }
                None => bail!("no task with id {id}"),
            }
        }
    }

    Ok(())
}
