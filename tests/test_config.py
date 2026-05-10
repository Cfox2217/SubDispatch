from __future__ import annotations

from subdispatch.config import (
    load_config,
    mask_secret,
    resolve_api_key,
    resolve_provider_config,
    save_api_key,
    save_provider_config,
)


def test_mask_secret():
    assert mask_secret("sk-1234567890") == "sk-1...7890"


def test_save_and_load_api_key(tmp_path):
    config_path = tmp_path / "config.json"
    report = save_api_key("sk-test-value", str(config_path))
    assert report["key_preview"] == "sk-t...alue"
    assert load_config(str(config_path))["providers"]["deepseek"]["api_key"] == "sk-test-value"


def test_resolve_api_key_from_argument(monkeypatch, tmp_path):
    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    monkeypatch.setattr("subdispatch.config.CONFIG_PATH", tmp_path / "missing.json")
    api_key, source = resolve_api_key("sk-arg")
    assert api_key == "sk-arg"
    assert source == "argument"


def test_resolve_api_key_from_environment(monkeypatch, tmp_path):
    monkeypatch.setenv("DEEPSEEK_API_KEY", "sk-env")
    monkeypatch.setattr("subdispatch.config.CONFIG_PATH", tmp_path / "missing.json")
    api_key, source = resolve_api_key()
    assert api_key == "sk-env"
    assert source == "environment:DEEPSEEK_API_KEY"


def test_resolve_api_key_from_local_config(monkeypatch, tmp_path):
    config_path = tmp_path / "config.json"
    save_api_key("sk-local", str(config_path))
    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    monkeypatch.setattr("subdispatch.config.CONFIG_PATH", config_path)
    api_key, source = resolve_api_key()
    assert api_key == "sk-local"
    assert source == "local_config:deepseek"


def test_save_and_resolve_openai_provider(monkeypatch, tmp_path):
    config_path = tmp_path / "config.json"
    save_provider_config(
        provider="openai",
        api_key="sk-openai",
        model="gpt-test",
        config_path=str(config_path),
    )
    monkeypatch.setattr("subdispatch.config.CONFIG_PATH", config_path)
    provider = resolve_provider_config()
    assert provider.name == "openai"
    assert provider.api_key == "sk-openai"
    assert provider.model == "gpt-test"
    assert provider.base_url == "https://api.openai.com/v1/chat/completions"


def test_generic_env_overrides_provider_key(monkeypatch, tmp_path):
    monkeypatch.setenv("SUBDISPATCH_PROVIDER", "gemini")
    monkeypatch.setenv("SUBDISPATCH_API_KEY", "sk-generic")
    monkeypatch.setenv("SUBDISPATCH_MODEL", "gemini-test")
    monkeypatch.setattr("subdispatch.config.CONFIG_PATH", tmp_path / "missing.json")
    provider = resolve_provider_config()
    assert provider.name == "gemini"
    assert provider.api_key == "sk-generic"
    assert provider.model == "gemini-test"


def test_anthropic_preset_uses_native_messages_api(monkeypatch, tmp_path):
    monkeypatch.setenv("SUBDISPATCH_PROVIDER", "anthropic")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant")
    monkeypatch.setattr("subdispatch.config.CONFIG_PATH", tmp_path / "missing.json")
    provider = resolve_provider_config()
    assert provider.api_style == "anthropic"
    assert provider.base_url == "https://api.anthropic.com/v1/messages"
    assert provider.api_key == "sk-ant"


def test_local_provider_can_skip_api_key(monkeypatch, tmp_path):
    monkeypatch.setenv("SUBDISPATCH_PROVIDER", "ollama")
    monkeypatch.setattr("subdispatch.config.CONFIG_PATH", tmp_path / "missing.json")
    provider = resolve_provider_config()
    assert provider.requires_api_key is False
    assert provider.api_key is None


def test_custom_provider_requires_custom_base_url(monkeypatch, tmp_path):
    config_path = tmp_path / "config.json"
    save_provider_config(
        provider="custom",
        api_key="sk-custom",
        model="custom-model",
        base_url="https://llm.example.test/v1/chat/completions",
        config_path=str(config_path),
    )
    monkeypatch.setattr("subdispatch.config.CONFIG_PATH", config_path)
    provider = resolve_provider_config()
    assert provider.name == "custom"
    assert provider.base_url == "https://llm.example.test/v1/chat/completions"


def test_subdispatch_env_overrides_local_config(monkeypatch, tmp_path):
    config_path = tmp_path / "config.json"
    save_provider_config(
        provider="openai",
        api_key="sk-openai",
        model="gpt-test",
        config_path=str(config_path),
    )
    monkeypatch.setenv("SUBDISPATCH_PROVIDER", "gemini")
    monkeypatch.setenv("SUBDISPATCH_API_KEY", "preferred-key")
    monkeypatch.setenv("SUBDISPATCH_MODEL", "preferred-model")
    monkeypatch.setattr("subdispatch.config.CONFIG_PATH", config_path)

    provider = resolve_provider_config()

    assert provider.name == "gemini"
    assert provider.api_key == "preferred-key"
    assert provider.api_key_source == "environment:SUBDISPATCH_API_KEY"
    assert provider.model == "preferred-model"
    assert provider.model_source == "environment:SUBDISPATCH_MODEL"
