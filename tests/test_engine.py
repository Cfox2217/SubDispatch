from __future__ import annotations

import pytest
from unittest.mock import patch, MagicMock
from codexsaver.engine import CodexSaverEngine, DEFAULT_CONSTRAINTS


class TestCodexSaverEngine:
    def setup_method(self):
        self.engine = CodexSaverEngine()

    def test_delegate_task_routes_high_risk_to_codex(self):
        result = self.engine.delegate_task({
            "instruction": "fix security vulnerability",
            "files": ["src/auth/login.go"],
        })
        assert result["route"] == "codex"
        assert result["status"] == "needs_codex"
        assert result["estimated_savings_percent"] == 0

    def test_delegate_task_routes_unknown_to_codex(self):
        result = self.engine.delegate_task({
            "instruction": "make it production ready",
            "files": ["src/app.py"],
        })
        assert result["route"] == "codex"
        assert result["status"] == "needs_codex"

    def test_delegate_task_dry_run_returns_preview(self):
        result = self.engine.delegate_task({
            "instruction": "add unit tests for utils",
            "files": [],
            "dry_run": True,
        })
        assert result["status"] == "dry_run"
        assert result["route"] == "deepseek"
        assert "task_preview" in result
        assert "decision" in result

    def test_delegate_task_calls_deepseek(self):
        with patch("codexsaver.engine.DeepSeekClient") as MockClient:
            mock_instance = MagicMock()
            mock_instance.complete_task.return_value = {
                "status": "success",
                "summary": "added tests",
                "changed_files": ["tests/foo_test.py"],
                "patch": "diff",
                "commands_to_run": ["pytest"],
                "risk_notes": [],
            }
            MockClient.return_value = mock_instance

            result = self.engine.delegate_task({
                "instruction": "add unit tests for utils",
                "files": [],
            })

            assert result["route"] == "deepseek"
            assert result["status"] == "success"
            assert result["estimated_savings_percent"] > 0
            mock_instance.complete_task.assert_called_once()

    def test_delegate_task_deepseek_failure_returns_codex(self):
        with patch("codexsaver.engine.DeepSeekClient") as MockClient:
            from codexsaver.deepseek_client import DeepSeekError
            mock_instance = MagicMock()
            mock_instance.complete_task.side_effect = DeepSeekError("API error")
            MockClient.return_value = mock_instance

            result = self.engine.delegate_task({
                "instruction": "add unit tests for utils",
                "files": [],
            })

            assert result["route"] == "codex"
            assert result["status"] == "failed"

    def test_delegate_task_verification_failure(self):
        with patch("codexsaver.engine.DeepSeekClient") as MockClient:
            mock_instance = MagicMock()
            mock_instance.complete_task.return_value = {
                "status": "success",
                "summary": "changed auth",
                "changed_files": ["src/auth/login.go"],
                "patch": "diff",
                "commands_to_run": [],
                "risk_notes": [],
            }
            MockClient.return_value = mock_instance

            result = self.engine.delegate_task({
                "instruction": "refactor auth service",
                "files": ["src/auth/login.go"],
            })

            assert result["status"] == "needs_codex"

    def test_default_constraints_added(self):
        with patch("codexsaver.engine.DeepSeekClient") as MockClient:
            mock_instance = MagicMock()
            mock_instance.complete_task.return_value = {
                "status": "success",
                "summary": "done",
                "changed_files": [],
                "patch": "",
                "commands_to_run": [],
                "risk_notes": [],
            }
            MockClient.return_value = mock_instance

            result = self.engine.delegate_task({
                "instruction": "explain the code",
                "files": [],
                "constraints": ["be concise"],
            })

            mock_instance.complete_task.assert_called_once()
            task = mock_instance.complete_task.call_args[0][0]
            assert "be concise" in task.constraints
            for c in DEFAULT_CONSTRAINTS:
                assert c in task.constraints

    def test_max_files_parameter(self):
        with patch("codexsaver.engine.DeepSeekClient") as MockClient:
            mock_instance = MagicMock()
            mock_instance.complete_task.return_value = {
                "status": "success",
                "summary": "done",
                "changed_files": [],
                "patch": "",
                "commands_to_run": [],
                "risk_notes": [],
            }
            MockClient.return_value = mock_instance

            self.engine.delegate_task({
                "instruction": "add tests",
                "files": [],
                "max_files": 3,
            })

            task = mock_instance.complete_task.call_args[0][0]
            assert task.files == []
