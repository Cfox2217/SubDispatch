from __future__ import annotations

import json
import os
import stat
from pathlib import Path
from typing import Any, Dict


CONFIG_DIR = Path.home() / ".codexsaver"
CONFIG_PATH = CONFIG_DIR / "config.json"


def load_config(config_path: str | None = None) -> Dict[str, Any]:
    path = Path(config_path).expanduser() if config_path else CONFIG_PATH
    if not path.exists():
        return {}
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return {}


def save_config(data: Dict[str, Any], config_path: str | None = None) -> str:
    path = Path(config_path).expanduser() if config_path else CONFIG_PATH
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    os.chmod(path, stat.S_IRUSR | stat.S_IWUSR)
    return str(path)


def save_api_key(api_key: str, config_path: str | None = None) -> Dict[str, Any]:
    config = load_config(config_path)
    config["deepseek_api_key"] = api_key
    path = save_config(config, config_path)
    return {
        "config_path": path,
        "key_preview": mask_secret(api_key),
    }


def resolve_api_key(explicit_api_key: str | None = None) -> tuple[str | None, str | None]:
    if explicit_api_key:
        return explicit_api_key, "argument"
    env_api_key = os.environ.get("DEEPSEEK_API_KEY")
    if env_api_key:
        return env_api_key, "environment"
    config_api_key = load_config().get("deepseek_api_key")
    if config_api_key:
        return config_api_key, "local_config"
    return None, None


def mask_secret(value: str | None) -> str | None:
    if not value:
        return None
    if len(value) <= 8:
        return "*" * len(value)
    return f"{value[:4]}...{value[-4:]}"
