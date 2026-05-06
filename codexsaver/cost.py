from __future__ import annotations

from .schema import WorkerTask


class CostEstimator:
    def estimate_savings_percent(self, task: WorkerTask, delegated: bool) -> int:
        if not delegated:
            return 0
        chars = len(task.instruction) + sum(len(f.content) for f in task.files)
        if chars < 8_000:
            return 45
        if chars < 50_000:
            return 62
        return 70
