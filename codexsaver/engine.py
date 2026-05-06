from __future__ import annotations

from pathlib import Path
from typing import Any, Dict

from .context import ContextPacker
from .cost import CostEstimator
from .deepseek_client import DeepSeekClient, DeepSeekError
from .router import Router
from .schema import DelegateTaskInput, WorkerTask, to_dict
from .verifier import Verifier


DEFAULT_CONSTRAINTS = [
    "Return JSON only.",
    "Prefer minimal, reviewable changes.",
    "Do not claim tests passed unless test output is provided.",
    "If uncertain or risky, return status=needs_codex.",
]


class CodexSaverEngine:
    def __init__(self):
        self.router = Router()
        self.verifier = Verifier()
        self.cost = CostEstimator()

    def delegate_task(self, input_data: Dict[str, Any]) -> Dict[str, Any]:
        req = DelegateTaskInput(
            instruction=input_data["instruction"],
            files=input_data.get("files", []),
            constraints=input_data.get("constraints", []),
            max_files=int(input_data.get("max_files", 8)),
            max_chars_per_file=int(input_data.get("max_chars_per_file", 24_000)),
            max_total_chars=int(input_data.get("max_total_chars", 120_000)),
            dry_run=bool(input_data.get("dry_run", False)),
        )

        decision = self.router.decide(req.instruction, req.files)

        packer = ContextPacker(
            max_files=req.max_files,
            max_chars_per_file=req.max_chars_per_file,
            max_total_chars=req.max_total_chars,
        )

        task = WorkerTask(
            instruction=req.instruction,
            task_type=decision.task_type,
            risk=decision.risk,
            constraints=(req.constraints or []) + DEFAULT_CONSTRAINTS,
            workspace=str(Path.cwd()),
            files=packer.load(req.files),
        )

        if decision.route == "codex":
            return {
                "route": "codex", "status": "needs_codex",
                "decision": to_dict(decision), "estimated_savings_percent": 0,
                "message": "CodexSaver recommends Codex handle this task directly.",
            }

        estimated_savings = self.cost.estimate_savings_percent(task, delegated=True)

        if req.dry_run:
            return {
                "route": "deepseek", "status": "dry_run",
                "decision": to_dict(decision),
                "estimated_savings_percent": estimated_savings,
                "task_preview": to_dict(task),
            }

        try:
            worker_result = DeepSeekClient().complete_task(task)
        except DeepSeekError as e:
            return {
                "route": "codex", "status": "failed",
                "decision": to_dict(decision), "estimated_savings_percent": 0,
                "message": f"DeepSeek failed; Codex should take over. Error: {e}",
            }

        verification = self.verifier.verify(worker_result, decision)

        return {
            "route": "deepseek" if not verification.fallback_to_codex else "codex",
            "status": "success" if verification.ok else "needs_codex",
            "decision": to_dict(decision),
            "estimated_savings_percent": estimated_savings if verification.ok else 0,
            "verification": to_dict(verification),
            "result": worker_result,
            "codex_instruction": (
                "Review the patch carefully. Apply only if safe. "
                "Run or ask the user to run commands_to_run before finalizing."
            ),
        }
