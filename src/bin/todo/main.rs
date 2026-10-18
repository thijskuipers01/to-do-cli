//! The `todo` command-line program, built on the `todo` library.

mod cli;
mod display;
mod notify;
mod tui;

use std::io;

use anyhow::Result;
use clap::Parser;
use todo::{RemindersList, Store};

use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Completions don't need Reminders, so handle them before asking for access.
    if let Command::Completions { shell } = cli.command {
        cli::print_completions(shell, &mut io::stdout());
        return Ok(());
    }
    // Neither do lists need one particular list, which would be created if
    // it didn't exist.
    if let Command::Lists = cli.command {
        return cli::print_lists(&RemindersList::all_lists()?, cli.json);
    }

    let mut store = Store::open(&cli::state_path()?, &cli.list)?;
    cli::run(cli.command, cli.json, &mut store)
}

/// A store backed by an empty in-memory list called "Todo", for the tests.
#[cfg(test)]
fn test_store() -> Store<todo::MemoryList> {
    Store::in_memory(todo::MemoryList::new("Todo")).unwrap()
}
