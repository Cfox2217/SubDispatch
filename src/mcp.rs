use crate::engine::SubDispatchEngine;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

const JSONRPC: &str = "2.0";

pub fn serve_stdio(workspace: PathBuf) -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|err| format!("failed to read stdin: {err}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                write_response(
                    &mut stdout,
                    Value::Null,
                    None,
                    Some(json!({
                        "code": -32700,
                        "message": format!("Parse error: {err}")
                    })),
                )?;
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        match handle_request(&workspace, &request) {
            Ok(Some(result)) => write_response(&mut stdout, id, Some(result), None)?,
            Ok(None) => {}
            Err(err) => write_response(
                &mut stdout,
                id,
                None,
                Some(json!({ "code": -32603, "message": err })),
            )?,
        }
    }
    Ok(())
}

fn handle_request(workspace: &PathBuf, request: &Value) -> Result<Option<Value>, String> {
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => Ok(Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "subdispatch", "version": env!("CARGO_PKG_VERSION") }
        }))),
        "notifications/initialized" => Ok(None),
        "tools/list" => Ok(Some(json!({ "tools": tool_schemas() }))),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let mut engine = SubDispatchEngine::new(workspace.clone())?;
            let result = match name {
                "list_workers" => engine.list_workers()?,
                "init_integration" => engine.init_integration()?,
                "start_run" => engine.start_run(arguments)?,
                "poll_run" => {
                    let run_id = required_arg(&arguments, "run_id")?;
                    engine.poll_run(&run_id)?
                }
                "collect_task" => {
                    let run_id = required_arg(&arguments, "run_id")?;
                    let task_id = required_arg(&arguments, "task_id")?;
                    engine.collect_task(&run_id, &task_id)?
                }
                "delete_worktree" => {
                    let run_id = required_arg(&arguments, "run_id")?;
                    let task_id = required_arg(&arguments, "task_id")?;
                    let force = arguments
                        .get("force")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let delete_branch = arguments
                        .get("delete_branch")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    engine.delete_worktree(&run_id, &task_id, force, delete_branch)?
                }
                _ => return Err(format!("Unknown tool: {name}")),
            };
            Ok(Some(json!({
                "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result).map_err(|err| err.to_string())? }]
            })))
        }
        _ => Err(format!("Unsupported method: {method}")),
    }
}

fn write_response(
    stdout: &mut io::Stdout,
    id: Value,
    result: Option<Value>,
    error: Option<Value>,
) -> Result<(), String> {
    let mut response = serde_json::Map::new();
    response.insert("jsonrpc".to_string(), Value::String(JSONRPC.to_string()));
    response.insert("id".to_string(), id);
    if let Some(error) = error {
        response.insert("error".to_string(), error);
    } else {
        response.insert("result".to_string(), result.unwrap_or(Value::Null));
    }
    writeln!(
        stdout,
        "{}",
        serde_json::to_string(&Value::Object(response)).map_err(|err| err.to_string())?
    )
    .map_err(|err| format!("failed to write stdout: {err}"))?;
    stdout
        .flush()
        .map_err(|err| format!("failed to flush stdout: {err}"))
}

fn required_arg(arguments: &Value, key: &str) -> Result<String, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("missing required argument: {key}"))
}

pub fn tool_schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "list_workers",
            "description": "List SubDispatch Claude Code workers and available concurrency slots.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "init_integration",
            "description": "Create or verify the persistent integration branch/worktree used as the default delegation base.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "start_run",
            "description": "Start multiple child coding-agent tasks in isolated git worktrees. When base is omitted, tasks start from the configured integration branch HEAD.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "goal": { "type": "string" },
                    "base": { "type": "string" },
                    "base_branch": { "type": "string" },
                    "run_id": { "type": "string" },
                    "tasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "instruction": { "type": "string" },
                                "worker": { "type": "string" },
                                "read_scope": { "type": "array", "items": { "type": "string" } },
                                "write_scope": { "type": "array", "items": { "type": "string" } },
                                "forbidden_paths": { "type": "array", "items": { "type": "string" } },
                                "context": { "type": "string" },
                                "context_files": { "type": "array", "items": { "type": "string" } }
                            },
                            "required": ["id", "instruction"]
                        }
                    }
                },
                "required": ["goal", "tasks"]
            }
        }),
        json!({
            "name": "poll_run",
            "description": "Poll status for all child tasks in a SubDispatch run.",
            "inputSchema": {
                "type": "object",
                "properties": { "run_id": { "type": "string" } },
                "required": ["run_id"]
            }
        }),
        json!({
            "name": "collect_task",
            "description": "Collect diff, logs, manifest, and scope checks for one child task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "run_id": { "type": "string" },
                    "task_id": { "type": "string" }
                },
                "required": ["run_id", "task_id"]
            }
        }),
        json!({
            "name": "delete_worktree",
            "description": "Delete one SubDispatch-managed task worktree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "run_id": { "type": "string" },
                    "task_id": { "type": "string" },
                    "force": { "type": "boolean" },
                    "delete_branch": { "type": "boolean" }
                },
                "required": ["run_id", "task_id"]
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_only_core_tools() {
        let names = tool_schemas()
            .into_iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "list_workers",
                "init_integration",
                "start_run",
                "poll_run",
                "collect_task",
                "delete_worktree"
            ]
        );
    }
}
