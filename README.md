# to-do-cli

A command-line to-do manager for macOS, written in Rust. Your tasks are stored
in **Apple Reminders**, so everything you add in the terminal also shows up on
your iPhone, iPad and Mac through iCloud, and the other way around.

```
$ todo add "Dentist" --due friday --priority high --tag health
Added task 12: Dentist

$ todo list --pending --sort due
╭────┬───┬──────────────┬──────────┬──────────────┬────────╮
│ ID ┆   ┆ Title        ┆ Priority ┆ Due          ┆ Tags   │
╞════╪═══╪══════════════╪══════════╪══════════════╪════════╡
│ 9  ┆   ┆ Pay rent     ┆ high     ┆ 2026-10-15   ┆ money  │
│ 11 ┆   ┆ Water plants ┆          ┆ 2026-10-17 ↻ ┆        │
│ 12 ┆   ┆ Dentist      ┆ high     ┆ 2026-10-23   ┆ health │
╰────┴───┴──────────────┴──────────┴──────────────┴────────╯
```

## Features

- Add, edit, complete and remove tasks, with **due dates**, **priorities** and **tags**
- **Repeating tasks** (`--every week`, `--every "2 days"`, ...)
- **Filter, search and sort** the list; a `today` view for what's due or overdue
- **Undo**, as many steps back as you like
- A **terminal UI** (`todo tui`) with keyboard shortcuts
- **Multiple lists** (`--list Shopping`)
- A macOS **notification** for tasks due tomorrow
- **Tab completion** for zsh, bash and fish
- **JSON output** (`--json`) for scripts and AI assistants

## Requirements

- macOS 14 (Sonoma) or newer
- Rust, installed with [rustup](https://rustup.rs):
  `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

## Installation

Install straight from GitHub:

```sh
cargo install --git https://github.com/thijskuipers01/to-do-cli
```

Or from a local copy, for example after making your own changes:

```sh
git clone https://github.com/thijskuipers01/to-do-cli.git
cd to-do-cli
cargo install --path .
```

Either way, Cargo builds the program and puts the `todo` command in
`~/.cargo/bin`, which the Rust installer adds to your `PATH`. Open a new
terminal and `todo` works from any folder; you don't need `cargo run` or the
source code anymore.

**First run.** macOS asks for permission to access Reminders. Click
**Allow**. If you declined by accident, turn it on under **System Settings →
Privacy & Security → Reminders** for your terminal app. Your tasks then go
in the **Todo** list of the Apple account you're signed in with.

**Updating.** Run the same `cargo install` command again; add `--force` to
reinstall when the version number hasn't changed.

**Uninstalling.** Run `cargo uninstall todo`. Your tasks stay in Reminders;
delete `~/.local/share/todo/` to remove the local task numbers and undo
history as well.

## Usage

```sh
todo add "Buy milk"                                  # just a title
todo add "Report" -d friday -p high -t work          # due date, priority, tag
todo add "Water plants" --every week                 # repeating, starts today
todo list                                            # everything
todo list --pending --sort due                       # open tasks, soonest first
todo list --tag work --search report                 # filter and search
todo today                                           # overdue or due today
todo show 3                                          # all details, including notes
todo edit 3 --title "Final report" -d 2026-11-01     # change fields
todo edit 3 -t urgent --untag work --no-repeat       # tags and repeat
todo add "Dentist" --notes "Bring insurance card"    # notes
todo edit 3 --append-notes "Booked by phone"         # add a line to the notes
todo done 3
todo remove 3
todo undo                                            # repeat to go further back
todo tui                                             # interactive mode
```

Run `todo --help` or `todo <command> --help` for every option.

### Commands

| Command | What it does |
|---|---|
| `add <title>` | Add a task. Options: `-d/--due`, `-p/--priority`, `-t/--tag` (repeatable), `-e/--every`, `--notes` |
| `list` | List tasks. Options: `-t/--tag`, `-p/--priority`, `--search`, `--pending`, `-s/--sort id\|due\|priority` |
| `today` | Unfinished tasks that are overdue or due today |
| `show <id>` | Every detail of one task, including its notes |
| `edit <id>` | Change a task: `--title`, `-d/--due`, `--no-due`, `-p/--priority`, `-t/--tag`, `--untag`, `-e/--every`, `--no-repeat`, `--notes`, `--no-notes`, `--append-notes` |
| `done <id>` | Mark a task as done |
| `remove <id>` | Delete a task |
| `undo` | Undo the last change; repeat to go further back |
| `tui` | Open the terminal UI |
| `notify` | Show a macOS notification for tasks due tomorrow |
| `completions <shell>` | Print a completion script (`zsh`, `bash`, `fish`, ...) |

Every command also accepts `-l/--list <name>` to work on another Reminders list.

### JSON output

Add `--json` to get machine-readable output, for scripts or an AI assistant
that manages your tasks through `todo`. `list` and `today` print an array of
tasks, `show` prints one task, and `add`, `edit`, `done` and `remove` print the
task they changed (so you get the ID of a new task back). `undo` prints
`{"undone": "..."}`, or `{"undone": null}` when there is nothing to undo.
Errors still go to stderr with a non-zero exit code.

```sh
$ todo add "Dentist" --due friday --priority high --tag health --json
{
  "id": 12,
  "title": "Dentist",
  "done": false,
  "due": "2026-10-23",
  "priority": "high",
  "tags": [
    "health"
  ],
  "notes": "",
  "repeat": null,
  "list": "Todo"
}
```

Values use the same words as the options (`"high"`, `"every 2 weeks"`), so
they can be passed straight back to `add` or `edit`. Empty fields are `null`.

### Input formats

| What | Accepted values |
|---|---|
| Due date | `2026-10-20`, `today`, `tomorrow`, or a weekday such as `friday`/`fri` (the next one after today) |
| Priority | `none` (default), `low`, `medium`, `high` |
| Repeat | `day`, `week`, `month`, `year`, optionally with a number and/or "every": `2 weeks`, `every 3 days`, `monthly` |

### Terminal UI

| Key | Action |
|---|---|
| `↑`/`↓` or `k`/`j` | Move the selection |
| `a` | Add a task (opens a form) |
| `e` | Edit the selected task |
| `Space`/`Enter` | Mark done / not done |
| `d`/`Delete` | Delete the selected task |
| `u` | Undo |
| `/` | Search (filters while you type; `Enter` keeps it, `Esc` clears it) |
| `s` | Change the sort order (id → due date → priority) |
| `h` | Hide or show completed tasks |
| `q`/`Esc` | Quit |

In the form, `Tab` or `↑`/`↓` moves between fields, `Enter` saves and `Esc` cancels.

## How it works

**Tasks live in Reminders.** By default `todo` uses the list called **Todo**
(it's created if it doesn't exist). Pick another list with `--list <name>` or
by setting `TODO_LIST`. Reminders you create on your phone appear in
`todo list` and get a number the first time they're seen.

**Short IDs and undo history** are kept in a small local database, because
Reminders itself uses long identifiers such as `3A365278-64BD-…`. Each task
gets a number that's never reused, even after it's deleted. The database lives
at `~/.local/share/todo/state.db` (or `$XDG_DATA_HOME/todo/state.db`; override
it with `TODO_STATE`). Deleting it is safe, but tasks get new numbers and the
undo history is lost.

**Tags** are stored as a line of hashtags at the end of the reminder's notes,
for example `#work #urgent`, because Apple doesn't let other apps read or set
Reminders' own tags. You can see and edit that line on your phone. Tags added
with the Reminders tag feature itself aren't visible to `todo`. Any other text
in the notes is left untouched, and `--notes`, `--no-notes` and
`--append-notes` change only that other text, never the hashtag line. Notes can
only be set from the command line; the terminal UI form has no notes field.

**Priorities** map to the ones in Reminders: high shows as `!!!`, medium as
`!!` and low as `!` on your phone.

**Repeating tasks** work like they do in Reminders: completing one moves it to
its next date and keeps a completed copy as a record. A repeating task needs a
due date, so one without a date starts today.

## Shell completion

With **Oh My Zsh**:

```sh
mkdir -p ~/.oh-my-zsh/custom/plugins/todo
todo completions zsh > ~/.oh-my-zsh/custom/plugins/todo/_todo
```

Then add `todo` to the `plugins=(...)` line in `~/.zshrc` and open a new terminal.

With **plain zsh**, write the script to a folder in your `$fpath` (for example
`~/.zfunc/_todo`, with `fpath+=~/.zfunc` before `compinit` in `~/.zshrc`). For
**bash** or **fish**, use `todo completions bash` or `todo completions fish`.

Run the command again after updating `todo` so new options are completed too.

## Notifications

`todo notify` shows a macOS notification listing the unfinished tasks due
tomorrow. macOS shows it as coming from **Script Editor**, which has to be
allowed under **System Settings → Notifications**. To get the reminder
automatically, schedule `todo notify` with launchd.

## Limitations

- macOS only. EventKit, the Reminders framework, doesn't exist elsewhere.
- Due dates are whole days; a time of day set on the iPhone is kept but not shown.
- Undoing a change overwrites edits made to that task on another device in
  the meantime. Undoing the completion of a repeating task leaves its completed
  copy in place.

## Using it as a library

The package is a library with the `todo` program built on top. Another Rust
project can use the library directly:

```toml
[dependencies]
# Without the default `cli` feature you get only the library, without
# clap, ratatui and comfy-table.
todo = { path = "../to-do-cli", default-features = false }
```

```rust
use todo::{Filter, MemoryList, Priority, Store, Task};

// Store::open(path, "Todo") uses the real Reminders app instead.
let mut store = Store::in_memory(MemoryList::new("Todo"))?;
let task = store.add(&Task { priority: Priority::High, ..Task::new("Write report") })?;
store.set_done(task.id, true)?;
let open = store.list(&Filter { pending_only: true, ..Filter::default() })?;
```

Run `cargo doc --open` for the full API documentation.

## Development

```sh
cargo build
cargo test                          # all tests, against an in-memory reminders list
cargo test --no-default-features    # only the library
cargo test -- --ignored             # integration tests against the real Reminders app
cargo clippy --all-targets
```

The integration tests create temporary lists named `todo-cli-test-…` (they
sync to iCloud briefly) and delete them afterwards. They need Reminders access.

### Project layout

```
src/
├── lib.rs                 library entry point and public API
├── model/                 data types, no storage
│   ├── task.rs            Task, DueStatus
│   ├── priority.rs        Priority
│   ├── repeat.rs          Repeat, RepeatUnit, parse_repeat
│   └── dates.rs           today, parse_due
├── reminders/             where tasks are kept
│   ├── mod.rs             the Backend trait, Reminder, ReminderData
│   ├── eventkit.rs        RemindersList: the Reminders app (all `unsafe` code)
│   └── memory.rs          MemoryList: in memory, for tests
├── store/                 the Store on top of a Backend
│   ├── mod.rs             short IDs, undo history (SQLite)
│   ├── filter.rs          Filter, SortKey
│   └── convert.rs         Task ↔ reminder conversion, tags as hashtags
└── bin/todo/              the command-line program (needs the `cli` feature)
    ├── main.rs            entry point
    ├── cli.rs             command-line options and running each command
    ├── display.rs         the colored table and `show` output
    ├── notify.rs          the "due tomorrow" notification
    └── tui/
        ├── mod.rs         the terminal UI
        └── form.rs        the add/edit form
tests/
└── library.rs             uses the library through its public API only
```
