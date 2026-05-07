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
            workspace=input_data.get("workspace", "."),
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
            workspace=req.workspace,
        )

        task = WorkerTask(
            instruction=req.instruction,
            task_type=decision.task_type,
            risk=decision.risk,
            constraints=(req.constraints or []) + DEFAULT_CONSTRAINTS,
            workspace=str(Path(req.workspace).resolve()),
            files=packer.load(req.files),
        )

        if decision.route == "codex":
            return {
                "route": "codex", "status": "needs_codex",
                "decision": to_dict(decision), "estimated_savings_percent": 0,
                "message": "CodexSaver recommends Codex handle this task directly.",
                "interaction": self._interaction_payload(
                    decision=to_dict(decision),
                    route="codex",
                    status="needs_codex",
                    estimated_savings_percent=0,
                    mode="codex_takeover",
                    detail="Protected domain or ambiguous task detected before delegation.",
                ),
            }

        estimated_savings = self.cost.estimate_savings_percent(task, delegated=True)

        if req.dry_run:
            return {
                "route": "deepseek", "status": "dry_run",
                "decision": to_dict(decision),
                "estimated_savings_percent": estimated_savings,
                "task_preview": to_dict(task),
                "interaction": self._interaction_payload(
                    decision=to_dict(decision),
                    route="deepseek",
                    status="dry_run",
                    estimated_savings_percent=estimated_savings,
                    mode="preview",
                    detail="Dry-run preview only. No external model call was made.",
                ),
            }

        try:
            worker_result = DeepSeekClient().complete_task(task)
        except DeepSeekError as e:
            return {
                "route": "codex", "status": "failed",
                "decision": to_dict(decision), "estimated_savings_percent": 0,
                "message": f"DeepSeek failed; Codex should take over. Error: {e}",
                "interaction": self._interaction_payload(
                    decision=to_dict(decision),
                    route="codex",
                    status="failed",
                    estimated_savings_percent=0,
                    mode="codex_takeover",
                    detail=f"Delegation failed and control returned to Codex: {e}",
                ),
            }

        verification = self.verifier.verify(worker_result, decision, workspace=task.workspace)

        final_route = "deepseek" if not verification.fallback_to_codex else "codex"
        final_status = "success" if verification.ok else "needs_codex"
        final_savings = estimated_savings if verification.ok else 0

        return {
            "route": final_route,
            "status": final_status,
            "decision": to_dict(decision),
            "estimated_savings_percent": final_savings,
            "verification": to_dict(verification),
            "result": worker_result,
            "codex_instruction": (
                "Review the patch carefully. Apply only if safe. "
                "Run or ask the user to run commands_to_run before finalizing."
            ),
            "interaction": self._interaction_payload(
                decision=to_dict(decision),
                route=final_route,
                status=final_status,
                estimated_savings_percent=final_savings,
                mode="delegated_execution" if verification.ok else "codex_takeover",
                detail=verification.reason,
            ),
        }

    def _interaction_payload(self, decision: Dict[str, Any], route: str, status: str,
                             estimated_savings_percent: int, mode: str,
                             detail: str) -> Dict[str, Any]:
        task_type = decision["task_type"]
        risk = decision["risk"]
        tool_name = "codexsaver.delegate_task"
        if route == "deepseek" and status == "success":
            headline = "CodexSaver delegated this task to DeepSeek."
            next_step = "Review the worker result and apply it only if the patch looks safe."
        elif route == "deepseek" and status == "dry_run":
            headline = "CodexSaver previewed a delegated run."
            next_step = "Call the tool without dry_run to execute the delegated task."
        elif status == "failed":
            headline = "CodexSaver attempted delegation but returned control to Codex."
            next_step = "Handle the task in Codex or retry after fixing the worker/API issue."
        else:
            headline = "CodexSaver kept this task in Codex."
            next_step = "Use Codex directly because the task is risky, protected, or ambiguous."
        return {
            "tool": tool_name,
            "mode": mode,
            "headline": headline,
            "route_label": f"[CodexSaver] route={route} task_type={task_type} risk={risk}",
            "reason": decision["reason"],
            "detail": detail,
            "estimated_savings_percent": estimated_savings_percent,
            "next_step": next_step,
        }
