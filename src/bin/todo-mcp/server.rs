//! The Model Context Protocol on top of the tools: JSON-RPC 2.0 messages,
//! one per line, as in MCP's stdio transport. Only what a tools-only server
//! needs is implemented: `initialize`, `ping`, `tools/list` and `tools/call`.

use anyhow::Result;
use serde_json::{Value, json};
use todo::{Backend, ErrorKind, Store, json as out};

use crate::tools;

/// Protocol versions this server speaks, newest first. A client asking for
/// one of these gets it; any other gets the newest.
const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Manages the user's tasks in Apple Reminders. Look up task IDs with \
    list_tasks or get_task instead of guessing them, and use list_lists to find the exact name of \
    a list the user mentions. For significant or ambiguous changes, call update_task, \
    complete_task or remove_task with dry_run first and confirm with the user. undo reverts the \
    most recent change.";

// JSON-RPC error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Opens the store; called on the first tool call, so that starting the
/// server and listing its tools doesn't need access to Reminders.
pub type Opener<B> = Box<dyn FnMut() -> Result<Store<B>>>;

pub struct Server<B: Backend> {
    open: Opener<B>,
    store: Option<Store<B>>,
    /// The list used when a tool call doesn't name one.
    default_list: String,
}

impl<B: Backend> Server<B> {
    pub fn new(default_list: &str, open: Opener<B>) -> Server<B> {
        Server { open, store: None, default_list: default_list.to_string() }
    }

    /// Handles one line of input and returns the line to answer with, if any
    /// (notifications get no answer).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let response = match serde_json::from_str::<Value>(line) {
            Err(e) => Some(error_response(Value::Null, PARSE_ERROR, &format!("invalid JSON: {e}"))),
            // A batch, which older protocol versions allowed.
            Ok(Value::Array(messages)) => {
                let responses: Vec<Value> = messages.into_iter().filter_map(|m| self.handle(m)).collect();
                (!responses.is_empty()).then_some(Value::Array(responses))
            }
            Ok(message) => self.handle(message),
        };
        response.map(|r| r.to_string())
    }

    /// Handles one JSON-RPC message.
    pub fn handle(&mut self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response to a request we never sent, or garbage.
            return id.map(|id| error_response(id, INVALID_REQUEST, "expected a request with a method"));
        };
        // Notifications (no id), such as `notifications/initialized`, need no answer.
        let id = id?;
        let params = message.get("params").cloned().unwrap_or(json!({}));
        let result = match method {
            "initialize" => Ok(initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::definitions() })),
            "tools/call" => self.call_tool(&params),
            _ => Err((METHOD_NOT_FOUND, format!("unknown method \"{method}\""))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => error_response(id, code, &message),
        })
    }

    /// Runs a tool. Problems with the call itself (an unknown tool) are
    /// protocol errors; everything that goes wrong in the tool is reported
    /// in the result with `isError`, so the model can read and fix it.
    fn call_tool(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
        if !tools::exists(name) {
            return Err((INVALID_PARAMS, format!("unknown tool \"{name}\"")));
        }
        let args = match params.get("arguments") {
            None | Some(Value::Null) => tools::Args::new(),
            Some(Value::Object(args)) => args.clone(),
            Some(_) => return Err((INVALID_PARAMS, "`arguments` must be an object".to_string())),
        };
        let default_list = self.default_list.clone();
        let result = self.store().and_then(|store| tools::call(store, &default_list, name, &args));
        Ok(match result {
            Ok(value) => tool_result(&value, false),
            Err(e) => tool_result(&out::error(ErrorKind::of(&e), &format!("{e:#}")), true),
        })
    }

    fn store(&mut self) -> Result<&mut Store<B>> {
        if self.store.is_none() {
            self.store = Some((self.open)()?);
        }
        Ok(self.store.as_mut().expect("just opened"))
    }
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested.filter(|v| PROTOCOL_VERSIONS.contains(v)).unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "todo", "title": "todo (Apple Reminders)", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

/// A `tools/call` result with `value` as indented JSON text.
fn tool_result(value: &Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use todo::MemoryList;

    use super::*;

    fn server() -> Server<MemoryList> {
        Server::new("Todo", Box::new(|| Store::in_memory(MemoryList::new("Todo"))))
    }

    fn request(server: &mut Server<MemoryList>, id: i64, method: &str, params: Value) -> Value {
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        let response: Value = serde_json::from_str(&server.handle_line(&line).unwrap()).unwrap();
        assert_eq!((&response["jsonrpc"], &response["id"]), (&json!("2.0"), &json!(id)));
        response
    }

    fn call(server: &mut Server<MemoryList>, name: &str, arguments: Value) -> (Value, bool) {
        let response = request(server, 9, "tools/call", json!({ "name": name, "arguments": arguments }));
        let result = &response["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        (serde_json::from_str(text).unwrap(), result["isError"].as_bool().unwrap())
    }

    #[test]
    fn initialize_agrees_on_a_protocol_version() {
        let mut server = server();
        let result = &request(&mut server, 1, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {} }))["result"];
        assert_eq!(result["protocolVersion"], "2025-06-18");
        assert_eq!(result["serverInfo"]["name"], "todo");
        assert!(result["capabilities"]["tools"].is_object());
        let result = &request(&mut server, 2, "initialize", json!({ "protocolVersion": "1999-01-01" }))["result"];
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn notifications_get_no_answer_and_ping_an_empty_one() {
        let mut server = server();
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string();
        assert_eq!(server.handle_line(&note), None);
        assert_eq!(request(&mut server, 1, "ping", json!({}))["result"], json!({}));
    }

    #[test]
    fn tools_list_does_not_open_the_store() {
        let mut server: Server<MemoryList> = Server::new("Todo", Box::new(|| panic!("opened")));
        let tools = &request(&mut server, 1, "tools/list", json!({}))["result"]["tools"];
        assert_eq!(tools.as_array().unwrap().len(), 9);
    }

    #[test]
    fn tool_calls_return_json_text() {
        let mut server = server();
        let (task, is_error) = call(&mut server, "add_task", json!({ "title": "Milk" }));
        assert!(!is_error);
        assert_eq!((&task["id"], &task["title"]), (&json!(1), &json!("Milk")));
        let (tasks, _) = call(&mut server, "list_tasks", json!({}));
        assert_eq!(tasks, json!([task]));
    }

    #[test]
    fn tool_errors_are_results_with_is_error() {
        let mut server = server();
        let (error, is_error) = call(&mut server, "get_task", json!({ "id": 7 }));
        assert!(is_error);
        assert_eq!(error, json!({ "error": "not_found", "message": "no task with id 7" }));
        let (error, _) = call(&mut server, "add_task", json!({ "title": "x", "due": "someday" }));
        assert_eq!(error["error"], "invalid_input");
    }

    #[test]
    fn failing_to_open_the_store_is_a_tool_error_and_retried() {
        let mut attempts = 0;
        let mut server: Server<MemoryList> = Server::new(
            "Todo",
            Box::new(move || {
                attempts += 1;
                match attempts {
                    1 => Err(todo::Error::new(ErrorKind::PermissionDenied, "no access to Reminders").into()),
                    _ => Store::in_memory(MemoryList::new("Todo")),
                }
            }),
        );
        let (error, is_error) = call(&mut server, "list_tasks", json!({}));
        assert!(is_error);
        assert_eq!(error["error"], "permission_denied");
        assert_eq!(call(&mut server, "list_tasks", json!({})), (json!([]), false));
    }

    #[test]
    fn protocol_errors() {
        let mut server = server();
        let code = |response: &Value| response["error"]["code"].as_i64().unwrap();
        assert_eq!(code(&request(&mut server, 1, "resources/list", json!({}))), METHOD_NOT_FOUND);
        assert_eq!(code(&request(&mut server, 2, "tools/call", json!({ "name": "nope" }))), INVALID_PARAMS);
        assert_eq!(code(&request(&mut server, 3, "tools/call", json!({ "name": "undo", "arguments": [] }))), INVALID_PARAMS);

        let response: Value = serde_json::from_str(&server.handle_line("{not json").unwrap()).unwrap();
        assert_eq!((code(&response), &response["id"]), (PARSE_ERROR, &Value::Null));
        let response: Value = serde_json::from_str(&server.handle_line(r#"{"jsonrpc":"2.0","id":4}"#).unwrap()).unwrap();
        assert_eq!(code(&response), INVALID_REQUEST);
    }

    #[test]
    fn batches_get_an_array_of_answers() {
        let mut server = server();
        let batch = json!([
            { "jsonrpc": "2.0", "id": 1, "method": "ping" },
            { "jsonrpc": "2.0", "method": "notifications/initialized" },
            { "jsonrpc": "2.0", "id": 2, "method": "ping" }
        ]);
        let response: Value = serde_json::from_str(&server.handle_line(&batch.to_string()).unwrap()).unwrap();
        assert_eq!(response.as_array().unwrap().iter().map(|r| r["id"].clone()).collect::<Vec<_>>(), [1, 2]);
    }
}
