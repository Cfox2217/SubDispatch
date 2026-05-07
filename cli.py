#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from codexsaver.engine import CodexSaverEngine
from codexsaver.config import save_api_key
from codexsaver.installer import doctor, install_config


def main(argv: list[str] | None = None) -> int:
    argv = argv or sys.argv[1:]
    if argv and argv[0] in {"install", "doctor", "delegate", "auth"}:
        return _run_subcommand(argv)
    return _run_delegate(argv)


def _run_delegate(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="CodexSaver CLI",
        epilog=(
            "Quick setup: `python cli.py install --project` then "
            "`python cli.py doctor`."
        ),
    )
    parser.add_argument("instruction")
    parser.add_argument("--files", nargs="*", default=[])
    parser.add_argument("--constraint", action="append", default=[])
    parser.add_argument("--workspace", default=".")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)

    result = CodexSaverEngine().delegate_task({
        "instruction": args.instruction,
        "files": args.files,
        "constraints": args.constraint,
        "workspace": args.workspace,
        "dry_run": args.dry_run,
    })
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


def _run_subcommand(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="CodexSaver setup and diagnostics")
    subparsers = parser.add_subparsers(dest="command", required=True)

    install_parser = subparsers.add_parser(
        "install",
        help="Write Codex MCP config for this project or globally.",
    )
    install_parser.add_argument("--workspace", default=".")
    install_parser.add_argument("--project", action="store_true")
    install_parser.add_argument("--global", dest="global_install", action="store_true")

    doctor_parser = subparsers.add_parser(
        "doctor",
        help="Check whether CodexSaver is ready in this workspace.",
    )
    doctor_parser.add_argument("--workspace", default=".")

    auth_parser = subparsers.add_parser(
        "auth",
        help="Persist a DeepSeek API key locally so it does not need to be exported every time.",
    )
    auth_subparsers = auth_parser.add_subparsers(dest="auth_command", required=True)
    auth_set_parser = auth_subparsers.add_parser("set", help="Save a DeepSeek API key locally.")
    auth_set_parser.add_argument("--api-key", required=True)

    delegate_parser = subparsers.add_parser(
        "delegate",
        help="Explicit delegation command equivalent to the default CLI mode.",
    )
    delegate_parser.add_argument("instruction")
    delegate_parser.add_argument("--files", nargs="*", default=[])
    delegate_parser.add_argument("--constraint", action="append", default=[])
    delegate_parser.add_argument("--workspace", default=".")
    delegate_parser.add_argument("--dry-run", action="store_true")

    args = parser.parse_args(argv)

    if args.command == "install":
        workspace = Path(args.workspace).resolve()
        script_path = str((workspace / "codexsaver_mcp.py").resolve())
        install_project = args.project or not args.global_install
        reports = []
        if install_project:
            reports.append(install_config(
                str(workspace / ".codex" / "config.toml"),
                "./codexsaver_mcp.py",
            ))
        if args.global_install:
            reports.append(install_config(
                str(Path.home() / ".codex" / "config.toml"),
                script_path,
            ))
        print(json.dumps({
            "status": "ok",
            "workspace": str(workspace),
            "actions": reports,
            "next_step": "Run `python cli.py doctor` to verify the installation.",
        }, ensure_ascii=False, indent=2))
        return 0

    if args.command == "doctor":
        print(json.dumps(doctor(args.workspace), ensure_ascii=False, indent=2))
        return 0

    if args.command == "auth":
        report = save_api_key(args.api_key)
        print(json.dumps({
            "status": "ok",
            "config_path": report["config_path"],
            "deepseek_api_key_saved": True,
            "deepseek_api_key_preview": report["key_preview"],
            "next_step": "Run `python cli.py doctor` to verify CodexSaver can see the saved key.",
        }, ensure_ascii=False, indent=2))
        return 0

    result = CodexSaverEngine().delegate_task({
        "instruction": args.instruction,
        "files": args.files,
        "constraints": args.constraint,
        "workspace": args.workspace,
        "dry_run": args.dry_run,
    })
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
