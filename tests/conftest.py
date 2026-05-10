from __future__ import annotations

import pytest


@pytest.fixture(autouse=True)
def isolate_subdispatch_config(monkeypatch, tmp_path):
    monkeypatch.setattr("subdispatch.config.CONFIG_PATH", tmp_path / "config.json")
    monkeypatch.setattr("subdispatch.installer.CONFIG_PATH", tmp_path / "config.json")
    for name in (
        "SUBDISPATCH_PROVIDER",
        "SUBDISPATCH_API_KEY",
        "SUBDISPATCH_BASE_URL",
        "SUBDISPATCH_MODEL",
        "DEEPSEEK_API_KEY",
        "DEEPSEEK_BASE_URL",
        "DEEPSEEK_MODEL",
    ):
        monkeypatch.delenv(name, raising=False)
