from __future__ import annotations

import pytest


@pytest.fixture(autouse=True)
def isolate_codexsaver_config(monkeypatch, tmp_path):
    monkeypatch.setattr("codexsaver.config.CONFIG_PATH", tmp_path / "config.json")
    monkeypatch.setattr("codexsaver.installer.CONFIG_PATH", tmp_path / "config.json")
    for name in (
        "CODEXSAVER_PROVIDER",
        "CODEXSAVER_API_KEY",
        "CODEXSAVER_BASE_URL",
        "CODEXSAVER_MODEL",
        "DEEPSEEK_API_KEY",
        "DEEPSEEK_BASE_URL",
        "DEEPSEEK_MODEL",
    ):
        monkeypatch.delenv(name, raising=False)
