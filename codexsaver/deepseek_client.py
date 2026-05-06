from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
from typing import Any, Dict

from .schema import WorkerTask, to_dict


class DeepSeekError(RuntimeError):
    pass


SYSTEM_PROMPT = """
You are CodexSaver's low-cost coding worker.

You are NOT the final authority. Codex will review your output.

Rules:
- Return valid JSON only. No markdown fences.
- Prefer small, reviewable patches.
- Do not claim tests passed unless test output is provided.
- If the task is risky, ambiguous, or requires architecture judgment, return status="needs_codex".
- Do not modify production logic when the instruction only asks for tests/docs.
- Use unified diff format in the patch field when proposing code changes.

Required JSON shape:
{
  "status": "success | failed | needs_codex",
  "summary": "short summary",
  "changed_files": ["path"],
  "patch": "unified diff or empty string",
  "commands_to_run": ["command"],
  "risk_notes": ["note"]
}
""".strip()


class DeepSeekClient:
    """Tiny OpenAI-compatible DeepSeek client using only stdlib."""

    def __init__(self, api_key: str | None = None, model: str | None = None,
                 base_url: str | None = None, timeout_seconds: int = 120):
        self.api_key = api_key or os.environ.get("DEEPSEEK_API_KEY")
        self.model = model or os.environ.get("DEEPSEEK_MODEL", "deepseek-chat")
        self.base_url = base_url or os.environ.get(
            "DEEPSEEK_BASE_URL", "https://api.deepseek.com/chat/completions")
        self.timeout_seconds = timeout_seconds
        if not self.api_key:
            raise DeepSeekError("Missing DEEPSEEK_API_KEY.")

    def complete_task(self, task: WorkerTask) -> Dict[str, Any]:
        payload = {
            "model": self.model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": json.dumps(to_dict(task), ensure_ascii=False)},
            ],
            "temperature": 0.2,
            "response_format": {"type": "json_object"},
        }
        request = urllib.request.Request(
            self.base_url,
            data=json.dumps(payload).encode("utf-8"),
            headers={
                "Authorization": f"Bearer {self.api_key}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout_seconds) as response:
                body = response.read().decode("utf-8")
        except urllib.error.HTTPError as e:
            detail = e.read().decode("utf-8", errors="replace")
            raise DeepSeekError(f"DeepSeek HTTP {e.code}: {detail}") from e
        except urllib.error.URLError as e:
            raise DeepSeekError(f"DeepSeek connection failed: {e}") from e
        try:
            data = json.loads(body)
            content = data["choices"][0]["message"]["content"]
            return json.loads(content)
        except Exception as e:
            raise DeepSeekError(f"Failed to parse DeepSeek response: {body[:1000]}") from e
