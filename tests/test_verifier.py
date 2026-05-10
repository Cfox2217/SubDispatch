from __future__ import annotations

import os
import sys
import tempfile

import pytest
from subdispatch.verifier import Verifier, REQUIRED_KEYS, VALID_STATUS
from subdispatch.schema import RouteDecision


class TestVerifier:
    def setup_method(self):
        self.verifier = Verifier()

    def make_result(self, **kwargs):
        result = {
            "status": "success",
            "summary": "added tests",
            "changed_files": ["tests/foo_test.py"],
            "patch": "diff here",
            "commands_to_run": [f'"{sys.executable}" -c "print(\'ok\')"'],
            "risk_notes": [],
        }
        result.update(kwargs)
        return result

    def make_decision(self, **kwargs):
        decision = RouteDecision(
            route="deepseek",
            task_type="write_tests",
            risk="low",
            reason="test",
            protected_hits=[],
        )
        for k, v in kwargs.items():
            setattr(decision, k, v)
        return decision

    def test_verify_ok_minimal(self):
        result = self.make_result()
        decision = self.make_decision()
        v = self.verifier.verify(result, decision)
        assert v.ok is True
        assert v.fallback_to_codex is False
        assert "print('ok')" in v.executed_commands[0]["command"]

    def test_verify_missing_required_key(self):
        result = self.make_result()
        del result["summary"]
        decision = self.make_decision()
        v = self.verifier.verify(result, decision)
        assert v.ok is False
        assert v.fallback_to_codex is True
        assert "missing" in v.reason.lower()

    def test_verify_invalid_status(self):
        result = self.make_result(status="invalid_status")
        decision = self.make_decision()
        v = self.verifier.verify(result, decision)
        assert v.ok is False
        assert "invalid" in v.reason.lower()

    def test_verify_needs_codex_status(self):
        result = self.make_result(status="needs_codex")
        decision = self.make_decision()
        v = self.verifier.verify(result, decision)
        assert v.ok is False
        assert v.fallback_to_codex is True

    def test_verify_changed_files_not_list(self):
        result = self.make_result(changed_files="not a list")
        decision = self.make_decision()
        v = self.verifier.verify(result, decision)
        assert v.ok is False
        assert "list" in v.reason.lower()

    def test_verify_protected_file_high_risk(self):
        result = self.make_result(changed_files=["auth/login.go"])
        decision = self.make_decision(risk="high")
        v = self.verifier.verify(result, decision)
        assert v.ok is False
        assert "protected" in v.reason.lower()

    def test_verify_protected_file_low_risk_allowed(self):
        result = self.make_result(changed_files=["auth/login.go"])
        decision = self.make_decision(risk="low")
        v = self.verifier.verify(result, decision)
        assert v.ok is True

    def test_verify_patch_too_large(self):
        result = self.make_result(patch="x" * 130_000)
        decision = self.make_decision()
        v = self.verifier.verify(result, decision)
        assert v.ok is False
        assert "too large" in v.reason.lower()

    def test_verify_warns_no_commands(self):
        result = self.make_result(commands_to_run=[])
        decision = self.make_decision()
        v = self.verifier.verify(result, decision)
        assert v.ok is True
        assert any("verification commands" in w for w in v.warnings)
        assert v.executed_commands == []

    def test_verify_runs_commands_in_workspace(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            result = self.make_result(commands_to_run=[
                f'"{sys.executable}" -c "import pathlib; print(pathlib.Path.cwd().name)"'
            ])
            decision = self.make_decision()
            v = self.verifier.verify(result, decision, workspace=tmpdir)
            assert v.ok is True
            assert v.executed_commands[0]["exit_code"] == 0
            assert os.path.basename(tmpdir) in v.executed_commands[0]["stdout"]

    def test_verify_fails_when_command_fails(self):
        result = self.make_result(commands_to_run=[
            f'"{sys.executable}" -c "raise SystemExit(3)"'
        ])
        decision = self.make_decision()
        v = self.verifier.verify(result, decision)
        assert v.ok is False
        assert v.fallback_to_codex is True
        assert "Verification command failed" in v.reason
        assert v.executed_commands[0]["exit_code"] == 3

    def test_verify_rejects_invalid_command_entry(self):
        result = self.make_result(commands_to_run=[""])
        decision = self.make_decision()
        v = self.verifier.verify(result, decision)
        assert v.ok is False
        assert "commands_to_run" in v.reason

    def test_verify_all_valid_statuses(self):
        for status in VALID_STATUS:
            commands = [] if status != "success" else [f'"{sys.executable}" -c "print(1)"']
            result = self.make_result(status=status, commands_to_run=commands)
            decision = self.make_decision()
            v = self.verifier.verify(result, decision)
            assert v is not None

    def test_required_keys_complete(self):
        assert REQUIRED_KEYS == {"status", "summary", "changed_files", "patch", "commands_to_run", "risk_notes"}
