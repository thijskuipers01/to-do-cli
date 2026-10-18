---
name: todo
description: Manage the user's tasks, to-dos and Apple Reminders with the `todo` command-line tool. Use it whenever the user asks to add, find, change, complete, reschedule or remove a task or reminder, asks what is due (today, this week, overdue), or wants something put on a list such as a shopping list. Prefer it over AppleScript or Shortcuts for Reminders.
---

# Managing tasks with `todo`

`todo` stores tasks in Apple Reminders, so they sync to the user's iPhone and
Mac. Always call it with `--json`: output is then JSON on stdout, and errors
are JSON on stderr with a meaningful exit code. Never script the Reminders
app with AppleScript, `osascript` or Shortcuts instead.

If `todo` isn't installed (`command -v todo` finds nothing), tell the user
and point them to the README; don't fall back to AppleScript.

## Read before you write

- Tasks are identified by a short number, the `id`. **Never guess an ID.**
  Look it up first with `todo list --json` (with `--search`, `--tag` or a date
  filter to narrow it down), `todo today --json` or `todo show <id> --json`.
- If several tasks match the user's description, ask which one they mean.
- Every command works on one Reminders list, `Todo` unless you pass
  `-l/--list <name>`. **`--list` silently creates a list that doesn't exist**,
  so before using a list the user named ("my shopping list"), run
  `todo lists --json` and use the exact name that is there (`Groceries`,
  `Shopping`, ...). If none fits, ask before creating one.
- An ID found in one list works with `show`, `edit`, `done` and `remove`
  whichever list is selected.
- When a change is significant or the request is ambiguous, show the result
  first with `--dry-run` (on `edit`, `done`, `remove`), confirm with the user,
  then run the same command without it.

## Commands

| Command | Use |
|---|---|
| `todo list --json [filters]` | Find tasks. Filters: `--pending` (hide done), `-t/--tag <tag>`, `-p/--priority <p>`, `--search <text>` (title and notes), `--due-before <date>`, `--due-after <date>` (inclusive), `--overdue`, `--no-due`, `-s/--sort id\|due\|priority` |
| `todo today --json` | Unfinished tasks that are overdue or due today |
| `todo show <id> --json` | One task with all its details |
| `todo lists --json` | All Reminders lists: `[{"name": "Todo", "open": 4}, ...]` |
| `todo add "<title>" --json [options]` | New task. Options: `-d/--due`, `-p/--priority`, `-t/--tag` (repeat for more), `-e/--every`, `--notes`. Prints the task with its new `id` |
| `todo edit <id> --json [options]` | Change a task: `--title`, `-d/--due`, `--no-due`, `-p/--priority`, `-t/--tag` (add), `--untag` (remove), `-e/--every`, `--no-repeat`, `--notes` (replace), `--append-notes` (add a line), `--no-notes`, `--dry-run` |
| `todo done <id>... --json` | Complete one or more tasks; `--dry-run` to preview |
| `todo remove <id>... --json` | Delete one or more tasks; `--dry-run` to preview |
| `todo history --json [-n N]` | Recent changes, newest first |
| `todo undo --json` | Revert the most recent change |

Don't run `todo tui` (interactive) or `todo notify` (desktop notification).

## Input formats

- **Due date** (`--due`): `2026-10-23`, `today`, `tomorrow`, or a weekday
  (`friday`, `fri`), meaning the *next* such day after today, so `friday` on
  a Friday is a week later. Use an explicit date when that matters.
- **Due time** (optional, after the date): `friday 15:00`, `tomorrow 3pm`,
  `2026-10-23 9:30am`, `2026-10-23T15:00`; a time alone (`15:00`) is today.
  A task with a time also gets an alert on the user's devices. Giving only a
  date makes the task all-day again.
- **Date filters** (`--due-before`, `--due-after`) take a date without a time.
- **Priority**: `none`, `low`, `medium`, `high`.
- **Repeat** (`--every`): `day`, `week`, `month`, `year`, with a count
  and/or "every": `2 weeks`, `every 3 days`, `monthly`. Without `--due`, a
  repeating task starts today.
- **Tags**: one word each; spaces become dashes. They're stored as a
  `#hashtag` line in the reminder's notes, which `--notes` never touches.

## Output

A task looks like this; values use the same words as the options, so they can
be passed straight back. Empty fields are `null`.

```json
{"id": 12, "title": "Dentist", "done": false, "due": "2026-10-23", "due_time": "15:00",
 "priority": "high", "tags": ["health"], "notes": "Bring insurance card",
 "repeat": "every 2 weeks", "list": "Todo"}
```

- `list` and `today` print an array of tasks; `show`, `add` and `edit` print
  one task.
- `done` and `remove` print the task for a single ID and an array for several
  IDs. After `done`, a repeating task has `"done": false` and its next `due`
  date: that's expected.
- `undo` prints `{"undone": "done of tasks 3, 5"}`, or `{"undone": null}`
  when there's nothing to undo.
- `history` prints `[{"action": "done", "id": 3, "title": "Milk", "at": "...",
  "batch": 2}, ...]`. Rows sharing a `batch` came from one command, and one
  `undo` reverts them all.

## Errors

Errors go to stderr as `{"error": "<kind>", "message": "..."}`:

| `error` | Exit code | What to do |
|---|---|---|
| `invalid_input` | 2 | Fix the arguments (date, priority, option); the message says what's wrong |
| `not_found` | 3 | The ID doesn't exist (anymore); look it up again with `list` |
| `permission_denied` | 4 | Tell the user to allow their terminal under System Settings → Privacy & Security → Reminders |
| `other` | 1 | Report the message to the user |

`done` and `remove` with several IDs check them all first: if one is unknown,
nothing changes.

## Undo

`undo` reverts the most recent change made through `todo` (add, edit, done,
remove), or the whole of the last `done`/`remove` with several IDs. Repeat it
to go further back. Check `todo history --json` first to see what the next
`undo` would revert. It can't undo changes made on the user's phone.

## Examples

**"What's due today?"**
```sh
todo today --json
```

**"What do I have this week?"**
```sh
todo list --pending --due-after today --due-before sunday --sort due --json
```

**"What's overdue?"**
```sh
todo list --overdue --json
```

**"Move the dentist to Monday at 9."**
```sh
todo list --pending --search dentist --json        # find the ID, e.g. 12
todo edit 12 --due "monday 9:00" --dry-run --json  # show the result, confirm if unsure
todo edit 12 --due "monday 9:00" --json
```

**"Add milk to my shopping list."**
```sh
todo lists --json                                  # finds "Groceries"
todo add "Milk" --list Groceries --json
```

**"Remind me to call mum on Friday at 3pm, it's about the holidays."**
```sh
todo add "Call mum" --due "friday 15:00" --notes "About the holidays" --json
```

**"I bought the milk, eggs and bread."**
```sh
todo list --list Groceries --pending --json        # IDs 4, 7 and 9
todo done 4 7 9 --json
```

**"Oops, undo that."**
```sh
todo history -n 3 --json                           # check what will be reverted
todo undo --json
```
