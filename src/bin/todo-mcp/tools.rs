//! The tools the server offers: their definitions (names, descriptions and
//! JSON Schemas for the arguments) and running them against a store.
//!
//! Inputs use the same words as the `todo` command-line options, and results
//! are the same JSON as `todo --json` prints (see [`todo::json`]).

use anyhow::Result;
use serde_json::{Map, Value, json};
use todo::model::{self, Priority, Task};
use todo::{Backend, Error, Filter, SortKey, Store, json as out};

/// A tool call's arguments.
pub type Args = Map<String, Value>;

const DUE_FORMATS: &str = "A date: YYYY-MM-DD, \"today\", \"tomorrow\" or a weekday such as \
    \"friday\" (the next one after today), optionally followed by a time: \"friday 15:00\", \
    \"tomorrow 3pm\", \"2026-10-23T15:00\". A time alone (\"15:00\") means today.";
const DATE_FORMATS: &str = "A date: YYYY-MM-DD, \"today\", \"tomorrow\" or a weekday such as \"friday\".";
const REPEAT_FORMATS: &str =
    "How often the task repeats: \"day\", \"week\", \"2 weeks\", \"every 3 days\", \"monthly\", ...";
const LIST: &str = "Name of the Reminders list. Defaults to the server's list (usually \"Todo\"). \
    Use list_lists to find the exact name.";
const RETURNS_TASK: &str = "Returns the task as JSON: {id, title, done, due, due_time, priority, \
    tags, notes, repeat, list}.";

/// Whether `name` is one of the tools.
pub fn exists(name: &str) -> bool {
    definitions().as_array().into_iter().flatten().any(|tool| tool["name"] == name)
}

/// The definitions returned by `tools/list`.
pub fn definitions() -> Value {
    let priority = json!({ "type": "string", "enum": ["none", "low", "medium", "high"] });
    let ids = json!({
        "type": "array",
        "items": { "type": "integer" },
        "minItems": 1,
        "description": "IDs of the tasks. If one doesn't exist, nothing is changed."
    });
    let dry_run = json!({
        "type": "boolean",
        "description": "Only show the result; change nothing and record nothing for undo."
    });
    json!([
        {
            "name": "list_tasks",
            "title": "List tasks",
            "description": "Lists the tasks in a Reminders list, filtered and sorted. Returns an \
                array of tasks. Use it to find a task's ID before changing it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "list": { "type": "string", "description": LIST },
                    "pending": { "type": "boolean", "description": "Leave out completed tasks." },
                    "tag": { "type": "string", "description": "Only tasks with this tag." },
                    "priority": priority,
                    "search": { "type": "string", "description": "Text that must appear in the title or notes (case is ignored)." },
                    "due_before": { "type": "string", "description": format!("Only tasks due on or before this date. {DATE_FORMATS}") },
                    "due_after": { "type": "string", "description": format!("Only tasks due on or after this date. {DATE_FORMATS}") },
                    "overdue": { "type": "boolean", "description": "Only unfinished tasks due before today." },
                    "no_due": { "type": "boolean", "description": "Only tasks without a due date." },
                    "sort": { "type": "string", "enum": ["id", "due", "priority"], "description": "Order of the tasks (default id)." }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "get_task",
            "title": "Get a task",
            "description": format!("Gets one task by its ID, in any list. {RETURNS_TASK}"),
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer" } },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "add_task",
            "title": "Add a task",
            "description": format!("Adds a task. A task with a due time also gets an alert at \
                that time. {RETURNS_TASK}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "list": { "type": "string", "description": format!("{LIST} If it doesn't exist, the call fails unless create_list is true.") },
                    "create_list": { "type": "boolean", "description": "Create `list` if it doesn't exist yet." },
                    "due": { "type": "string", "description": DUE_FORMATS },
                    "priority": priority,
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "notes": { "type": "string" },
                    "repeat": { "type": "string", "description": format!("{REPEAT_FORMATS} Without `due`, it starts today.") }
                },
                "required": ["title"],
                "additionalProperties": false
            }
        },
        {
            "name": "update_task",
            "title": "Change a task",
            "description": format!("Changes a task; only the given fields change. Pass null for \
                `due`, `repeat` or `notes` to clear them. {RETURNS_TASK}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "title": { "type": "string" },
                    "due": { "type": ["string", "null"], "description": format!("{DUE_FORMATS} A date without a time makes the task all-day.") },
                    "priority": priority,
                    "add_tags": { "type": "array", "items": { "type": "string" } },
                    "remove_tags": { "type": "array", "items": { "type": "string" } },
                    "notes": { "type": ["string", "null"], "description": "Replaces the notes. Tags are kept." },
                    "append_notes": { "type": "string", "description": "A line to add to the end of the notes." },
                    "repeat": { "type": ["string", "null"], "description": REPEAT_FORMATS },
                    "dry_run": dry_run
                },
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "complete_task",
            "title": "Complete tasks",
            "description": "Marks tasks as done. Returns an array of the tasks as they are now; a \
                repeating task stays open and moves to its next due date. One undo reverts the \
                whole call.",
            "inputSchema": {
                "type": "object",
                "properties": { "ids": ids, "dry_run": dry_run },
                "required": ["ids"],
                "additionalProperties": false
            }
        },
        {
            "name": "remove_task",
            "title": "Remove tasks",
            "description": "Deletes tasks. Returns an array of the tasks as they were. One undo \
                brings them all back.",
            "inputSchema": {
                "type": "object",
                "properties": { "ids": ids, "dry_run": dry_run },
                "required": ["ids"],
                "additionalProperties": false
            },
            "annotations": { "destructiveHint": true }
        },
        {
            "name": "undo",
            "title": "Undo",
            "description": "Reverts the most recent change made through todo (add, update, \
                complete, remove; a whole complete_task or remove_task call at once). Returns \
                {\"undone\": \"<what>\"}, or {\"undone\": null} when there is nothing to undo. \
                Check history first to see what it will revert.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "destructiveHint": true }
        },
        {
            "name": "history",
            "title": "Recent changes",
            "description": "The most recent changes that undo can revert, newest first: \
                [{action, id, title, at, batch}]. Changes with the same batch are undone together.",
            "inputSchema": {
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "description": "How many changes (default 10)." } },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "list_lists",
            "title": "List Reminders lists",
            "description": "All Reminders lists with their number of unfinished tasks: \
                [{name, open}]. Use it to find the exact list name the user means.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        }
    ])
}

/// Runs the tool `name` and returns its result.
pub fn call<B: Backend>(store: &mut Store<B>, default_list: &str, name: &str, args: &Args) -> Result<Value> {
    match name {
        "list_tasks" => list_tasks(store, default_list, args),
        "get_task" => {
            let id = id(args, "id")?;
            Ok(out::task(&store.get(id)?.ok_or_else(|| Error::no_task(id))?))
        }
        "add_task" => add_task(store, default_list, args),
        "update_task" => update_task(store, args),
        "complete_task" => {
            let ids = ids(args)?;
            Ok(out::tasks(&match flag(args, "dry_run")? {
                true => store.preview_done(&ids)?,
                false => store.set_done_many(&ids, true)?,
            }))
        }
        "remove_task" => {
            let ids = ids(args)?;
            Ok(out::tasks(&match flag(args, "dry_run")? {
                true => store.require_all(&ids)?,
                false => store.remove_many(&ids)?,
            }))
        }
        "undo" => Ok(out::undone(store.undo()?.as_deref())),
        "history" => {
            let limit = match given(args, "limit") {
                None => 10,
                Some(value) => value
                    .as_u64()
                    .filter(|&n| n > 0)
                    .ok_or_else(|| invalid("`limit` must be a positive integer"))?
                    as usize,
            };
            Ok(out::history(&store.history(limit)?))
        }
        "list_lists" => Ok(out::lists(&store.backend().lists()?)),
        _ => Err(invalid(format!("unknown tool \"{name}\"")).into()),
    }
}

fn list_tasks<B: Backend>(store: &mut Store<B>, default_list: &str, args: &Args) -> Result<Value> {
    let list = string(args, "list")?.unwrap_or_else(|| default_list.to_string());
    let sort = match string(args, "sort")?.as_deref() {
        None | Some("id") => SortKey::Id,
        Some("due") => SortKey::Due,
        Some("priority") => SortKey::Priority,
        Some(other) => return Err(invalid(format!("`sort` must be id, due or priority, got \"{other}\"")).into()),
    };
    let mut filter = Filter {
        tag: string(args, "tag")?,
        priority: parsed(args, "priority", str::parse::<Priority>)?,
        search: string(args, "search")?,
        pending_only: flag(args, "pending")?,
        due_by: parsed(args, "due_before", model::parse_due)?,
        due_after: parsed(args, "due_after", model::parse_due)?,
        no_due: flag(args, "no_due")?,
        sort,
    };
    if flag(args, "overdue")? {
        filter.only_overdue(model::today());
    }
    use_list(store, &list, false)?;
    Ok(out::tasks(&store.list(&filter)?))
}

fn add_task<B: Backend>(store: &mut Store<B>, default_list: &str, args: &Args) -> Result<Value> {
    let title = string(args, "title")?.filter(|t| !t.trim().is_empty());
    let title = title.ok_or_else(|| invalid("`title` is required"))?;
    let due = parsed(args, "due", model::parse_due_time)?;
    let task = Task {
        due: due.map(|d| d.date),
        due_time: due.and_then(|d| d.time),
        priority: parsed(args, "priority", str::parse::<Priority>)?.unwrap_or(Priority::None),
        tags: strings(args, "tags")?,
        notes: string(args, "notes")?.unwrap_or_default(),
        repeat: parsed(args, "repeat", model::parse_repeat)?,
        ..Task::new(title.trim())
    };
    // The server's own list is created when needed, like `todo add` does;
    // another list only when asked for, so a wrong guess doesn't make one.
    match string(args, "list")? {
        Some(list) => use_list(store, &list, flag(args, "create_list")?)?,
        None => use_list(store, default_list, true)?,
    }
    Ok(out::task(&store.add(&task)?))
}

fn update_task<B: Backend>(store: &mut Store<B>, args: &Args) -> Result<Value> {
    const CHANGES: [&str; 8] =
        ["title", "due", "priority", "add_tags", "remove_tags", "notes", "append_notes", "repeat"];
    let id = id(args, "id")?;
    if !CHANGES.iter().any(|key| args.contains_key(*key)) {
        return Err(invalid(format!("nothing to change; give one of: {}", CHANGES.join(", "))).into());
    }

    let mut task = store.get(id)?.ok_or_else(|| Error::no_task(id))?;
    if let Some(title) = string(args, "title")? {
        task.title = title;
    }
    if let Some(due) = clearable(args, "due", model::parse_due_time)? {
        (task.due, task.due_time) = (due.map(|d| d.date), due.and_then(|d| d.time));
    }
    if let Some(priority) = parsed(args, "priority", str::parse::<Priority>)? {
        task.priority = priority;
    }
    let remove = strings(args, "remove_tags")?;
    task.tags.retain(|tag| !remove.contains(tag));
    task.tags.extend(strings(args, "add_tags")?);
    if let Some(notes) = clearable(args, "notes", |text| Ok::<_, String>(text.to_string()))? {
        task.notes = notes.unwrap_or_default();
    }
    if let Some(line) = string(args, "append_notes")? {
        task.notes = match task.notes.trim_end() {
            "" => line,
            notes => format!("{notes}\n{line}"),
        };
    }
    if let Some(repeat) = clearable(args, "repeat", model::parse_repeat)? {
        task.repeat = repeat;
    }

    Ok(out::task(&match flag(args, "dry_run")? {
        true => store.preview_update(&task)?,
        false => store.update(&task)?,
    }))
}

/// Switches the store to the list `name`. A list that doesn't exist is
/// created when `create` is set, and is a "not found" error otherwise.
fn use_list<B: Backend>(store: &mut Store<B>, name: &str, create: bool) -> Result<()> {
    if store.list_name() == name {
        return Ok(());
    }
    if !create && !store.backend().lists()?.iter().any(|list| list.name == name) {
        let message = format!("no Reminders list named \"{name}\"; list_lists shows the lists that exist");
        return Err(Error::not_found(message).into());
    }
    store.select_list(name)
}

// --- Reading arguments ------------------------------------------------------

fn invalid(message: impl Into<String>) -> Error {
    Error::invalid_input(message)
}

/// The argument `key`, unless it's missing or null.
fn given<'a>(args: &'a Args, key: &str) -> Option<&'a Value> {
    args.get(key).filter(|value| !value.is_null())
}

fn string(args: &Args, key: &str) -> Result<Option<String>> {
    match given(args, key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(invalid(format!("`{key}` must be a string")).into()),
    }
}

fn strings(args: &Args, key: &str) -> Result<Vec<String>> {
    match given(args, key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            Ok(items.iter().filter_map(|item| item.as_str().map(String::from)).collect())
        }
        Some(_) => Err(invalid(format!("`{key}` must be an array of strings")).into()),
    }
}

fn flag(args: &Args, key: &str) -> Result<bool> {
    match given(args, key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(invalid(format!("`{key}` must be true or false")).into()),
    }
}

/// An ID: an integer, or a string holding one (some clients send those).
fn as_id(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.trim().parse().ok())
}

fn id(args: &Args, key: &str) -> Result<i64> {
    given(args, key).and_then(as_id).ok_or_else(|| invalid(format!("`{key}` must be a task ID (an integer)")).into())
}

fn ids(args: &Args) -> Result<Vec<i64>> {
    let error = || invalid("`ids` must be a non-empty array of task IDs");
    let items = given(args, "ids").and_then(Value::as_array).filter(|items| !items.is_empty()).ok_or_else(error)?;
    Ok(items.iter().map(as_id).collect::<Option<_>>().ok_or_else(error)?)
}

/// The string argument `key` run through `parse`, if given.
fn parsed<T, E: std::fmt::Display>(
    args: &Args,
    key: &str,
    parse: impl Fn(&str) -> Result<T, E>,
) -> Result<Option<T>> {
    match string(args, key)? {
        None => Ok(None),
        Some(text) => parse(&text).map(Some).map_err(|e| invalid(format!("`{key}`: {e}")).into()),
    }
}

/// For fields that can be cleared: `None` when `key` is missing (leave it
/// alone), `Some(None)` when it's null (clear it), otherwise the parsed value.
fn clearable<T, E: std::fmt::Display>(
    args: &Args,
    key: &str,
    parse: impl Fn(&str) -> Result<T, E>,
) -> Result<Option<Option<T>>> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(_) => Ok(Some(parsed(args, key, parse)?)),
    }
}

#[cfg(test)]
mod tests {
    use todo::{ErrorKind, MemoryList, ReminderData};

    use super::*;

    fn store() -> Store<MemoryList> {
        Store::in_memory(MemoryList::new("Todo")).unwrap()
    }

    fn args(value: Value) -> Args {
        value.as_object().unwrap().clone()
    }

    fn run(store: &mut Store<MemoryList>, name: &str, arguments: Value) -> Result<Value> {
        call(store, "Todo", name, &args(arguments))
    }

    fn ok(store: &mut Store<MemoryList>, name: &str, arguments: Value) -> Value {
        run(store, name, arguments).unwrap_or_else(|e| panic!("{name} failed: {e:#}"))
    }

    fn kind(result: Result<Value>) -> ErrorKind {
        ErrorKind::of(&result.unwrap_err())
    }

    #[test]
    fn definitions_have_names_schemas_and_descriptions() {
        let tools = definitions();
        let names: Vec<&str> = tools.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            ["list_tasks", "get_task", "add_task", "update_task", "complete_task", "remove_task", "undo", "history", "list_lists"]
        );
        for tool in tools.as_array().unwrap() {
            assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
            assert!(tool["description"].as_str().unwrap().len() > 20);
        }
        assert!(exists("add_task") && !exists("delete_everything"));
    }

    #[test]
    fn add_get_and_list_return_the_cli_json() {
        let mut db = store();
        let added = ok(&mut db, "add_task", json!({
            "title": "Dentist", "due": "2026-10-23 15:00", "priority": "high",
            "tags": ["health"], "notes": "Bring card", "repeat": "every 2 weeks"
        }));
        assert_eq!(added, out::task(&db.get(1).unwrap().unwrap()));
        assert_eq!(
            (&added["id"], &added["due"], &added["due_time"], &added["priority"], &added["repeat"], &added["list"]),
            (&json!(1), &json!("2026-10-23"), &json!("15:00"), &json!("high"), &json!("every 2 weeks"), &json!("Todo"))
        );
        assert_eq!(ok(&mut db, "get_task", json!({ "id": 1 })), added);
        assert_eq!(ok(&mut db, "get_task", json!({ "id": "1" })), added, "IDs as strings work too");
        assert_eq!(ok(&mut db, "list_tasks", json!({})), json!([added]));
    }

    #[test]
    fn list_tasks_filters_and_sorts() {
        let mut db = store();
        let today = model::today();
        let day = |offset: i64| (today + chrono::TimeDelta::days(offset)).to_string();
        ok(&mut db, "add_task", json!({ "title": "Late", "due": day(-1), "priority": "low" }));
        ok(&mut db, "add_task", json!({ "title": "Soon", "due": day(2), "priority": "high", "tags": ["work"] }));
        ok(&mut db, "add_task", json!({ "title": "Undated" }));
        ok(&mut db, "complete_task", json!({ "ids": [3] }));
        let titles = |db: &mut Store<MemoryList>, filter: Value| -> Vec<String> {
            let tasks = ok(db, "list_tasks", filter);
            tasks.as_array().unwrap().iter().map(|t| t["title"].as_str().unwrap().to_string()).collect()
        };
        assert_eq!(titles(&mut db, json!({ "overdue": true })), ["Late"]);
        assert_eq!(titles(&mut db, json!({ "due_after": "today", "tag": "work" })), ["Soon"]);
        assert_eq!(titles(&mut db, json!({ "no_due": true })), ["Undated"]);
        assert_eq!(titles(&mut db, json!({ "pending": true, "sort": "priority" })), ["Soon", "Late"]);
        assert_eq!(titles(&mut db, json!({ "search": "SOON", "due_before": day(2) })), ["Soon"]);
    }

    #[test]
    fn update_task_changes_only_what_is_given() {
        let mut db = store();
        ok(&mut db, "add_task", json!({ "title": "Call", "due": "2026-10-23 15:00", "tags": ["a"], "notes": "x", "repeat": "week" }));
        let task = ok(&mut db, "update_task", json!({ "id": 1, "priority": "medium", "add_tags": ["b"], "remove_tags": ["a"], "append_notes": "y" }));
        assert_eq!((&task["priority"], &task["tags"], &task["notes"]), (&json!("medium"), &json!(["b"]), &json!("x\ny")));
        assert_eq!((&task["due_time"], &task["repeat"]), (&json!("15:00"), &json!("every week")));

        let task = ok(&mut db, "update_task", json!({ "id": 1, "due": null, "repeat": null, "notes": null }));
        assert_eq!((&task["due"], &task["repeat"], &task["notes"], &task["tags"]), (&Value::Null, &Value::Null, &json!(""), &json!(["b"])));
        let task = ok(&mut db, "update_task", json!({ "id": 1, "title": "Call mum", "due": "2026-10-24" }));
        assert_eq!((&task["title"], &task["due"], &task["due_time"]), (&json!("Call mum"), &json!("2026-10-24"), &Value::Null));
    }

    #[test]
    fn complete_and_remove_take_ids_and_undo_together() {
        let mut db = store();
        for title in ["a", "b", "c"] {
            ok(&mut db, "add_task", json!({ "title": title }));
        }
        let done = ok(&mut db, "complete_task", json!({ "ids": [1, 3] }));
        assert_eq!(done.as_array().unwrap().iter().map(|t| t["done"].clone()).collect::<Vec<_>>(), [true, true]);
        let removed = ok(&mut db, "remove_task", json!({ "ids": [2, 3] }));
        assert_eq!(removed.as_array().unwrap().len(), 2);
        assert_eq!(ok(&mut db, "undo", json!({})), json!({ "undone": "remove of tasks 2, 3" }));
        assert_eq!(ok(&mut db, "undo", json!({})), json!({ "undone": "done of tasks 1, 3" }));
        let history = ok(&mut db, "history", json!({ "limit": 2 }));
        assert_eq!(history.as_array().unwrap().len(), 2);
        assert_eq!(history[0]["action"], "add");
    }

    #[test]
    fn dry_runs_change_nothing() {
        let mut db = store();
        ok(&mut db, "add_task", json!({ "title": "Plants", "due": "2026-10-20", "repeat": "week" }));
        let before = (db.backend().all().unwrap().into_iter().map(|r| r.data).collect::<Vec<_>>(), db.history(10).unwrap());

        let preview = ok(&mut db, "update_task", json!({ "id": 1, "title": "Water plants", "dry_run": true }));
        assert_eq!(preview["title"], "Water plants");
        let preview = ok(&mut db, "complete_task", json!({ "ids": [1], "dry_run": true }));
        assert_eq!((&preview[0]["done"], &preview[0]["due"]), (&json!(false), &json!("2026-10-27")));
        ok(&mut db, "remove_task", json!({ "ids": [1], "dry_run": true }));

        let after = (db.backend().all().unwrap().into_iter().map(|r| r.data).collect::<Vec<_>>(), db.history(10).unwrap());
        assert_eq!(after, before);
    }

    #[test]
    fn lists_are_found_switched_and_only_created_when_asked() {
        let mut db = store();
        db.backend().push(ReminderData::new("Eggs"), "Groceries");
        ok(&mut db, "add_task", json!({ "title": "Report" }));

        assert_eq!(ok(&mut db, "list_lists", json!({})), json!([{ "name": "Groceries", "open": 1 }, { "name": "Todo", "open": 1 }]));
        let milk = ok(&mut db, "add_task", json!({ "title": "Milk", "list": "Groceries" }));
        assert_eq!(milk["list"], "Groceries");
        let groceries = ok(&mut db, "list_tasks", json!({ "list": "Groceries" }));
        assert_eq!(groceries.as_array().unwrap().len(), 2);
        // Without `list`, calls are back on the server's own list.
        assert_eq!(ok(&mut db, "list_tasks", json!({}))[0]["title"], "Report");

        assert_eq!(kind(run(&mut db, "add_task", json!({ "title": "x", "list": "Shopping" }))), ErrorKind::NotFound);
        assert_eq!(kind(run(&mut db, "list_tasks", json!({ "list": "Shopping" }))), ErrorKind::NotFound);
        ok(&mut db, "add_task", json!({ "title": "x", "list": "Shopping", "create_list": true }));
        assert_eq!(ok(&mut db, "list_lists", json!({})).as_array().unwrap().len(), 3);
    }

    #[test]
    fn bad_arguments_are_invalid_input_and_missing_tasks_not_found() {
        let mut db = store();
        ok(&mut db, "add_task", json!({ "title": "a" }));
        for (name, arguments) in [
            ("add_task", json!({})),
            ("add_task", json!({ "title": "  " })),
            ("add_task", json!({ "title": "a", "due": "someday" })),
            ("add_task", json!({ "title": "a", "priority": "urgent" })),
            ("add_task", json!({ "title": "a", "tags": "work" })),
            ("add_task", json!({ "title": 5 })),
            ("update_task", json!({ "id": 1 })),
            ("update_task", json!({ "id": "one", "title": "b" })),
            ("update_task", json!({ "id": 1, "repeat": "fortnight" })),
            ("complete_task", json!({ "ids": [] })),
            ("complete_task", json!({ "ids": 1 })),
            ("remove_task", json!({ "ids": [1], "dry_run": "yes" })),
            ("list_tasks", json!({ "sort": "title" })),
            ("history", json!({ "limit": 0 })),
        ] {
            assert_eq!(kind(run(&mut db, name, arguments.clone())), ErrorKind::InvalidInput, "{name} {arguments}");
        }
        assert_eq!(kind(run(&mut db, "get_task", json!({ "id": 9 }))), ErrorKind::NotFound);
        assert_eq!(kind(run(&mut db, "update_task", json!({ "id": 9, "title": "b" }))), ErrorKind::NotFound);
        assert_eq!(kind(run(&mut db, "complete_task", json!({ "ids": [1, 9] }))), ErrorKind::NotFound);
        assert!(!db.get(1).unwrap().unwrap().done, "nothing changed");
    }
}
