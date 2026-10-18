//! The `todo` command-line program, built on the `todo` library.

mod cli;
mod display;
mod notify;
mod tui;

use std::env;
use std::io;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use todo::{ErrorKind, RemindersList, Store};

use cli::{Cli, Command};

fn main() -> ExitCode {
    // Looked up before parsing, so that even a parse error can be JSON.
    let json = cli::wants_json(env::args_os().skip(1));
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // `--help` and `--version` also end up here; clap prints those as usual.
        Err(err) if !json || !err.use_stderr() => err.exit(),
        Err(err) => return cli::report_json_error(ErrorKind::InvalidInput, &cli::clap_message(&err)),
    };
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if json => cli::report_json_error(ErrorKind::of(&err), &format!("{err:#}")),
        // The same output as returning the error from `main`.
        Err(err) => {
            eprintln!("Error: {err:?}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
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
