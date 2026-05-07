from __future__ import annotations

from codexsaver.config import load_config, mask_secret, resolve_api_key, save_api_key


def test_mask_secret():
    assert mask_secret("sk-1234567890") == "sk-1...7890"


def test_save_and_load_api_key(tmp_path):
    config_path = tmp_path / "config.json"
    report = save_api_key("sk-test-value", str(config_path))
    assert report["key_preview"] == "sk-t...alue"
    assert load_config(str(config_path))["deepseek_api_key"] == "sk-test-value"


def test_resolve_api_key_from_argument(monkeypatch, tmp_path):
    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    monkeypatch.setattr("codexsaver.config.CONFIG_PATH", tmp_path / "missing.json")
    api_key, source = resolve_api_key("sk-arg")
    assert api_key == "sk-arg"
    assert source == "argument"


def test_resolve_api_key_from_environment(monkeypatch, tmp_path):
    monkeypatch.setenv("DEEPSEEK_API_KEY", "sk-env")
    monkeypatch.setattr("codexsaver.config.CONFIG_PATH", tmp_path / "missing.json")
    api_key, source = resolve_api_key()
    assert api_key == "sk-env"
    assert source == "environment"


def test_resolve_api_key_from_local_config(monkeypatch, tmp_path):
    config_path = tmp_path / "config.json"
    save_api_key("sk-local", str(config_path))
    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    monkeypatch.setattr("codexsaver.config.CONFIG_PATH", config_path)
    api_key, source = resolve_api_key()
    assert api_key == "sk-local"
    assert source == "local_config"
