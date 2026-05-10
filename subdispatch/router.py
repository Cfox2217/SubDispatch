from __future__ import annotations

import re
from typing import List

from .schema import RouteDecision, RiskLevel, TaskType

PROTECTED_PATH_KEYWORDS = [
    "auth", "oauth", "jwt", "session", "security", "permission", "rbac",
    "payment", "payments", "billing", "invoice", "migration", "migrations",
    "schema", "infra", "terraform", ".github/workflows", ".env", "secret",
    "key", "token",
]

HIGH_RISK_INSTRUCTION_KEYWORDS = [
    "authentication", "authorization", "permission", "security", "payment",
    "billing", "migration", "database schema", "encrypt", "decrypt", "secret",
    "token", "production", "deploy",
]

DELEGATABLE: set[TaskType] = {
    "code_search", "explain", "write_tests", "fix_lint", "docs", "boilerplate",
    "simple_refactor",
}


class Router:
    def classify(self, instruction: str) -> TaskType:
        text = instruction.lower()
        if self._has(text, ["find", "search", "locate", "where is", "scan", "grep"]):
            return "code_search"
        if self._has(text, ["explain", "summarize", "what does", "walk me through"]):
            return "explain"
        if self._has(text, ["test", "unit test", "pytest", "jest", "spec", "coverage"]):
            return "write_tests"
        if self._has(text, ["lint", "eslint", "prettier", "mypy", "ruff", "type error", "tsc"]):
            return "fix_lint"
        if self._has(text, ["readme", "docs", "documentation", "comment", "docstring"]):
            return "docs"
        if self._has(text, ["boilerplate", "scaffold", "template", "generate"]):
            return "boilerplate"
        if self._has(text, ["refactor", "rename", "cleanup", "simplify", "deduplicate"]):
            return "simple_refactor"
        if self._has(text, ["review", "check this patch", "audit"]):
            return "review_draft"
        return "unknown"

    def risk(self, instruction: str, files: List[str]) -> tuple[RiskLevel, List[str]]:
        text = instruction.lower()
        paths = "\n".join(files).lower()
        protected_hits = sorted({k for k in PROTECTED_PATH_KEYWORDS if k in paths or k in text})
        high_instruction_hits = sorted({k for k in HIGH_RISK_INSTRUCTION_KEYWORDS if k in text})
        if high_instruction_hits:
            return "high", high_instruction_hits
        if protected_hits:
            if self.classify(instruction) in {"write_tests", "docs", "explain", "code_search"}:
                return "medium", protected_hits
            return "high", protected_hits
        if len(files) > 5:
            return "medium", []
        return "low", []

    def decide(self, instruction: str, files: List[str]) -> RouteDecision:
        task_type = self.classify(instruction)
        risk, protected_hits = self.risk(instruction, files)
        if risk == "high":
            return RouteDecision(route="codex", task_type=task_type, risk=risk,
                reason="High-risk task or protected domain detected.", protected_hits=protected_hits)
        if task_type == "unknown":
            return RouteDecision(route="codex", task_type=task_type, risk=risk,
                reason="Ambiguous task type.", protected_hits=protected_hits)
        if task_type not in DELEGATABLE:
            return RouteDecision(route="codex", task_type=task_type, risk=risk,
                reason=f"Task type '{task_type}' is not delegatable.", protected_hits=protected_hits)
        if risk == "medium" and task_type not in {"write_tests", "docs", "explain", "code_search"}:
            return RouteDecision(route="codex", task_type=task_type, risk=risk,
                reason="Medium-risk code modification.", protected_hits=protected_hits)
        return RouteDecision(route="deepseek", task_type=task_type, risk=risk,
            reason="Task is low/acceptable risk.", protected_hits=protected_hits)

    @staticmethod
    def _has(text: str, words: List[str]) -> bool:
        return any(w in text for w in words)
