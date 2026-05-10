from __future__ import annotations

import pytest
from subdispatch.cost import CostEstimator
from subdispatch.schema import (
    FileContext, RouteDecision, WorkerTask, VerificationResult, to_dict
)


class TestCostEstimator:
    def setup_method(self):
        self.estimator = CostEstimator()

    def test_estimate_savings_not_delegated(self):
        task = WorkerTask(
            instruction="hello",
            task_type="write_tests",
            risk="low",
            constraints=[],
            workspace=".",
            files=[],
        )
        assert self.estimator.estimate_savings_percent(task, delegated=False) == 0

    def test_estimate_savings_small_context(self):
        task = WorkerTask(
            instruction="hi",
            task_type="write_tests",
            risk="low",
            constraints=[],
            workspace=".",
            files=[FileContext(path="f.py", content="x" * 100)],
        )
        assert self.estimator.estimate_savings_percent(task, delegated=True) == 45

    def test_estimate_savings_medium_context(self):
        task = WorkerTask(
            instruction="hi",
            task_type="write_tests",
            risk="low",
            constraints=[],
            workspace=".",
            files=[FileContext(path="f.py", content="x" * 10_000)],
        )
        assert self.estimator.estimate_savings_percent(task, delegated=True) == 62

    def test_estimate_savings_large_context(self):
        task = WorkerTask(
            instruction="hi",
            task_type="write_tests",
            risk="low",
            constraints=[],
            workspace=".",
            files=[FileContext(path="f.py", content="x" * 60_000)],
        )
        assert self.estimator.estimate_savings_percent(task, delegated=True) == 70


class TestSchema:
    def test_file_context_to_dict(self):
        fc = FileContext(path="foo.py", content="pass")
        d = to_dict(fc)
        assert d == {"path": "foo.py", "content": "pass"}

    def test_route_decision_to_dict(self):
        rd = RouteDecision(
            route="deepseek",
            task_type="write_tests",
            risk="low",
            reason="test",
            protected_hits=[],
        )
        d = to_dict(rd)
        assert d["route"] == "deepseek"
        assert d["task_type"] == "write_tests"
        assert d["risk"] == "low"

    def test_verification_result_to_dict(self):
        vr = VerificationResult(
            ok=True,
            fallback_to_codex=False,
            reason="ok",
            warnings=["watch out"],
            executed_commands=[],
        )
        d = to_dict(vr)
        assert d["ok"] is True
        assert "watch out" in d["warnings"]
