from __future__ import annotations

import json
import os
import shlex
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from string import Template
from typing import Any, Dict, List


TASK_STATUS_QUEUED = "queued"
TASK_STATUS_RUNNING = "running"
TASK_STATUS_COMPLETED = "completed"
TASK_STATUS_FAILED = "failed"
TASK_STATUS_CANCELLED = "cancelled"
TASK_STATUS_MISSING = "missing"


SUPERVISOR_SCRIPT = r"""
import json
import subprocess
import sys
import time
from pathlib import Path

launch_path = Path(sys.argv[1])
spec = json.loads(launch_path.read_text(encoding="utf-8"))
exit_path = Path(spec["exit_path"])
exit_path.parent.mkdir(parents=True, exist_ok=True)
started_at = time.time()
try:
    with open(spec["stdout_path"], "ab") as stdout, open(spec["stderr_path"], "ab") as stderr:
        completed = subprocess.run(
            spec["command"],
            cwd=spec["cwd"],
            env=spec["env"],
            stdout=stdout,
            stderr=stderr,
        )
    exit_code = completed.returncode
    error = None
except Exception as exc:
    exit_code = 127
    error = str(exc)
exit_path.write_text(json.dumps({
    "exit_code": exit_code,
    "error": error,
    "started_at": started_at,
    "finished_at": time.time(),
}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
"""


HOOK_RECORDER_SCRIPT = r"""
import json
import os
import sys
import time
from pathlib import Path

event_path = Path(os.environ["SUBDISPATCH_HOOK_EVENTS_PATH"])
summary_path = Path(os.environ["SUBDISPATCH_HOOK_SUMMARY_PATH"])
run_id = os.environ.get("SUBDISPATCH_RUN_ID")
task_id = os.environ.get("SUBDISPATCH_TASK_ID")
event_path.parent.mkdir(parents=True, exist_ok=True)
summary_path.parent.mkdir(parents=True, exist_ok=True)

try:
    raw_input = sys.stdin.read()
    payload = json.loads(raw_input) if raw_input.strip() else {}
except Exception as exc:
    payload = {"hook_event_name": "HookParseError", "error": str(exc)}

recorded_at = time.time()
event = {
    "recorded_at": recorded_at,
    "run_id": run_id,
    "task_id": task_id,
    "hook_event_name": payload.get("hook_event_name"),
    "session_id": payload.get("session_id"),
    "transcript_path": payload.get("transcript_path"),
    "agent_transcript_path": payload.get("agent_transcript_path"),
    "tool_name": payload.get("tool_name"),
    "cwd": payload.get("cwd"),
    "file_path": payload.get("file_path"),
    "reason": payload.get("reason"),
    "notification_type": payload.get("notification_type"),
    "last_assistant_message": payload.get("last_assistant_message"),
    "raw": payload,
}
with event_path.open("a", encoding="utf-8") as handle:
    handle.write(json.dumps(event, ensure_ascii=False) + "\n")

previous = {}
if summary_path.exists():
    try:
        previous = json.loads(summary_path.read_text(encoding="utf-8"))
    except Exception:
        previous = {}

last_message = payload.get("last_assistant_message")
summary = {
    "run_id": run_id,
    "task_id": task_id,
    "event_count": int(previous.get("event_count", 0)) + 1,
    "last_event_at": recorded_at,
    "last_event_name": payload.get("hook_event_name"),
    "last_session_id": payload.get("session_id") or previous.get("last_session_id"),
    "transcript_path": payload.get("transcript_path") or previous.get("transcript_path"),
    "agent_transcript_path": payload.get("agent_transcript_path") or previous.get("agent_transcript_path"),
    "last_tool_name": payload.get("tool_name") or previous.get("last_tool_name"),
    "last_cwd": payload.get("cwd") or previous.get("last_cwd"),
    "last_reason": payload.get("reason") or previous.get("last_reason"),
    "last_assistant_message_tail": (
        last_message[-2000:] if isinstance(last_message, str)
        else previous.get("last_assistant_message_tail")
    ),
}
summary_path.write_text(
    json.dumps(summary, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
"""


@dataclass(frozen=True)
class WorkerConfig:
    id: str
    command: List[str]
    max_concurrency: int = 1
    model: str | None = None
    enabled: bool = True
    timeout_seconds: int | None = None
    env: Dict[str, str] | None = None
    worker_mode: str = "trusted-worktree"
    permission_mode: str = "bypassPermissions"
    description: str = ""
    strengths: List[str] | None = None
    cost: str = "unknown"
    speed: str = "unknown"


class SubDispatchEngine:
    def __init__(self, workspace: str = ".", workers: Dict[str, WorkerConfig] | None = None):
        self.workspace = Path(workspace).resolve()
        self.root = self.workspace / ".subdispatch"
        self.runs_dir = self.root / "runs"
        self.worktrees_dir = self.root / "worktrees"
        self.env = load_env(self.workspace)
        self.workers = workers or default_workers(self.workspace, self.env)

    def list_workers(self) -> Dict[str, Any]:
        workers = []
        running_counts = self._running_counts_by_worker()
        queued_counts = self._queued_counts_by_worker()
        for worker in self.workers.values():
            running = running_counts.get(worker.id, 0)
            queued = queued_counts.get(worker.id, 0)
            available_slots = max(worker.max_concurrency - running, 0)
            executable = self._command_available(worker.command)
            enabled = worker.enabled and executable
            unavailable_reason = None
            if not worker.enabled:
                unavailable_reason = "worker disabled"
            elif not executable:
                unavailable_reason = f"command not found: {worker.command[0]}"
            elif available_slots <= 0:
                unavailable_reason = "concurrency limit reached"
            workers.append({
                "id": worker.id,
                "runner": worker.command[0] if worker.command else "",
                "command": worker.command,
                "model": worker.model,
                "worker_mode": worker.worker_mode,
                "permission_mode": worker.permission_mode,
                "sandbox": "none",
                "risk": "high" if worker.permission_mode == "bypassPermissions" else "medium",
                "description": worker.description,
                "strengths": worker.strengths or [],
                "cost": worker.cost,
                "speed": worker.speed,
                "enabled": enabled,
                "max_concurrency": worker.max_concurrency,
                "running": running,
                "queued": queued,
                "available_slots": available_slots if enabled else 0,
                "unavailable_reason": unavailable_reason,
            })
        return {"status": "ok", "workers": workers}

    def start_run(self, input_data: Dict[str, Any]) -> Dict[str, Any]:
        self._ensure_git_repo()
        goal = input_data["goal"]
        base_ref = input_data.get("base") or input_data.get("base_branch") or "HEAD"
        base_commit = self._git(["rev-parse", base_ref]).strip()
        run_id = input_data.get("run_id") or self._new_run_id()
        tasks = input_data.get("tasks") or []
        if not tasks:
            raise ValueError("start_run requires at least one task")

        run_dir = self._run_dir(run_id)
        run_dir.mkdir(parents=True, exist_ok=False)
        (run_dir / "tasks").mkdir(parents=True, exist_ok=True)
        (self.worktrees_dir / run_id).mkdir(parents=True, exist_ok=True)

        run = {
            "id": run_id,
            "goal": goal,
            "base_ref": base_ref,
            "base_commit": base_commit,
            "workspace": str(self.workspace),
            "created_at": time.time(),
            "tasks": [],
        }
        self._write_json(run_dir / "run.json", run)

        task_reports = []
        for raw_task in tasks:
            task_id = raw_task["id"]
            worker_id = raw_task.get("worker") or "claude-code"
            if worker_id not in self.workers:
                raise ValueError(f"Unknown worker: {worker_id}")
            branch = raw_task.get("branch") or f"agent/{run_id}/{task_id}"
            worktree = self.worktrees_dir / run_id / task_id
            task_dir = self._task_dir(run_id, task_id)
            task_dir.mkdir(parents=True, exist_ok=True)
            self._git(["branch", branch, base_commit])
            self._git(["worktree", "add", str(worktree), branch])
            task = {
                "id": task_id,
                "run_id": run_id,
                "goal": goal,
                "instruction": raw_task["instruction"],
                "worker": worker_id,
                "status": TASK_STATUS_QUEUED,
                "branch": branch,
                "worktree": str(worktree),
                "base_commit": base_commit,
                "read_scope": raw_task.get("read_scope", []),
                "write_scope": raw_task.get("write_scope", []),
                "forbidden_paths": raw_task.get("forbidden_paths", []),
                "context": raw_task.get("context", ""),
                "context_files": raw_task.get("context_files", []),
                "created_at": time.time(),
                "pid": None,
                "exit_code": None,
                "error": None,
            }
            self._write_json(task_dir / "task.json", task)
            run["tasks"].append(task_id)
            task_reports.append({
                "id": task_id,
                "status": task["status"],
                "worker": worker_id,
                "branch": branch,
                "worktree": str(worktree),
            })
        self._write_json(run_dir / "run.json", run)
        self._schedule_queued_tasks(run_id)
        return {
            "status": "ok",
            "run_id": run_id,
            "base_commit": base_commit,
            "tasks": self._task_summaries(run_id),
        }

    def poll_run(self, input_data: Dict[str, Any]) -> Dict[str, Any]:
        run_id = input_data["run_id"]
        self._refresh_run(run_id)
        self._schedule_queued_tasks(run_id)
        self._refresh_run(run_id)
        tasks = self._task_summaries(run_id)
        run_status = "completed" if tasks and all(
            t["status"] in {TASK_STATUS_COMPLETED, TASK_STATUS_FAILED, TASK_STATUS_CANCELLED, TASK_STATUS_MISSING}
            for t in tasks
        ) else "running"
        return {"status": run_status, "run_id": run_id, "tasks": tasks}

    def collect_task(self, input_data: Dict[str, Any]) -> Dict[str, Any]:
        run_id = input_data["run_id"]
        task_id = input_data["task_id"]
        self._refresh_task(run_id, task_id)
        task = self._read_task(run_id, task_id)
        task_dir = self._task_dir(run_id, task_id)
        worktree = Path(task["worktree"])
        changed_files = self._changed_files(task["base_commit"], task["branch"], worktree)
        diff = self._task_diff(task["base_commit"], task["branch"], worktree)
        patch_path = task_dir / "diff.patch"
        patch_path.write_text(diff, encoding="utf-8")
        manifest_path = worktree / ".subdispatch" / "result.json"
        manifest = self._read_json(manifest_path) if manifest_path.exists() else None
        artifact = {
            "run_id": run_id,
            "task_id": task_id,
            "status": task["status"],
            "instruction": task["instruction"],
            "worker": task["worker"],
            "base_commit": task["base_commit"],
            "branch": task["branch"],
            "worktree": task["worktree"],
            "changed_files": changed_files,
            "diff": diff,
            "patch_path": str(patch_path),
            "manifest": manifest,
            "stdout_tail": self._tail(task_dir / "stdout.log"),
            "stderr_tail": self._tail(task_dir / "stderr.log"),
            "hook_summary": self._hook_summary(task),
            "hook_events_tail": self._hook_events_tail(task),
            "transcript_tail": self._transcript_tail(task),
            "scope_check": self._scope_check(changed_files, task.get("write_scope", [])),
            "forbidden_path_check": self._forbidden_path_check(
                changed_files, task.get("forbidden_paths", [])
            ),
        }
        self._write_json(task_dir / "artifact.json", artifact)
        return artifact

    def delete_worktree(self, input_data: Dict[str, Any]) -> Dict[str, Any]:
        run_id = input_data["run_id"]
        task_id = input_data["task_id"]
        force = bool(input_data.get("force", False))
        delete_branch = bool(input_data.get("delete_branch", False))
        self._refresh_task(run_id, task_id)
        task = self._read_task(run_id, task_id)
        if task["status"] == TASK_STATUS_RUNNING and not force:
            raise ValueError("Refusing to delete running task worktree without force=true")
        worktree = Path(task["worktree"]).resolve()
        managed_root = (self.worktrees_dir / run_id).resolve()
        if not worktree.is_relative_to(managed_root):
            raise ValueError(f"Refusing to delete unmanaged worktree: {worktree}")
        removed = False
        if worktree.exists():
            self._git(["worktree", "remove", "--force", str(worktree)])
            removed = True
        branch_deleted = False
        if delete_branch:
            self._git(["branch", "-D", task["branch"]])
            branch_deleted = True
        task["worktree_deleted_at"] = time.time()
        task["worktree_removed"] = removed
        self._write_task(run_id, task_id, task)
        return {
            "status": "ok",
            "run_id": run_id,
            "task_id": task_id,
            "worktree_removed": removed,
            "branch_deleted": branch_deleted,
            "artifact_dir": str(self._task_dir(run_id, task_id)),
        }

    def _schedule_queued_tasks(self, run_id: str) -> None:
        self._refresh_run(run_id)
        running_counts = self._running_counts_by_worker()
        for task_id in self._read_run(run_id)["tasks"]:
            task = self._read_task(run_id, task_id)
            if task["status"] != TASK_STATUS_QUEUED:
                continue
            worker = self.workers[task["worker"]]
            if not worker.enabled or not self._command_available(worker.command):
                task["error"] = f"Worker unavailable: {worker.id}"
                self._write_task(run_id, task_id, task)
                continue
            if running_counts.get(worker.id, 0) >= worker.max_concurrency:
                continue
            self._start_task_process(task, worker)
            running_counts[worker.id] = running_counts.get(worker.id, 0) + 1

    def _start_task_process(self, task: Dict[str, Any], worker: WorkerConfig) -> None:
        task_dir = self._task_dir(task["run_id"], task["id"])
        prompt_path = task_dir / "prompt.txt"
        result_path = Path(task["worktree"]) / ".subdispatch" / "result.json"
        launch_path = task_dir / "launch.json"
        exit_path = task_dir / "exit.json"
        hook_events_path = task_dir / "hook_events.jsonl"
        hook_summary_path = task_dir / "hook_summary.json"
        result_path.parent.mkdir(parents=True, exist_ok=True)
        self._install_claude_hooks(
            worktree=Path(task["worktree"]),
            task_dir=task_dir,
            hook_events_path=hook_events_path,
            hook_summary_path=hook_summary_path,
        )
        prompt = self._render_prompt(task, result_path)
        prompt_path.write_text(prompt, encoding="utf-8")
        command = [
            Template(part).safe_substitute(
                prompt=prompt,
                prompt_path=str(prompt_path),
                result_path=str(result_path),
                model=worker.model or "",
                permission_mode=worker.permission_mode,
                worker_mode=worker.worker_mode,
                task_id=task["id"],
                run_id=task["run_id"],
                worktree=task["worktree"],
            )
            for part in worker.command
        ]
        env = os.environ.copy()
        env.update(worker.env or {})
        env.update({
            "SUBDISPATCH_RUN_ID": task["run_id"],
            "SUBDISPATCH_TASK_ID": task["id"],
            "SUBDISPATCH_RESULT_PATH": str(result_path),
            "SUBDISPATCH_PROMPT_PATH": str(prompt_path),
            "SUBDISPATCH_WORKER_MODE": worker.worker_mode,
            "SUBDISPATCH_PERMISSION_MODE": worker.permission_mode,
            "SUBDISPATCH_HOOK_EVENTS_PATH": str(hook_events_path),
            "SUBDISPATCH_HOOK_SUMMARY_PATH": str(hook_summary_path),
        })
        launch_spec = {
            "command": command,
            "cwd": task["worktree"],
            "env": env,
            "stdout_path": str(task_dir / "stdout.log"),
            "stderr_path": str(task_dir / "stderr.log"),
            "exit_path": str(exit_path),
        }
        self._write_json(launch_path, launch_spec)
        launch_path.chmod(0o600)
        process = subprocess.Popen(
            [sys.executable, "-c", SUPERVISOR_SCRIPT, str(launch_path)],
            cwd=self.workspace,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        task["status"] = TASK_STATUS_RUNNING
        task["pid"] = process.pid
        task["started_at"] = time.time()
        task["command"] = command
        task["exit_path"] = str(exit_path)
        task["hook_events_path"] = str(hook_events_path)
        task["hook_summary_path"] = str(hook_summary_path)
        self._write_task(task["run_id"], task["id"], task)

    def _install_claude_hooks(
        self,
        worktree: Path,
        task_dir: Path,
        hook_events_path: Path,
        hook_summary_path: Path,
    ) -> None:
        claude_dir = worktree / ".claude"
        hooks_dir = claude_dir / "hooks"
        hooks_dir.mkdir(parents=True, exist_ok=True)
        recorder_path = hooks_dir / "subdispatch_hook_recorder.py"
        recorder_path.write_text(HOOK_RECORDER_SCRIPT, encoding="utf-8")
        recorder_path.chmod(0o700)
        command = " ".join([
            shlex.quote(sys.executable),
            shlex.quote(str(recorder_path)),
        ])
        settings = {
            "hooks": {
                event_name: [
                    {
                        "matcher": "",
                        "hooks": [
                            {
                                "type": "command",
                                "command": command,
                            }
                        ],
                    }
                ]
                for event_name in (
                    "SessionStart",
                    "UserPromptSubmit",
                    "PreToolUse",
                    "PostToolUse",
                    "Notification",
                    "Stop",
                    "SubagentStop",
                )
            }
        }
        settings_path = claude_dir / "settings.local.json"
        settings_path.write_text(
            json.dumps(settings, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        hook_summary_path.write_text(json.dumps({
            "event_count": 0,
            "hook_events_path": str(hook_events_path),
            "hook_summary_path": str(hook_summary_path),
            "settings_path": str(settings_path),
            "recorder_path": str(recorder_path),
        }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    def _refresh_run(self, run_id: str) -> None:
        run = self._read_run(run_id)
        for task_id in run["tasks"]:
            self._refresh_task(run_id, task_id)

    def _refresh_task(self, run_id: str, task_id: str) -> None:
        task_path = self._task_dir(run_id, task_id) / "task.json"
        if not task_path.exists():
            return
        task = self._read_task(run_id, task_id)
        if task["status"] != TASK_STATUS_RUNNING:
            return
        pid = task.get("pid")
        if not pid:
            task["status"] = TASK_STATUS_MISSING
            task["error"] = "running task has no pid"
            self._write_task(run_id, task_id, task)
            return
        exit_code = self._recorded_exit_code(task)
        if exit_code is None and self._process_is_running(int(pid)):
            return
        if exit_code is None:
            exit_code = 0
            task["warning"] = "process disappeared before SubDispatch recorded an exit code"
        if exit_code is None:
            return
        task["exit_code"] = exit_code
        task["finished_at"] = time.time()
        task["status"] = TASK_STATUS_COMPLETED if exit_code == 0 else TASK_STATUS_FAILED
        self._write_task(run_id, task_id, task)

    def _recorded_exit_code(self, task: Dict[str, Any]) -> int | None:
        exit_path = Path(task.get("exit_path") or self._task_dir(task["run_id"], task["id"]) / "exit.json")
        if not exit_path.exists():
            return None
        try:
            data = self._read_json(exit_path)
        except json.JSONDecodeError:
            return None
        return int(data["exit_code"])

    def _process_is_running(self, pid: int) -> bool:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        except PermissionError:
            return True
        return True

    def _running_counts_by_worker(self) -> Dict[str, int]:
        counts: Dict[str, int] = {}
        for task in self._all_tasks():
            if task.get("status") == TASK_STATUS_RUNNING and task.get("worker"):
                counts[task["worker"]] = counts.get(task["worker"], 0) + 1
        return counts

    def _queued_counts_by_worker(self) -> Dict[str, int]:
        counts: Dict[str, int] = {}
        for task in self._all_tasks():
            if task.get("status") == TASK_STATUS_QUEUED and task.get("worker"):
                counts[task["worker"]] = counts.get(task["worker"], 0) + 1
        return counts

    def _all_tasks(self) -> List[Dict[str, Any]]:
        if not self.runs_dir.exists():
            return []
        tasks = []
        for task_path in self.runs_dir.glob("*/tasks/*/task.json"):
            try:
                tasks.append(self._read_json(task_path))
            except json.JSONDecodeError:
                continue
        return tasks

    def _task_summaries(self, run_id: str) -> List[Dict[str, Any]]:
        run = self._read_run(run_id)
        summaries = []
        for task_id in run["tasks"]:
            task = self._read_task(run_id, task_id)
            hook_summary = self._hook_summary(task)
            runtime_seconds = None
            if task.get("started_at"):
                runtime_seconds = int((task.get("finished_at") or time.time()) - task["started_at"])
            summaries.append({
                "id": task["id"],
                "status": task["status"],
                "worker": task["worker"],
                "pid": task.get("pid"),
                "exit_code": task.get("exit_code"),
                "runtime_seconds": runtime_seconds,
                "branch": task["branch"],
                "worktree": task["worktree"],
                "worktree_exists": Path(task["worktree"]).exists(),
                "branch_exists": self._branch_exists(task["branch"]),
                "manifest_exists": (Path(task["worktree"]) / ".subdispatch" / "result.json").exists(),
                "changed_files_count": len(
                    self._changed_files(task["base_commit"], task["branch"], Path(task["worktree"]))
                ) if Path(task["worktree"]).exists() else 0,
                "last_event_at": hook_summary.get("last_event_at"),
                "last_event_name": hook_summary.get("last_event_name"),
                "event_count": hook_summary.get("event_count", 0),
                "transcript_path": hook_summary.get("transcript_path"),
                "agent_transcript_path": hook_summary.get("agent_transcript_path"),
                "last_tool_name": hook_summary.get("last_tool_name"),
                "last_assistant_message_tail": hook_summary.get("last_assistant_message_tail"),
                "error": task.get("error"),
            })
        return summaries

    def _render_prompt(self, task: Dict[str, Any], result_path: Path) -> str:
        lines = [
            "You are a SubDispatch child coding agent working in an isolated git worktree.",
            f"Goal: {task['goal']}",
            f"Task: {task['instruction']}",
            f"Read scope: {task.get('read_scope', [])}",
            f"Write scope: {task.get('write_scope', [])}",
            f"Forbidden paths: {task.get('forbidden_paths', [])}",
            f"Write a JSON result manifest to: {result_path}",
            "Do not modify any worktree outside the current directory.",
            "Do not read or modify secrets, home directory files, or unrelated repositories.",
            "Do not run destructive commands such as rm -rf, git reset --hard, or force pushes.",
            "Do not merge, push, or delete branches.",
        ]
        extra_context = self._task_context(task)
        if extra_context:
            lines.extend([
                "",
                "Primary-agent supplied context follows. Treat it as authoritative even if the worktree files differ.",
                extra_context,
            ])
        return "\n".join(lines)

    def _task_context(self, task: Dict[str, Any]) -> str:
        chunks = []
        if task.get("context"):
            chunks.append("## Inline context\n" + str(task["context"]))
        for raw_path in task.get("context_files", []):
            path = (self.workspace / raw_path).resolve()
            if not path.is_relative_to(self.workspace):
                chunks.append(f"## Context file skipped: {raw_path}\nPath is outside the primary workspace.")
                continue
            if not path.exists() or not path.is_file():
                chunks.append(f"## Context file missing: {raw_path}")
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            chunks.append(f"## Context file: {raw_path}\n{text}")
        return "\n\n".join(chunks)

    def _scope_check(self, changed_files: List[str], write_scope: List[str]) -> Dict[str, Any]:
        if not write_scope:
            return {"ok": True, "violations": []}
        violations = [
            path for path in changed_files
            if not any(path == scope.rstrip("/") or path.startswith(scope.rstrip("/") + "/")
                       for scope in write_scope)
        ]
        return {"ok": not violations, "violations": violations}

    def _forbidden_path_check(self, changed_files: List[str], forbidden_paths: List[str]) -> Dict[str, Any]:
        violations = [
            path for path in changed_files
            if any(path == forbidden.rstrip("/") or path.startswith(forbidden.rstrip("/") + "/")
                   for forbidden in forbidden_paths)
        ]
        return {"ok": not violations, "violations": violations}

    def _changed_files(self, base_commit: str, branch: str, worktree: Path) -> List[str]:
        paths = set()
        output = self._git(["diff", "--name-only", f"{base_commit}...{branch}"])
        paths.update(line for line in output.splitlines() if line.strip())
        status = self._git_in(worktree, ["status", "--porcelain"])
        for line in status.splitlines():
            if not line.strip():
                continue
            path = line[3:]
            if " -> " in path:
                path = path.split(" -> ", 1)[1]
            if not self._is_internal_artifact_path(path):
                paths.add(path)
        return sorted(paths)

    def _task_diff(self, base_commit: str, branch: str, worktree: Path) -> str:
        parts = []
        committed = self._git(["diff", f"{base_commit}...{branch}"])
        if committed:
            parts.append(committed)
        dirty = self._git_in(worktree, ["diff", "HEAD"])
        if dirty:
            parts.append(dirty)
        status = self._git_in(worktree, ["status", "--porcelain"])
        for line in status.splitlines():
            if not line.startswith("?? "):
                continue
            path = line[3:]
            if self._is_internal_artifact_path(path):
                continue
            file_path = worktree / path
            if file_path.is_file():
                parts.append(self._untracked_file_diff(worktree, path))
        return "\n".join(part for part in parts if part)

    def _is_internal_artifact_path(self, path: str) -> bool:
        return (
            path == ".subdispatch"
            or path.startswith(".subdispatch/")
            or path == ".claude"
            or path.startswith(".claude/")
            or path == ".pytest_cache"
            or path.startswith(".pytest_cache/")
            or path == "uv.lock"
        )

    def _hook_summary(self, task: Dict[str, Any]) -> Dict[str, Any]:
        path = Path(
            task.get("hook_summary_path")
            or self._task_dir(task["run_id"], task["id"]) / "hook_summary.json"
        )
        if not path.exists():
            return {}
        try:
            return self._read_json(path)
        except json.JSONDecodeError:
            return {"error": "invalid hook summary json", "hook_summary_path": str(path)}

    def _hook_events_tail(self, task: Dict[str, Any], limit: int = 20) -> List[Dict[str, Any]]:
        path = Path(
            task.get("hook_events_path")
            or self._task_dir(task["run_id"], task["id"]) / "hook_events.jsonl"
        )
        if not path.exists():
            return []
        events = []
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines()[-limit:]:
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                events.append({"error": "invalid hook event json", "raw": line})
        return events

    def _transcript_tail(self, task: Dict[str, Any], limit: int = 8000) -> str:
        summary = self._hook_summary(task)
        transcript = summary.get("agent_transcript_path") or summary.get("transcript_path")
        if not transcript:
            return ""
        path = Path(transcript).expanduser()
        if not path.exists() or not path.is_file():
            return ""
        return self._tail(path, limit=limit)

    def _untracked_file_diff(self, worktree: Path, path: str) -> str:
        completed = subprocess.run(
            ["git", "diff", "--no-index", "--", "/dev/null", path],
            cwd=worktree,
            text=True,
            capture_output=True,
        )
        if completed.returncode not in {0, 1}:
            raise RuntimeError(completed.stderr.strip() or completed.stdout.strip())
        return completed.stdout

    def _branch_exists(self, branch: str) -> bool:
        try:
            self._git(["rev-parse", "--verify", branch])
            return True
        except RuntimeError:
            return False

    def _command_available(self, command: List[str]) -> bool:
        if not command:
            return False
        executable = command[0]
        if os.path.isabs(executable) or "/" in executable:
            return Path(executable).exists()
        return shutil.which(executable) is not None

    def _ensure_git_repo(self) -> None:
        self._git(["rev-parse", "--show-toplevel"])

    def _git(self, args: List[str]) -> str:
        completed = subprocess.run(
            ["git", *args],
            cwd=self.workspace,
            text=True,
            capture_output=True,
        )
        if completed.returncode != 0:
            raise RuntimeError(completed.stderr.strip() or completed.stdout.strip())
        return completed.stdout

    def _git_in(self, cwd: Path, args: List[str]) -> str:
        completed = subprocess.run(
            ["git", *args],
            cwd=cwd,
            text=True,
            capture_output=True,
        )
        if completed.returncode != 0:
            raise RuntimeError(completed.stderr.strip() or completed.stdout.strip())
        return completed.stdout

    def _run_dir(self, run_id: str) -> Path:
        return self.runs_dir / run_id

    def _task_dir(self, run_id: str, task_id: str) -> Path:
        return self._run_dir(run_id) / "tasks" / task_id

    def _read_run(self, run_id: str) -> Dict[str, Any]:
        return self._read_json(self._run_dir(run_id) / "run.json")

    def _read_task(self, run_id: str, task_id: str) -> Dict[str, Any]:
        return self._read_json(self._task_dir(run_id, task_id) / "task.json")

    def _write_task(self, run_id: str, task_id: str, task: Dict[str, Any]) -> None:
        self._write_json(self._task_dir(run_id, task_id) / "task.json", task)

    def _read_json(self, path: Path) -> Dict[str, Any]:
        return json.loads(path.read_text(encoding="utf-8"))

    def _write_json(self, path: Path, data: Dict[str, Any]) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    def _tail(self, path: Path, limit: int = 4000) -> str:
        if not path.exists():
            return ""
        text = path.read_text(encoding="utf-8", errors="replace")
        return text[-limit:]

    def _new_run_id(self) -> str:
        return f"run_{time.strftime('%Y%m%d_%H%M%S')}_{os.getpid()}"


def default_workers(workspace: str | Path = ".", env: Dict[str, str] | None = None) -> Dict[str, WorkerConfig]:
    settings = env or load_env(Path(workspace).resolve())
    configured_workers = [
        worker_id.strip()
        for worker_id in settings.get("SUBDISPATCH_WORKERS", "").split(",")
        if worker_id.strip()
    ]
    if configured_workers:
        return {
            worker_id: worker_from_env(worker_id, settings)
            for worker_id in configured_workers
        }
    return {"claude-code": default_claude_worker(settings)}


def default_claude_worker(settings: Dict[str, str]) -> WorkerConfig:
    model = settings.get("SUBDISPATCH_CLAUDE_MODEL")
    max_concurrency = int(settings.get("SUBDISPATCH_CLAUDE_MAX_CONCURRENCY", "1"))
    worker_mode = settings.get("SUBDISPATCH_WORKER_MODE", "trusted-worktree")
    permission_mode = settings.get("SUBDISPATCH_CLAUDE_PERMISSION_MODE", "bypassPermissions")
    command_text = settings.get(
        "SUBDISPATCH_CLAUDE_COMMAND",
        "claude -p $prompt --permission-mode $permission_mode --output-format text",
    )
    command = split_command(command_text)
    if model and "$model" not in command_text:
        command.extend(["--model", "$model"])
    worker_env = {}
    for key in ("ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"):
        if settings.get(key):
            worker_env[key] = settings[key]
    return WorkerConfig(
        id="claude-code",
        command=command,
        max_concurrency=max_concurrency,
        model=model,
        enabled=settings.get("SUBDISPATCH_CLAUDE_ENABLED", "1") != "0",
            env=worker_env,
            worker_mode=worker_mode,
            permission_mode=permission_mode,
            description=settings.get(
                "SUBDISPATCH_CLAUDE_DESCRIPTION",
                "Default Claude Code worker for general coding tasks.",
            ),
            strengths=csv_list(settings.get(
                "SUBDISPATCH_CLAUDE_STRENGTHS",
                "general coding,repository edits,tests,documentation",
            )),
            cost=settings.get("SUBDISPATCH_CLAUDE_COST", "unknown"),
            speed=settings.get("SUBDISPATCH_CLAUDE_SPEED", "unknown"),
        )


def worker_from_env(worker_id: str, settings: Dict[str, str]) -> WorkerConfig:
    prefix = f"SUBDISPATCH_WORKER_{env_key(worker_id)}_"
    model = settings.get(prefix + "MODEL")
    max_concurrency = int(settings.get(prefix + "MAX_CONCURRENCY", "1"))
    worker_mode = settings.get(prefix + "MODE", settings.get("SUBDISPATCH_WORKER_MODE", "trusted-worktree"))
    permission_mode = settings.get(
        prefix + "PERMISSION_MODE",
        settings.get("SUBDISPATCH_CLAUDE_PERMISSION_MODE", "bypassPermissions"),
    )
    command_text = settings.get(
        prefix + "COMMAND",
        settings.get(
            "SUBDISPATCH_CLAUDE_COMMAND",
            "claude -p $prompt --permission-mode $permission_mode --output-format text",
        ),
    )
    command = split_command(command_text)
    if model and "$model" not in command_text:
        command.extend(["--model", "$model"])
    worker_env = {
        key.removeprefix(prefix + "ENV_"): value
        for key, value in settings.items()
        if key.startswith(prefix + "ENV_")
    }
    return WorkerConfig(
        id=worker_id,
        command=command,
        max_concurrency=max_concurrency,
        model=model,
        enabled=settings.get(prefix + "ENABLED", "1") != "0",
        env=worker_env,
        worker_mode=worker_mode,
        permission_mode=permission_mode,
        description=settings.get(prefix + "DESCRIPTION", f"{worker_id} Claude Code worker."),
        strengths=csv_list(settings.get(prefix + "STRENGTHS", "general coding")),
        cost=settings.get(prefix + "COST", "unknown"),
        speed=settings.get(prefix + "SPEED", "unknown"),
    )


def env_key(value: str) -> str:
    return "".join(ch if ch.isalnum() else "_" for ch in value).upper()


def load_env(workspace: str | Path = ".") -> Dict[str, str]:
    values = dict(os.environ)
    env_path = Path(workspace).resolve() / ".env"
    if not env_path.exists():
        return values
    for raw_line in env_path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip()
        if (value.startswith('"') and value.endswith('"')) or (
            value.startswith("'") and value.endswith("'")
        ):
            value = value[1:-1]
        values[key] = value
    return values


def split_command(command: str) -> List[str]:
    import shlex

    return shlex.split(command)


def csv_list(value: str) -> List[str]:
    return [item.strip() for item in value.split(",") if item.strip()]


ENV_TEMPLATE = """# SubDispatch local configuration.
# Copy this file to .env and edit values for this workspace.
# .env is intentionally git-ignored.

# Enable or disable the default Claude Code worker.
SUBDISPATCH_CLAUDE_ENABLED=1

# The MVP runs child agents in trusted worktrees. This is not a security
# sandbox; it is an isolated git worktree plus hook/session observation and
# artifact review.
SUBDISPATCH_WORKER_MODE=trusted-worktree

# Default to high-permission child execution so delegated agents can run full
# coding loops. Use only with trusted repositories and credentials.
SUBDISPATCH_CLAUDE_PERMISSION_MODE=bypassPermissions

# SubDispatch injects temporary Claude Code hooks into each child worktree under
# .claude/settings.local.json. Hook events are recorded under .subdispatch/runs
# and are returned by poll-run and collect-task. These files are treated as
# internal observation artifacts and are excluded from collected task diffs.

# Worker metadata shown to the primary agent for routing decisions.
SUBDISPATCH_CLAUDE_DESCRIPTION=Default Claude Code worker for general coding tasks.
SUBDISPATCH_CLAUDE_STRENGTHS=general coding,repository edits,tests,documentation
SUBDISPATCH_CLAUDE_COST=unknown
SUBDISPATCH_CLAUDE_SPEED=unknown

# Claude Code command template. SubDispatch replaces $prompt, $prompt_path,
# $result_path, $run_id, $task_id, $worktree, $model, $worker_mode, and
# $permission_mode.
SUBDISPATCH_CLAUDE_COMMAND=claude -p $prompt --permission-mode $permission_mode --output-format text

# Optional Claude model. When set, SubDispatch appends: --model $model
# SUBDISPATCH_CLAUDE_MODEL=claude-sonnet-4-5

# Maximum number of Claude Code child agents allowed to run at once.
SUBDISPATCH_CLAUDE_MAX_CONCURRENCY=1

# Optional Claude API configuration. Leave empty if Claude Code is already
# authenticated globally.
# ANTHROPIC_API_KEY=
# ANTHROPIC_BASE_URL=

# Multiple worker mode. Uncomment to define explicit workers instead of the
# default claude-code worker above.
# SUBDISPATCH_WORKERS=glm,minimax

# GLM worker example.
# SUBDISPATCH_WORKER_GLM_ENABLED=1
# SUBDISPATCH_WORKER_GLM_MODEL=glm-5.1
# SUBDISPATCH_WORKER_GLM_MAX_CONCURRENCY=2
# SUBDISPATCH_WORKER_GLM_DESCRIPTION=Balanced worker for Chinese/English coding tasks, repo edits, and reasoning-heavy implementation.
# SUBDISPATCH_WORKER_GLM_STRENGTHS=general coding,Chinese context,reasoning,tests,documentation
# SUBDISPATCH_WORKER_GLM_COST=medium
# SUBDISPATCH_WORKER_GLM_SPEED=medium
# SUBDISPATCH_WORKER_GLM_PERMISSION_MODE=bypassPermissions
# SUBDISPATCH_WORKER_GLM_COMMAND=claude -p $prompt --permission-mode $permission_mode --output-format text
# SUBDISPATCH_WORKER_GLM_ENV_ANTHROPIC_BASE_URL=https://open.bigmodel.cn/api/anthropic
# SUBDISPATCH_WORKER_GLM_ENV_ANTHROPIC_API_KEY=

# MiniMax worker example.
# SUBDISPATCH_WORKER_MINIMAX_ENABLED=1
# SUBDISPATCH_WORKER_MINIMAX_MODEL=MiniMax-M2.7
# SUBDISPATCH_WORKER_MINIMAX_MAX_CONCURRENCY=3
# SUBDISPATCH_WORKER_MINIMAX_DESCRIPTION=Fast lower-cost worker for parallel simple edits, docs, search, and small scoped changes.
# SUBDISPATCH_WORKER_MINIMAX_STRENGTHS=parallel throughput,simple edits,documentation,code search,boilerplate
# SUBDISPATCH_WORKER_MINIMAX_COST=low
# SUBDISPATCH_WORKER_MINIMAX_SPEED=fast
# SUBDISPATCH_WORKER_MINIMAX_PERMISSION_MODE=bypassPermissions
# SUBDISPATCH_WORKER_MINIMAX_COMMAND=claude -p $prompt --permission-mode $permission_mode --output-format text
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_BASE_URL=https://api.minimaxi.com/anthropic
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_AUTH_TOKEN=
# SUBDISPATCH_WORKER_MINIMAX_ENV_API_TIMEOUT_MS=3000000
# SUBDISPATCH_WORKER_MINIMAX_ENV_CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_MODEL=MiniMax-M2.7
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_DEFAULT_SONNET_MODEL=MiniMax-M2.7
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_DEFAULT_OPUS_MODEL=MiniMax-M2.7
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_DEFAULT_HAIKU_MODEL=MiniMax-M2.7
"""


def init_env(workspace: str | Path = ".", overwrite: bool = False) -> Dict[str, Any]:
    root = Path(workspace).resolve()
    root.mkdir(parents=True, exist_ok=True)
    example_path = root / ".env.example"
    env_path = root / ".env"
    example_changed = overwrite or not example_path.exists()
    env_created = False
    if example_changed:
        example_path.write_text(ENV_TEMPLATE, encoding="utf-8")
    if overwrite or not env_path.exists():
        env_path.write_text(ENV_TEMPLATE, encoding="utf-8")
        env_created = True
    return {
        "status": "ok",
        "env_path": str(env_path),
        "env_created": env_created,
        "example_path": str(example_path),
        "example_changed": example_changed,
        "next_step": "Edit .env, then run `python cli.py workers`.",
    }
