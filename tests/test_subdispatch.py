from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path

import pytest

from cli import main
from subdispatch_mcp import subdispatch_tool_schemas
from subdispatch.subdispatch import SubDispatchEngine, WorkerConfig, init_env


def init_repo(path: Path) -> None:
    subprocess.run(["git", "init", "-q"], cwd=path, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=path, check=True)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=path, check=True)
    (path / "README.md").write_text("hello\n", encoding="utf-8")
    subprocess.run(["git", "add", "README.md"], cwd=path, check=True)
    subprocess.run(["git", "commit", "-q", "-m", "init"], cwd=path, check=True)


def fake_worker_command() -> list[str]:
    script = (
        "import json, os, pathlib; "
        "pathlib.Path('notes.txt').write_text('child work\\n'); "
        "result=pathlib.Path(os.environ['SUBDISPATCH_RESULT_PATH']); "
        "result.parent.mkdir(parents=True, exist_ok=True); "
        "result.write_text(json.dumps({'status':'success','summary':'done'}))"
    )
    return [sys.executable, "-c", script]


def sleep_worker_command() -> list[str]:
    return [sys.executable, "-c", "import time; time.sleep(2)"]


def delayed_manifest_worker_command() -> list[str]:
    script = (
        "import json, os, pathlib, time; "
        "time.sleep(1); "
        "pathlib.Path('delayed.txt').write_text('done\\n'); "
        "result=pathlib.Path(os.environ['SUBDISPATCH_RESULT_PATH']); "
        "result.parent.mkdir(parents=True, exist_ok=True); "
        "result.write_text(json.dumps({'status':'success','summary':'delayed'}))"
    )
    return [sys.executable, "-c", script]


def capture_prompt_worker_command() -> list[str]:
    script = (
        "import json, os, pathlib; "
        "prompt=pathlib.Path(os.environ['SUBDISPATCH_PROMPT_PATH']).read_text(); "
        "pathlib.Path('prompt_snapshot.txt').write_text(prompt); "
        "result=pathlib.Path(os.environ['SUBDISPATCH_RESULT_PATH']); "
        "result.parent.mkdir(parents=True, exist_ok=True); "
        "result.write_text(json.dumps({'status':'success','summary':'captured'}))"
    )
    return [sys.executable, "-c", script]


def test_list_workers_reports_capacity(tmp_path):
    init_repo(tmp_path)
    engine = SubDispatchEngine(
        workspace=str(tmp_path),
        workers={
            "fake": WorkerConfig(id="fake", command=fake_worker_command(), max_concurrency=2)
        },
    )

    result = engine.list_workers()

    assert result["status"] == "ok"
    worker = result["workers"][0]
    assert worker["id"] == "fake"
    assert worker["available_slots"] == 2
    assert worker["enabled"] is True


def test_cli_workers_lists_default_worker(tmp_path, capsys):
    init_repo(tmp_path)
    assert main(["workers", "--workspace", str(tmp_path)]) == 0
    output = json.loads(capsys.readouterr().out)
    assert output["status"] == "ok"
    assert output["workers"][0]["id"] == "claude-code"


def test_workers_load_project_env(tmp_path):
    init_repo(tmp_path)
    (tmp_path / ".env").write_text(
        "\n".join([
            f"SUBDISPATCH_CLAUDE_COMMAND={sys.executable} -c 'print(1)'",
            "SUBDISPATCH_CLAUDE_MODEL=claude-test",
            "SUBDISPATCH_CLAUDE_MAX_CONCURRENCY=3",
            "ANTHROPIC_API_KEY=sk-test",
        ]),
        encoding="utf-8",
    )

    result = SubDispatchEngine(workspace=str(tmp_path)).list_workers()

    worker = result["workers"][0]
    assert worker["command"] == [sys.executable, "-c", "print(1)", "--model", "$model"]
    assert worker["model"] == "claude-test"
    assert worker["max_concurrency"] == 3


def test_default_worker_reports_trusted_bypass_mode(tmp_path):
    init_repo(tmp_path)

    result = SubDispatchEngine(workspace=str(tmp_path)).list_workers()

    worker = result["workers"][0]
    assert worker["worker_mode"] == "trusted-worktree"
    assert worker["permission_mode"] == "bypassPermissions"
    assert worker["sandbox"] == "none"
    assert worker["risk"] == "high"


def test_multiple_workers_load_from_env_without_leaking_tokens(tmp_path):
    init_repo(tmp_path)
    (tmp_path / ".env").write_text(
        "\n".join([
            "SUBDISPATCH_WORKERS=glm,minimax",
            "SUBDISPATCH_WORKER_GLM_MODEL=glm-5.1",
            "SUBDISPATCH_WORKER_GLM_MAX_CONCURRENCY=2",
            "SUBDISPATCH_WORKER_GLM_DESCRIPTION=Balanced worker",
            "SUBDISPATCH_WORKER_GLM_STRENGTHS=general coding,tests",
            "SUBDISPATCH_WORKER_GLM_COST=medium",
            "SUBDISPATCH_WORKER_GLM_SPEED=medium",
            "SUBDISPATCH_WORKER_GLM_ENV_ANTHROPIC_BASE_URL=https://open.bigmodel.cn/api/anthropic",
            "SUBDISPATCH_WORKER_GLM_ENV_ANTHROPIC_API_KEY=glm-secret",
            "SUBDISPATCH_WORKER_MINIMAX_MODEL=MiniMax-M2.7",
            "SUBDISPATCH_WORKER_MINIMAX_MAX_CONCURRENCY=3",
            "SUBDISPATCH_WORKER_MINIMAX_DESCRIPTION=Fast worker",
            "SUBDISPATCH_WORKER_MINIMAX_STRENGTHS=simple edits,docs",
            "SUBDISPATCH_WORKER_MINIMAX_COST=low",
            "SUBDISPATCH_WORKER_MINIMAX_SPEED=fast",
            "SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_BASE_URL=https://api.minimaxi.com/anthropic",
            "SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_AUTH_TOKEN=minimax-secret",
            "SUBDISPATCH_WORKER_MINIMAX_ENV_API_TIMEOUT_MS=3000000",
        ]),
        encoding="utf-8",
    )

    result = SubDispatchEngine(workspace=str(tmp_path)).list_workers()

    workers = {worker["id"]: worker for worker in result["workers"]}
    assert workers["glm"]["model"] == "glm-5.1"
    assert workers["glm"]["max_concurrency"] == 2
    assert workers["glm"]["description"] == "Balanced worker"
    assert workers["glm"]["strengths"] == ["general coding", "tests"]
    assert workers["glm"]["cost"] == "medium"
    assert workers["glm"]["speed"] == "medium"
    assert workers["minimax"]["model"] == "MiniMax-M2.7"
    assert workers["minimax"]["max_concurrency"] == 3
    assert workers["minimax"]["description"] == "Fast worker"
    assert workers["minimax"]["strengths"] == ["simple edits", "docs"]
    assert workers["minimax"]["cost"] == "low"
    assert workers["minimax"]["speed"] == "fast"
    assert "secret" not in json.dumps(result)


def test_init_env_creates_templates(tmp_path):
    result = init_env(tmp_path)

    assert result["status"] == "ok"
    assert (tmp_path / ".env").exists()
    assert (tmp_path / ".env.example").exists()
    assert "SUBDISPATCH_CLAUDE_COMMAND" in (tmp_path / ".env.example").read_text(encoding="utf-8")


def test_mcp_exposes_subdispatch_tools():
    names = {tool["name"] for tool in subdispatch_tool_schemas()}
    assert names == {
        "list_workers",
        "start_run",
        "poll_run",
        "collect_task",
        "delete_worktree",
    }


def test_start_run_embeds_primary_context_files(tmp_path):
    init_repo(tmp_path)
    (tmp_path / ".subdispatch").mkdir()
    (tmp_path / ".subdispatch" / "note.txt").write_text("uncommitted rename evidence", encoding="utf-8")
    engine = SubDispatchEngine(
        workspace=str(tmp_path),
        workers={
            "fake": WorkerConfig(id="fake", command=capture_prompt_worker_command(), max_concurrency=1)
        },
    )

    started = engine.start_run({
        "run_id": "run_context",
        "goal": "verify prompt context",
        "tasks": [{
            "id": "task_context",
            "instruction": "capture prompt",
            "worker": "fake",
            "context": "inline audit context",
            "context_files": [".subdispatch/note.txt"],
        }],
    })
    assert started["status"] == "ok"

    poll = wait_for_task(engine, "run_context", "task_context", "completed")
    assert poll["status"] == "completed"
    artifact = engine.collect_task({"run_id": "run_context", "task_id": "task_context"})

    assert artifact["changed_files"] == ["prompt_snapshot.txt"]
    prompt_snapshot = Path(artifact["worktree"]) / "prompt_snapshot.txt"
    prompt = prompt_snapshot.read_text(encoding="utf-8")
    assert "inline audit context" in prompt
    assert "uncommitted rename evidence" in prompt


def test_start_poll_collect_and_delete_worktree(tmp_path):
    init_repo(tmp_path)
    engine = SubDispatchEngine(
        workspace=str(tmp_path),
        workers={
            "fake": WorkerConfig(id="fake", command=fake_worker_command(), max_concurrency=1)
        },
    )

    started = engine.start_run({
        "run_id": "run_test",
        "goal": "test child task execution",
        "tasks": [{
            "id": "task_a",
            "instruction": "write notes",
            "worker": "fake",
            "write_scope": ["notes.txt"],
            "forbidden_paths": ["secrets"],
        }],
    })

    assert started["run_id"] == "run_test"
    assert started["tasks"][0]["status"] in {"queued", "running"}

    poll = wait_for_task(engine, "run_test", "task_a", "completed")
    assert poll["status"] == "completed"

    artifact = engine.collect_task({"run_id": "run_test", "task_id": "task_a"})
    assert artifact["changed_files"] == ["notes.txt"]
    assert "child work" in artifact["diff"]
    assert artifact["manifest"]["summary"] == "done"
    assert artifact["hook_summary"]["event_count"] == 0
    assert artifact["hook_events_tail"] == []
    assert artifact["transcript_tail"] == ""
    assert artifact["scope_check"]["ok"] is True
    assert artifact["forbidden_path_check"]["ok"] is True
    assert Path(artifact["patch_path"]).exists()

    deleted = engine.delete_worktree({"run_id": "run_test", "task_id": "task_a"})
    assert deleted["worktree_removed"] is True
    assert Path(deleted["artifact_dir"]).exists()


def test_concurrency_limit_queues_extra_tasks(tmp_path):
    init_repo(tmp_path)
    engine = SubDispatchEngine(
        workspace=str(tmp_path),
        workers={
            "slow": WorkerConfig(id="slow", command=sleep_worker_command(), max_concurrency=1)
        },
    )

    engine.start_run({
        "run_id": "run_queue",
        "goal": "queue tasks",
        "tasks": [
            {"id": "first", "instruction": "sleep", "worker": "slow"},
            {"id": "second", "instruction": "sleep", "worker": "slow"},
        ],
    })

    poll = engine.poll_run({"run_id": "run_queue"})
    statuses = {task["id"]: task["status"] for task in poll["tasks"]}
    assert list(statuses.values()).count("running") == 1
    assert list(statuses.values()).count("queued") == 1


def test_poll_from_new_engine_does_not_mark_detached_process_complete_early(tmp_path):
    init_repo(tmp_path)
    workers = {
        "slow": WorkerConfig(
            id="slow",
            command=delayed_manifest_worker_command(),
            max_concurrency=1,
        )
    }
    engine = SubDispatchEngine(workspace=str(tmp_path), workers=workers)

    engine.start_run({
        "run_id": "run_detached_poll",
        "goal": "detached status",
        "tasks": [{"id": "slow_task", "instruction": "sleep then write", "worker": "slow"}],
    })

    same_workspace_new_engine = SubDispatchEngine(workspace=str(tmp_path), workers=workers)
    early_poll = same_workspace_new_engine.poll_run({"run_id": "run_detached_poll"})
    early_status = early_poll["tasks"][0]["status"]

    assert early_status == "running"

    final_poll = wait_for_task(
        same_workspace_new_engine,
        "run_detached_poll",
        "slow_task",
        "completed",
    )
    assert final_poll["status"] == "completed"


def test_start_run_installs_claude_hook_observation_files(tmp_path):
    init_repo(tmp_path)
    engine = SubDispatchEngine(
        workspace=str(tmp_path),
        workers={
            "fake": WorkerConfig(id="fake", command=fake_worker_command(), max_concurrency=1)
        },
    )

    started = engine.start_run({
        "run_id": "run_hooks",
        "goal": "hook observation",
        "tasks": [{"id": "task_a", "instruction": "write notes", "worker": "fake"}],
    })

    task = started["tasks"][0]
    worktree = Path(task["worktree"])
    settings_path = worktree / ".claude" / "settings.local.json"
    recorder_path = worktree / ".claude" / "hooks" / "subdispatch_hook_recorder.py"

    assert settings_path.exists()
    assert recorder_path.exists()
    settings = json.loads(settings_path.read_text(encoding="utf-8"))
    assert "PostToolUse" in settings["hooks"]
    assert "Stop" in settings["hooks"]

    poll = wait_for_task(engine, "run_hooks", "task_a", "completed")
    poll_task = poll["tasks"][0]
    assert poll_task["runtime_seconds"] >= 0
    assert poll_task["event_count"] == 0


def test_poll_and_collect_include_hook_session_summary(tmp_path):
    init_repo(tmp_path)
    engine = SubDispatchEngine(
        workspace=str(tmp_path),
        workers={
            "fake": WorkerConfig(id="fake", command=fake_worker_command(), max_concurrency=1)
        },
    )

    engine.start_run({
        "run_id": "run_hook_summary",
        "goal": "hook summary",
        "tasks": [{"id": "task_a", "instruction": "write notes", "worker": "fake"}],
    })
    wait_for_task(engine, "run_hook_summary", "task_a", "completed")
    task_dir = tmp_path / ".subdispatch" / "runs" / "run_hook_summary" / "tasks" / "task_a"
    transcript_path = task_dir / "transcript.jsonl"
    transcript_path.write_text('{"type":"assistant","message":"planning work"}\n', encoding="utf-8")
    hook_summary = {
        "event_count": 2,
        "last_event_at": 123.0,
        "last_event_name": "PostToolUse",
        "last_session_id": "session-1",
        "transcript_path": str(transcript_path),
        "last_tool_name": "Edit",
        "last_assistant_message_tail": "planning work",
    }
    (task_dir / "hook_summary.json").write_text(
        json.dumps(hook_summary, ensure_ascii=False),
        encoding="utf-8",
    )
    event = {
        "hook_event_name": "PostToolUse",
        "session_id": "session-1",
        "transcript_path": str(transcript_path),
    }
    (task_dir / "hook_events.jsonl").write_text(
        json.dumps(event, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )

    poll = engine.poll_run({"run_id": "run_hook_summary"})
    poll_task = poll["tasks"][0]
    assert poll_task["event_count"] == 2
    assert poll_task["last_event_name"] == "PostToolUse"
    assert poll_task["transcript_path"] == str(transcript_path)
    assert poll_task["last_assistant_message_tail"] == "planning work"

    artifact = engine.collect_task({"run_id": "run_hook_summary", "task_id": "task_a"})
    assert artifact["hook_summary"]["last_session_id"] == "session-1"
    assert artifact["hook_events_tail"][0]["hook_event_name"] == "PostToolUse"
    assert "planning work" in artifact["transcript_tail"]


def test_claude_observation_files_are_excluded_from_artifact_diff(tmp_path):
    init_repo(tmp_path)
    command = [
        sys.executable,
        "-c",
        "import pathlib; pathlib.Path('.claude/noise.txt').write_text('noise\\n')",
    ]
    engine = SubDispatchEngine(
        workspace=str(tmp_path),
        workers={"fake": WorkerConfig(id="fake", command=command, max_concurrency=1)},
    )

    engine.start_run({
        "run_id": "run_claude_noise",
        "goal": "ignore observation files",
        "tasks": [{"id": "task_a", "instruction": "write observation noise", "worker": "fake"}],
    })
    wait_for_task(engine, "run_claude_noise", "task_a", "completed")

    artifact = engine.collect_task({"run_id": "run_claude_noise", "task_id": "task_a"})

    assert artifact["changed_files"] == []
    assert artifact["diff"] == ""


def test_common_tool_noise_is_excluded_from_artifact_diff(tmp_path):
    init_repo(tmp_path)
    command = [
        sys.executable,
        "-c",
        (
            "import pathlib; "
            "pathlib.Path('uv.lock').write_text('generated\\n'); "
            "pathlib.Path('.pytest_cache').mkdir(); "
            "pathlib.Path('.pytest_cache/README.md').write_text('cache\\n')"
        ),
    ]
    engine = SubDispatchEngine(
        workspace=str(tmp_path),
        workers={"fake": WorkerConfig(id="fake", command=command, max_concurrency=1)},
    )

    engine.start_run({
        "run_id": "run_tool_noise",
        "goal": "ignore tool noise",
        "tasks": [{"id": "task_a", "instruction": "write tool noise", "worker": "fake"}],
    })
    wait_for_task(engine, "run_tool_noise", "task_a", "completed")

    artifact = engine.collect_task({"run_id": "run_tool_noise", "task_id": "task_a"})

    assert artifact["changed_files"] == []
    assert artifact["diff"] == ""


def test_collect_reports_scope_and_forbidden_violations(tmp_path):
    init_repo(tmp_path)
    command = [
        sys.executable,
        "-c",
        "import pathlib; pathlib.Path('blocked.txt').write_text('bad\\n')",
    ]
    engine = SubDispatchEngine(
        workspace=str(tmp_path),
        workers={"fake": WorkerConfig(id="fake", command=command, max_concurrency=1)},
    )

    engine.start_run({
        "run_id": "run_violation",
        "goal": "detect bad paths",
        "tasks": [{
            "id": "task_a",
            "instruction": "write forbidden file",
            "worker": "fake",
            "write_scope": ["allowed"],
            "forbidden_paths": ["blocked.txt"],
        }],
    })
    wait_for_task(engine, "run_violation", "task_a", "completed")

    artifact = engine.collect_task({"run_id": "run_violation", "task_id": "task_a"})

    assert artifact["scope_check"]["ok"] is False
    assert artifact["scope_check"]["violations"] == ["blocked.txt"]
    assert artifact["forbidden_path_check"]["ok"] is False
    assert artifact["forbidden_path_check"]["violations"] == ["blocked.txt"]


def wait_for_task(engine: SubDispatchEngine, run_id: str, task_id: str, status: str) -> dict:
    deadline = time.time() + 5
    while time.time() < deadline:
        poll = engine.poll_run({"run_id": run_id})
        task = next(task for task in poll["tasks"] if task["id"] == task_id)
        if task["status"] == status:
            return poll
        time.sleep(0.05)
    raise AssertionError(f"task {task_id} did not reach {status}")
