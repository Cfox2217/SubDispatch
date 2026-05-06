#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json

from codexsaver.engine import CodexSaverEngine


def main() -> int:
    parser = argparse.ArgumentParser(description="CodexSaver CLI")
    parser.add_argument("instruction")
    parser.add_argument("--files", nargs="*", default=[])
    parser.add_argument("--constraint", action="append", default=[])
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    result = CodexSaverEngine().delegate_task({
        "instruction": args.instruction,
        "files": args.files,
        "constraints": args.constraint,
        "dry_run": args.dry_run,
    })
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
