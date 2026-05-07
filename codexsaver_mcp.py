#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
import traceback
from typing import Any, Dict

from codexsaver.engine import CodexSaverEngine

JSONRPC = "2.0"


def respond(id_: Any, result: Any = None, error: Any = None) -> None:
    msg: Dict[str, Any] = {"jsonrpc": JSONRPC, "id": id_}
    if error is not None:
        msg["error"] = error
    else:
        msg["result"] = result
    print(json.dumps(msg, ensure_ascii=False), flush=True)


def tool_schema() -> Dict[str, Any]:
    return {
        "name": "delegate_task",
        "description": (
            "Delegate low-risk coding tasks to DeepSeek API to reduce Codex cost. "
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


def handle(request: Dict[str, Any], engine: CodexSaverEngine) -> None:
    method = request.get("method")
    id_ = request.get("id")
    if method == "initialize":
        respond(id_, {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                     "serverInfo": {"name": "codexsaver", "version": "0.2.0"}})
        return
    if method == "notifications/initialized":
        return
    if method == "tools/list":
        respond(id_, {"tools": [tool_schema()]})
        return
    if method == "tools/call":
        params = request.get("params", {})
        name = params.get("name")
        arguments = params.get("arguments", {})
        if name != "delegate_task":
            respond(id_, error={"code": -32601, "message": f"Unknown tool: {name}"})
            return
        result = engine.delegate_task(arguments)
        respond(id_, {"content": [{"type": "text", "text": json.dumps(result, ensure_ascii=False, indent=2)}]})
        return
    respond(id_, error={"code": -32601, "message": f"Unsupported method: {method}"})


def main() -> int:
    engine = CodexSaverEngine()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            handle(request, engine)
        except Exception as e:
            traceback.print_exc(file=sys.stderr)
            id_ = request.get("id") if "request" in locals() and isinstance(request, dict) else None
            respond(id_, error={"code": -32603, "message": str(e)})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
