from __future__ import annotations

from typing import Any, Dict, List

from .router import PROTECTED_PATH_KEYWORDS
from .schema import RouteDecision, VerificationResult


REQUIRED_KEYS = {"status", "summary", "changed_files", "patch", "commands_to_run", "risk_notes"}
VALID_STATUS = {"success", "failed", "needs_codex"}


class Verifier:
    def verify(self, worker_result: Dict[str, Any], decision: RouteDecision) -> VerificationResult:
        warnings: List[str] = []
        missing = REQUIRED_KEYS - set(worker_result.keys())
        if missing:
            return VerificationResult(False, True,
                f"Worker result missing required keys: {sorted(missing)}", warnings)
        status = worker_result.get("status")
        if status not in VALID_STATUS:
            return VerificationResult(False, True,
                f"Invalid worker status: {status}", warnings)
        if status != "success":
            return VerificationResult(False, True,
                f"Worker requested fallback with status={status}.", warnings)
        changed_files = worker_result.get("changed_files") or []
        if not isinstance(changed_files, list):
            return VerificationResult(False, True,
                "changed_files must be a list.", warnings)
        risky = [path for path in changed_files
                 if any(keyword in str(path).lower() for keyword in PROTECTED_PATH_KEYWORDS)]
        if risky and decision.risk != "low":
            return VerificationResult(False, True,
                f"Worker touched protected files under non-low risk: {risky}", warnings)
        patch = worker_result.get("patch", "")
        if patch and len(patch) > 120_000:
            return VerificationResult(False, True,
                "Patch is too large for safe automatic delegation.", warnings)
        if not worker_result.get("commands_to_run"):
            warnings.append("No verification commands suggested by worker.")
        return VerificationResult(True, False,
            "Worker result passed structural verification.", warnings)
