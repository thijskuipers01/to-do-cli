//! `todo-mcp`: a Model Context Protocol server that gives AI assistants such
//! as Claude Desktop and Claude Code the `todo` actions as typed tools. It
//! speaks MCP over stdin/stdout and is built on the `todo` library, so tasks,
//! short IDs and the undo history are shared with the `todo` command.

mod server;
mod tools;

use std::env;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use todo::Store;

const USAGE: &str = "\
todo-mcp: an MCP server for the todo task manager (Apple Reminders)

Usage: todo-mcp [--list <name>]

Speaks the Model Context Protocol on stdin/stdout; start it from an MCP
client such as Claude Desktop or Claude Code rather than by hand.

Options:
  -l, --list <name>  Reminders list used when a tool call doesn't name one
                     [env: TODO_LIST] [default: Todo]
  -h, --help         Print this help
  -V, --version      Print the version

The local state (short IDs and undo history) is the same as the todo
command's: $TODO_STATE, or $XDG_DATA_HOME/todo/state.db, or
~/.local/share/todo/state.db.";

fn main() -> ExitCode {
    let mut list = env::var("TODO_LIST").ok().filter(|name| !name.is_empty()).unwrap_or("Todo".into());
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("todo-mcp {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "-l" | "--list" => match args.next() {
                Some(name) => list = name,
                None => return usage_error("--list needs a list name"),
            },
            other => return usage_error(&format!("unexpected argument '{other}'")),
        }
    }

    let default_list = list.clone();
    let mut server = server::Server::new(
        &default_list,
        Box::new(move || Store::open(&todo::store::state_path()?, &list)),
    );

    // Only protocol messages may go to stdout; anything else goes to stderr.
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("todo-mcp: could not read stdin: {e}");
                return ExitCode::FAILURE;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line)
            && writeln!(stdout, "{response}").and_then(|()| stdout.flush()).is_err()
        {
            // The client went away.
            return ExitCode::SUCCESS;
        }
    }
    ExitCode::SUCCESS
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("todo-mcp: {message}\n\n{USAGE}");
    ExitCode::from(2)
}
