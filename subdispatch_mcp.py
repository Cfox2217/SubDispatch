#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
import traceback
from typing import Any, Dict

from subdispatch.engine import SubDispatchDelegateEngine
from subdispatch.subdispatch import SubDispatchEngine

JSONRPC = "2.0"


def respond(id_: Any, result: Any = None, error: Any = None) -> None:
    msg: Dict[str, Any] = {"jsonrpc": JSONRPC, "id": id_}
    if error is not None:
        msg["error"] = error
    else:
        msg["result"] = result
    print(json.dumps(msg, ensure_ascii=False), flush=True)


def delegate_task_tool_schema() -> Dict[str, Any]:
    return {
        "name": "delegate_task",
        "description": (
            "Delegate low-risk coding tasks to a configured low-cost LLM provider to reduce Codex cost. "
            "Use for tests, docs, code search, explanations, lint fixes, boilerplate, "
            "and small refactors. Do not use for high-risk architecture/security/payment/migration tasks."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "instruction": {"type": "string", "description": "The coding task to delegate."},
                "files": {"type": "array", "items": {"type": "string"},
                          "description": "File paths to include as bounded context."},
                "constraints": {"type": "array", "items": {"type": "string"},
                                "description": "Extra safety or output constraints."},
                "workspace": {"type": "string",
                              "description": "Workspace root used to resolve relative file paths and run verification commands."},
                "max_files": {"type": "integer", "minimum": 1,
                              "description": "Maximum number of files to include in delegated context."},
                "max_chars_per_file": {"type": "integer", "minimum": 1,
                                       "description": "Maximum characters loaded per file."},
                "max_total_chars": {"type": "integer", "minimum": 1,
                                    "description": "Maximum total characters loaded across all files."},
                "dry_run": {"type": "boolean",
                            "description": "If true, only show routing decision and task preview."}
            },
            "required": ["instruction"]
        }
    }


def subdispatch_tool_schemas() -> list[Dict[str, Any]]:
    return [
        {
            "name": "list_workers",
            "description": "List SubDispatch Claude Code workers and available concurrency slots.",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "start_run",
            "description": "Start multiple child coding-agent tasks in isolated git worktrees.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "goal": {"type": "string"},
                    "base": {"type": "string"},
                    "base_branch": {"type": "string"},
                    "run_id": {"type": "string"},
                    "tasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {"type": "string"},
                                "instruction": {"type": "string"},
                                "worker": {"type": "string"},
                                "read_scope": {"type": "array", "items": {"type": "string"}},
                                "write_scope": {"type": "array", "items": {"type": "string"}},
                                "forbidden_paths": {"type": "array", "items": {"type": "string"}},
                                "context": {"type": "string"},
                                "context_files": {"type": "array", "items": {"type": "string"}},
                            },
                            "required": ["id", "instruction"],
                        },
                    },
                },
                "required": ["goal", "tasks"],
            },
        },
        {
            "name": "poll_run",
            "description": "Poll status for all child tasks in a SubDispatch run.",
            "inputSchema": {
                "type": "object",
                "properties": {"run_id": {"type": "string"}},
                "required": ["run_id"],
            },
        },
        {
            "name": "collect_task",
            "description": "Collect diff, logs, manifest, and scope checks for one child task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "run_id": {"type": "string"},
                    "task_id": {"type": "string"},
                },
                "required": ["run_id", "task_id"],
            },
        },
        {
            "name": "delete_worktree",
            "description": "Delete one SubDispatch-managed task worktree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "run_id": {"type": "string"},
                    "task_id": {"type": "string"},
                    "force": {"type": "boolean"},
                    "delete_branch": {"type": "boolean"},
                },
                "required": ["run_id", "task_id"],
            },
        },
    ]


def handle(request: Dict[str, Any], engine: SubDispatchDelegateEngine,
           subdispatch: SubDispatchEngine) -> None:
    method = request.get("method")
    id_ = request.get("id")
    if method == "initialize":
        respond(id_, {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                     "serverInfo": {"name": "subdispatch", "version": "0.2.0"}})
        return
    if method == "notifications/initialized":
        return
    if method == "tools/list":
        respond(id_, {"tools": [delegate_task_tool_schema(), *subdispatch_tool_schemas()]})
        return
    if method == "tools/call":
        params = request.get("params", {})
        name = params.get("name")
        arguments = params.get("arguments", {})
        if name == "delegate_task":
            result = engine.delegate_task(arguments)
            respond(id_, {"content": [{"type": "text", "text": json.dumps(result, ensure_ascii=False, indent=2)}]})
            return
        if name == "list_workers":
            result = subdispatch.list_workers()
            respond(id_, {"content": [{"type": "text", "text": json.dumps(result, ensure_ascii=False, indent=2)}]})
            return
        if name == "start_run":
            result = subdispatch.start_run(arguments)
            respond(id_, {"content": [{"type": "text", "text": json.dumps(result, ensure_ascii=False, indent=2)}]})
            return
        if name == "poll_run":
            result = subdispatch.poll_run(arguments)
            respond(id_, {"content": [{"type": "text", "text": json.dumps(result, ensure_ascii=False, indent=2)}]})
            return
        if name == "collect_task":
            result = subdispatch.collect_task(arguments)
            respond(id_, {"content": [{"type": "text", "text": json.dumps(result, ensure_ascii=False, indent=2)}]})
            return
        if name == "delete_worktree":
            result = subdispatch.delete_worktree(arguments)
            respond(id_, {"content": [{"type": "text", "text": json.dumps(result, ensure_ascii=False, indent=2)}]})
            return
        else:
            respond(id_, error={"code": -32601, "message": f"Unknown tool: {name}"})
            return
    respond(id_, error={"code": -32601, "message": f"Unsupported method: {method}"})


def main() -> int:
    engine = SubDispatchDelegateEngine()
    subdispatch = SubDispatchEngine()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            handle(request, engine, subdispatch)
        except Exception as e:
            traceback.print_exc(file=sys.stderr)
            id_ = request.get("id") if "request" in locals() and isinstance(request, dict) else None
            respond(id_, error={"code": -32603, "message": str(e)})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
