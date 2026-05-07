from __future__ import annotations

import json
from pathlib import Path

from cli import main
from codexsaver.installer import doctor, install_config, render_mcp_config


def test_render_mcp_config():
    text = render_mcp_config("./codexsaver_mcp.py")
    assert '[mcp_servers.codexsaver]' in text
    assert 'args = ["./codexsaver_mcp.py"]' in text


def test_install_config_creates_file(tmp_path):
    config_path = tmp_path / ".codex" / "config.toml"
    result = install_config(str(config_path), "./codexsaver_mcp.py")
    assert result["changed"] is True
    assert config_path.exists()
    assert "codexsaver" in config_path.read_text(encoding="utf-8")


def test_install_config_replaces_existing_section(tmp_path):
    config_path = tmp_path / "config.toml"
    config_path.write_text(
        "[mcp_servers.codexsaver]\ncommand = \"python\"\nargs = [\"old.py\"]\n\n[other]\nvalue = 1\n",
        encoding="utf-8",
    )
    install_config(str(config_path), "./codexsaver_mcp.py")
    text = config_path.read_text(encoding="utf-8")
    assert 'args = ["./codexsaver_mcp.py"]' in text
    assert 'args = ["old.py"]' not in text
    assert "[other]" in text


def test_doctor_reports_missing_setup(tmp_path, monkeypatch):
    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    monkeypatch.setattr("codexsaver.installer.CONFIG_PATH", tmp_path / "missing.json")
    result = doctor(str(tmp_path))
    assert result["script_exists"] is False
    assert "project root" in result["recommended_next_step"]


def test_cli_install_and_doctor(tmp_path, monkeypatch, capsys):
    (tmp_path / "codexsaver_mcp.py").write_text("print('ok')\n", encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    assert main(["install", "--project", "--workspace", str(tmp_path)]) == 0
    install_output = json.loads(capsys.readouterr().out)
    assert install_output["status"] == "ok"
    assert Path(install_output["actions"][0]["config_path"]).exists()

    monkeypatch.setenv("DEEPSEEK_API_KEY", "test-key")
    monkeypatch.setattr("codexsaver.installer.CONFIG_PATH", tmp_path / ".codexsaver-config.json")
    assert main(["doctor", "--workspace", str(tmp_path)]) == 0
    doctor_output = json.loads(capsys.readouterr().out)
    assert doctor_output["project_config_exists"] is True
    assert doctor_output["deepseek_api_key_configured"] is True
    assert doctor_output["deepseek_api_key_source"] == "environment"


def test_cli_auth_set(tmp_path, monkeypatch, capsys):
    config_path = tmp_path / "saved-config.json"
    monkeypatch.setattr("codexsaver.config.CONFIG_PATH", config_path)
    assert main(["auth", "set", "--api-key", "sk-test-key"]) == 0
    auth_output = json.loads(capsys.readouterr().out)
    assert auth_output["deepseek_api_key_saved"] is True
    assert auth_output["deepseek_api_key_preview"] == "sk-t...-key"
    assert config_path.exists()
