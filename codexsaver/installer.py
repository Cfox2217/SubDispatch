from __future__ import annotations

import os
import re
from pathlib import Path
from typing import Any, Dict

from .config import CONFIG_PATH, mask_secret, resolve_api_key

SECTION_RE = re.compile(
    r"(?ms)^\[mcp_servers\.codexsaver\]\n.*?(?=^\[|\Z)"
)


def render_mcp_config(script_path: str) -> str:
    return (
        "[mcp_servers.codexsaver]\n"
        'command = "python"\n'
        f'args = ["{script_path}"]\n'
        "startup_timeout_sec = 10\n"
        "tool_timeout_sec = 120\n"
    )


def install_config(config_path: str, script_path: str) -> Dict[str, Any]:
    path = Path(config_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    new_section = render_mcp_config(script_path).rstrip() + "\n"
    existed = path.exists()
    previous = path.read_text(encoding="utf-8") if existed else ""
    replaced = bool(SECTION_RE.search(previous))
    if replaced:
        updated = SECTION_RE.sub(new_section, previous, count=1)
    else:
        updated = previous
        if updated and not updated.endswith("\n"):
            updated += "\n"
        updated += new_section
    changed = updated != previous
    if changed:
        path.write_text(updated, encoding="utf-8")
    return {
        "config_path": str(path),
        "script_path": script_path,
        "changed": changed,
        "mode": "updated" if replaced else "created",
    }


def doctor(workspace: str) -> Dict[str, Any]:
    root = Path(workspace).resolve()
    script_path = root / "codexsaver_mcp.py"
    project_config = root / ".codex" / "config.toml"
    global_config = Path.home() / ".codex" / "config.toml"
    api_key, api_key_source = resolve_api_key()
    return {
        "workspace": str(root),
        "script_exists": script_path.exists(),
        "script_path": str(script_path),
        "project_config_path": str(project_config),
        "project_config_exists": project_config.exists(),
        "global_config_path": str(global_config),
        "global_config_exists": global_config.exists(),
        "local_config_path": str(CONFIG_PATH),
        "local_config_exists": CONFIG_PATH.exists(),
        "deepseek_api_key_configured": bool(api_key),
        "deepseek_api_key_source": api_key_source,
        "deepseek_api_key_preview": mask_secret(api_key),
        "recommended_next_step": _recommended_next_step(
            script_exists=script_path.exists(),
            project_config_exists=project_config.exists(),
            api_key_configured=bool(api_key),
        ),
    }


def _recommended_next_step(script_exists: bool, project_config_exists: bool,
                           api_key_configured: bool) -> str:
    if not script_exists:
        return "Run this command from the CodexSaver project root."
    if not project_config_exists:
        return "Run `python cli.py install --project` to enable CodexSaver in this workspace."
    if not api_key_configured:
        return "Export DEEPSEEK_API_KEY before making live delegated calls."
    return "CodexSaver is ready. Open this workspace in Codex and call codexsaver.delegate_task."
