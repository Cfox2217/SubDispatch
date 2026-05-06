# LeanCodex MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a working CLI wrapper (`leancodex`) that routes low-cost tasks to DeepSeek-TUI while delegating complex tasks to Codex, with JSONL stdio protocol.

**Architecture:** Shell + Python. LeanCodex is a single CLI entry point that reads `task.json`, classifies the task, executes via DeepSeek-TUI CLI, verifies output, and streams JSONL results. The routing decision is rule-based with risk scoring.

**Tech Stack:** Python 3.10+, shell scripting, DeepSeek-TUI CLI

---

## File Structure

```
leancodex/
├── SPEC.md
├── README.md
├── pyproject.toml
├── src/
│   ├── __init__.py
│   ├── cli.py           # CLI entry point, argument parsing
│   ├── router.py        # Task classification + risk scoring
│   ├── packer.py        # Context pruning for DeepSeek
│   ├── runner.py        # DeepSeek-TUI execution (CLI mode)
│   ├── verifier.py     # Diff + test verification
│   ├── fallback.py     # Escalation logic
│   └── models.py       # Task/Result dataclasses
├── bin/
│   └── leancodex       # Shell wrapper that calls src/cli.py
├── tests/
│   ├── test_router.py
│   ├── test_packer.py
│   ├── test_verifier.py
│   └── test_integration.py
└── docs/
    └── ...
```

---

### Task 1: Project Scaffold + pyproject.toml

**Files:**
- Create: `pyproject.toml`
- Create: `src/__init__.py`
- Create: `src/models.py`
- Create: `bin/leancodex`

- [ ] **Step 1: Create pyproject.toml**

```toml
[project]
name = "leancodex"
version = "0.1.0"
description = "Hybrid AI coding router — DeepSeek does the work, Codex makes the decisions."
readme = "README.md"
requires-python = ">=3.10"
dependencies = []

[project.scripts]
leancodex = "leancodex.cli:main"

[build-system]
requires = ["setuptools>=61.0"]
build-backend = "setuptools.build_meta"
```

- [ ] **Step 2: Create src/__init__.py**

```python
"""LeanCodex — Hybrid AI coding router."""
```

- [ ] **Step 3: Create src/models.py**

```python
from dataclasses import dataclass, field
from typing import Literal

@dataclass
class Task:
    id: str
    workspace: str
    instruction: str
    mode: Literal["plan", "edit", "test"] = "edit"
    model: str = "auto"
    allowed_paths: list[str] = field(default_factory=list)
    forbidden_paths: list[str] = field(default_factory=lambda: [
        "auth/*", "security/*", "billing/*", "payments/*",
        "migrations/*", "infra/*", ".github/workflows/*"
    ])
    acceptance: list[str] = field(default_factory=list)

@dataclass
class TaskResult:
    status: Literal["success", "failed", "needs_codex"]
    summary: str
    changed_files: list[str] = field(default_factory=list)
    commands_run: list[str] = field(default_factory=list)
    test_result: str = "skipped"
    risk_notes: list[str] = field(default_factory=list)
    patch: str = ""

@dataclass
class JSONLLine:
    type: Literal["started", "progress", "command", "file_change", "completed"]
    id: str = ""
    message: str = ""
    cmd: str = ""
    path: str = ""
    kind: str = ""
    status: str = ""
    summary: str = ""
    diff: str = ""
```

- [ ] **Step 4: Create bin/leancodex shell wrapper**

```bash
#!/usr/bin/env bash
# LeanCodex CLI wrapper
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$SCRIPT_DIR/../src/cli.py" "$@"
```

- [ ] **Step 5: Make bin/leancodex executable**

Run: `chmod +x bin/leancodex`

- [ ] **Step 6: Commit**

```bash
git add pyproject.toml src/__init__.py src/models.py bin/leancodex
git commit -m "feat: project scaffold with models and CLI wrapper"
```

---

### Task 2: Router — Task Classification + Risk Scoring

**Files:**
- Create: `src/router.py`
- Create: `tests/test_router.py`

- [ ] **Step 1: Write failing test for router**

```python
# tests/test_router.py
import pytest
from leancodex.router import TaskRouter, RoutingDecision

def test_delegate_simple_task():
    router = TaskRouter()
    decision = router.decide("add unit tests for user service", "src/")
    assert decision.delegate_to_deepseek is True
    assert decision.risk_score <= 3

def test_keep_architecture_task():
    router = TaskRouter()
    decision = router.decide("design new authentication architecture", "src/")
    assert decision.delegate_to_deepseek is False

def test_keep_security_sensitive():
    router = TaskRouter()
    decision = router.decide("fix the auth logic", "src/auth/")
    assert decision.delegate_to_deepseek is False

def test_risk_score_calculation():
    router = TaskRouter()
    decision = router.decide("write tests for user service", "src/")
    assert 0 <= decision.risk_score <= 10
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tests/test_router.py -v`
Expected: FAIL — router module not found

- [ ] **Step 3: Write router implementation**

```python
# src/router.py
import re
from dataclasses import dataclass
from typing import Literal

DELEGATE_PATTERNS = [
    r"summarize",
    r"repository",
    r"locate|find.*file",
    r"explain.*code",
    r"write.*test",
    r"simple.*refactor",
    r"fix.*lint",
    r"fix.*type.*error",
    r"update.*doc",
    r"add.*test",
    r"generate.*draft",
]

KEEP_PATTERNS = [
    r"architecture",
    r"security",
    r"auth",
    r"payment",
    r"migration",
    r"database",
    r"ambig",
    r"review.*before.*commit",
    r"final.*review",
]

HIGH_RISK_PATHS = [
    "auth", "security", "billing", "payments",
    "migrations", "infra", ".github/workflows"
]

HIGH_RISK_KEYWORDS = [
    "auth", "security", "billing", "payment", "permission",
    "password", "credential", "token", "encryption"
]

@dataclass
class RoutingDecision:
    delegate_to_deepseek: bool
    risk_score: int
    reason: str
    suggested_action: str

class TaskRouter:
    def __init__(self):
        self.delegate_re = [re.compile(p, re.I) for p in DELEGATE_PATTERNS]
        self.keep_re = [re.compile(p, re.I) for p in KEEP_PATTERNS]

    def decide(self, instruction: str, workspace: str) -> RoutingDecision:
        instruction_lower = instruction.lower()

        # Check keep patterns first (they override)
        for pattern in self.keep_re:
            if pattern.search(instruction_lower):
                return RoutingDecision(
                    delegate_to_deepseek=False,
                    risk_score=8,
                    reason=f"keep pattern matched: {pattern.pattern}",
                    suggested_action="Codex handles this"
                )

        # Check delegate patterns
        for pattern in self.delegate_re:
            if pattern.search(instruction_lower):
                risk = self._calc_risk(instruction_lower, workspace)
                return RoutingDecision(
                    delegate_to_deepseek=risk < 7,
                    risk_score=risk,
                    reason=f"delegate pattern matched: {pattern.pattern}",
                    suggested_action="DeepSeek" if risk < 7 else "Codex"
                )

        # Default: moderate risk, Codex decides
        risk = self._calc_risk(instruction_lower, workspace)
        return RoutingDecision(
            delegate_to_deepseek=False,
            risk_score=risk,
            reason="default — no pattern matched",
            suggested_action="DeepSeek" if risk <= 3 else "Codex"
        )

    def _calc_risk(self, instruction: str, workspace: str) -> int:
        risk = 0

        # Task risk
        if any(kw in instruction for kw in ["refactor", "migrate", "rewrite"]):
            risk += 2
        if "test" in instruction:
            risk -= 1  # Tests are safer

        # File path risk
        for path in HIGH_RISK_PATHS:
            if path in workspace.lower():
                risk += 5
                break

        # Keyword risk
        if any(kw in instruction for kw in HIGH_RISK_KEYWORDS):
            risk += 3

        return max(0, min(10, risk))
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tests/test_router.py -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/router.py tests/test_router.py
git commit -m "feat: add task router with rule-based classification and risk scoring"
```

---

### Task 3: Context Packer

**Files:**
- Create: `src/packer.py`
- Create: `tests/test_packer.py`

- [ ] **Step 1: Write failing test for packer**

```python
# tests/test_packer.py
import pytest
import tempfile
import os
from leancodex.packer import ContextPacker

def test_prune_node_modules():
    with tempfile.TemporaryDirectory() as tmpdir:
        os.makedirs(os.path.join(tmpdir, "node_modules/foo"))
        os.makedirs(os.path.join(tmpdir, "src/foo"))
        os.path.join(tmpdir, "src/main.js")
        with open(os.path.join(tmpdir, "src/main.js"), "w") as f:
            f.write("// code")

        packer = ContextPacker()
        pruned = packer.prune_context(tmpdir, max_files=10)
        paths = [p.replace(tmpdir + "/", "") for p in pruned]

        assert not any("node_modules" in p for p in paths)
        assert any("src/main.js" in p for p in paths)

def test_respect_allowed_paths():
    packer = ContextPacker()
    allowed = ["src/user", "tests/user"]
    filtered = packer.filter_paths(["src/user", "src/auth", "tests/user", "infra"], allowed, [])
    assert "src/auth" not in filtered
    assert "src/user" in filtered
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tests/test_packer.py -v`
Expected: FAIL — packer module not found

- [ ] **Step 3: Write packer implementation**

```python
# src/packer.py
import os
import fnmatch
from typing import list

DEFAULT_EXCLUDE = [
    "node_modules/**",
    ".git/**",
    "__pycache__/**",
    "*.pyc",
    ".venv/**",
    "dist/**",
    "build/**",
    ".next/**",
    "coverage/**",
    ".pytest_cache/**",
]

class ContextPacker:
    def __init__(self, exclude_patterns: list[str] | None = None):
        self.exclude = exclude_patterns or DEFAULT_EXCLUDE

    def prune_context(self, workspace: str, max_files: int = 50) -> list[str]:
        """Return list of file paths to include, pruned of boilerplate."""
        included = []
        for root, dirs, files in os.walk(workspace):
            # Skip excluded directories in-place
            dirs[:] = [d for d in dirs if not self._is_excluded(os.path.join(root, d))]

            for file in files:
                if self._is_excluded(file):
                    continue
                path = os.path.join(root, file)
                included.append(path)
                if len(included) >= max_files:
                    return included
        return included

    def filter_paths(
        self,
        paths: list[str],
        allowed: list[str],
        forbidden: list[str]
    ) -> list[str]:
        """Filter paths by allowed/forbidden rules."""
        result = []
        for path in paths:
            # Check forbidden
            if any(fnmatch.fnmatch(path, f) for f in forbidden):
                continue
            # Check allowed (if set, path must match)
            if allowed and not any(fnmatch.fnmatch(path, a) for a in allowed):
                continue
            result.append(path)
        return result

    def _is_excluded(self, path: str) -> bool:
        for pattern in self.exclude:
            if fnmatch.fnmatch(path, pattern) or pattern in path:
                return True
        return False
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tests/test_packer.py -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/packer.py tests/test_packer.py
git commit -m "feat: add context packer for pruning workspace"
```

---

### Task 4: DeepSeek Runner

**Files:**
- Create: `src/runner.py`
- Create: `tests/test_runner.py`

- [ ] **Step 1: Write failing test for runner**

```python
# tests/test_runner.py
import pytest
from unittest.mock import patch, MagicMock
from leancodex.runner import DeepSeekRunner

def test_build_cli_command():
    runner = DeepSeekRunner()
    cmd = runner._build_command("add tests", "auto")
    assert "deepseek" in cmd
    assert "add tests" in cmd

def test_parse_jsonl_line():
    runner = DeepSeekRunner()
    line = runner._parse_line('{"type":"started","id":"task_001"}')
    assert line["type"] == "started"
    assert line["id"] == "task_001"

def test_parse_jsonl_progress():
    runner = DeepSeekRunner()
    line = runner._parse_line('{"type":"progress","message":"scanning"}')
    assert line["type"] == "progress"
    assert line["message"] == "scanning"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tests/test_runner.py -v`
Expected: FAIL — runner module not found

- [ ] **Step 3: Write runner implementation**

```python
# src/runner.py
import subprocess
import json
import sys
from dataclasses import dataclass
from typing import Generator

@dataclass
class JSONLLine:
    type: str
    id: str = ""
    message: str = ""
    cmd: str = ""
    path: str = ""
    kind: str = ""
    status: str = ""
    summary: str = ""
    diff: str = ""

class DeepSeekRunner:
    def __init__(self, deepseek_path: str = "deepseek"):
        self.deepseek_path = deepseek_path

    def run(self, instruction: str, workspace: str, model: str = "auto") -> Generator[JSONLLine, None, None]:
        """Execute DeepSeek via one-shot CLI, yield JSONL lines."""
        cmd = self._build_command(instruction, model)
        proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=workspace,
            text=True,
        )

        for line in proc.stdout:
            line = line.strip()
            if not line:
                continue
            parsed = self._parse_line(line)
            if parsed:
                yield JSONLLine(**parsed)

        proc.wait()
        if proc.returncode != 0:
            stderr = proc.stderr.read()
            raise RuntimeError(f"DeepSeek failed: {stderr}")

    def _build_command(self, instruction: str, model: str) -> list[str]:
        return [
            self.deepseek_path,
            "--model", model,
            instruction
        ]

    def _parse_line(self, line: str) -> dict | None:
        try:
            return json.loads(line)
        except json.JSONDecodeError:
            return None
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tests/test_runner.py -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/runner.py tests/test_runner.py
git commit -m "feat: add DeepSeek runner with JSONL streaming"
```

---

### Task 5: Verifier — Diff + Test Verification

**Files:**
- Create: `src/verifier.py`
- Create: `tests/test_verifier.py`

- [ ] **Step 1: Write failing test for verifier**

```python
# tests/test_verifier.py
import pytest
import tempfile
import subprocess
from leancodex.verifier import Verifier

def test_git_diff_returns_changed_files():
    with tempfile.TemporaryDirectory() as tmpdir:
        subprocess.run(["git", "init"], cwd=tmpdir, capture_output=True)
        with open(os.path.join(tmpdir, "foo.txt"), "w") as f:
            f.write("hello")
        subprocess.run(["git", "add", "foo.txt"], cwd=tmpdir, capture_output=True)
        subprocess.run(["git", "commit", "-m", "init"], cwd=tmpdir, capture_output=True)
        with open(os.path.join(tmpdir, "foo.txt"), "w") as f:
            f.write("world")

        verifier = Verifier(tmpdir)
        diff, files = verifier.get_git_diff()
        assert "foo.txt" in files

def test_forbidden_path_check():
    verifier = Verifier(".")
    forbidden = ["auth/*", "security/*"]
    # Simulate a changed file
    result = verifier._check_forbidden(["auth/login.go"], forbidden)
    assert result is True  # blocked
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tests/test_verifier.py -v`
Expected: FAIL — verifier module not found

- [ ] **Step 3: Write verifier implementation**

```python
# src/verifier.py
import subprocess
import re
from typing import Tuple

class Verifier:
    def __init__(self, workspace: str):
        self.workspace = workspace

    def get_git_diff(self) -> Tuple[str, list[str]]:
        """Return (diff_text, list_of_changed_files)."""
        result = subprocess.run(
            ["git", "diff", "--name-only"],
            cwd=self.workspace,
            capture_output=True,
            text=True
        )
        files = [f.strip() for f in result.stdout.splitlines() if f.strip()]

        diff_result = subprocess.run(
            ["git", "diff"],
            cwd=self.workspace,
            capture_output=True,
            text=True
        )
        return diff_result.stdout, files

    def check_forbidden(self, changed_files: list[str], forbidden_paths: list[str]) -> list[str]:
        """Return list of forbidden files that were modified."""
        violated = []
        for file in changed_files:
            for pattern in forbidden_paths:
                if self._match_path(file, pattern):
                    violated.append(file)
        return violated

    def run_tests(self, test_cmd: str | None = None) -> Tuple[bool, str]:
        """Run test command. Returns (passed, output)."""
        if not test_cmd:
            # Auto-detect
            for cmd in ["npm test", "pytest", "cargo test", "go test ./..."]:
                result = subprocess.run(
                    cmd.split(),
                    cwd=self.workspace,
                    capture_output=True,
                    text=True,
                    timeout=60
                )
                if result.returncode in (0, 1):  # Ran something
                    return result.returncode == 0, result.stdout + result.stderr
            return False, "no test command found"

        result = subprocess.run(
            test_cmd.split(),
            cwd=self.workspace,
            capture_output=True,
            text=True,
            timeout=120
        )
        return result.returncode == 0, result.stdout + result.stderr

    def _match_path(self, file: str, pattern: str) -> bool:
        """Simple glob match for path patterns."""
        import fnmatch
        pattern = pattern.replace("/*", "/**/*")
        return fnmatch.fnmatch(file, pattern) or fnmatch.fnmatch(file, pattern.replace("/**/*", "/*"))
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tests/test_verifier.py -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/verifier.py tests/test_verifier.py
git commit -m "feat: add verifier for diff parsing and test running"
```

---

### Task 6: Fallback Engine

**Files:**
- Create: `src/fallback.py`
- Create: `tests/test_fallback.py`

- [ ] **Step 1: Write failing test for fallback**

```python
# tests/test_fallback.py
import pytest
from leancodex.fallback import FallbackEngine

def test_should_escalate_on_forbidden_change():
    engine = FallbackEngine()
    decision = engine.should_escalate(
        risk_score=8,
        changed_files=["auth/login.go"],
        test_passed=False,
        deepseek_failed=False
    )
    assert decision.escalate is True
    assert "forbidden" in decision.reason.lower()

def test_no_escalate_low_risk_passed():
    engine = FallbackEngine()
    decision = engine.should_escalate(
        risk_score=2,
        changed_files=["src/utils/foo.go"],
        test_passed=True,
        deepseek_failed=False
    )
    assert decision.escalate is False

def test_escalate_on_deepseek_failure():
    engine = FallbackEngine()
    decision = engine.should_escalate(
        risk_score=3,
        changed_files=["src/foo.go"],
        test_passed=True,
        deepseek_failed=True
    )
    assert decision.escalate is True
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tests/test_fallback.py -v`
Expected: FAIL — fallback module not found

- [ ] **Step 3: Write fallback implementation**

```python
# src/fallback.py
from dataclasses import dataclass

@dataclass
class EscalationDecision:
    escalate: bool
    reason: str
    suggested_action: str

class FallbackEngine:
    def should_escalate(
        self,
        risk_score: int,
        changed_files: list[str],
        test_passed: bool,
        deepseek_failed: bool
    ) -> EscalationDecision:
        # High risk score
        if risk_score >= 7:
            return EscalationDecision(
                escalate=True,
                reason=f"high risk score: {risk_score}",
                suggested_action="Codex handles this"
            )

        # Forbidden file modified
        forbidden_hit = any(
            any(segment in f for segment in ["auth", "security", "billing", "payments", "migrations", "infra"])
            for f in changed_files
        )
        if forbidden_hit:
            return EscalationDecision(
                escalate=True,
                reason="forbidden path modified",
                suggested_action="Codex must review"
            )

        # DeepSeek crashed
        if deepseek_failed:
            return EscalationDecision(
                escalate=True,
                reason="DeepSeek execution failed",
                suggested_action="Escalate to Codex"
            )

        # Tests failed
        if not test_passed:
            return EscalationDecision(
                escalate=True,
                reason="tests failed after DeepSeek execution",
                suggested_action="Codex must investigate"
            )

        return EscalationDecision(
            escalate=False,
            reason="task completed successfully within risk bounds",
            suggested_action="Return result to Codex"
        )
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tests/test_fallback.py -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/fallback.py tests/test_fallback.py
git commit -m "feat: add fallback engine for escalation decisions"
```

---

### Task 7: CLI Entry Point + Main Integration

**Files:**
- Create: `src/cli.py`
- Modify: `bin/leancodex` (already created in Task 1)

- [ ] **Step 1: Write CLI implementation**

```python
# src/cli.py
import sys
import json
import argparse
from pathlib import Path

from leancodex.models import Task, TaskResult
from leancodex.router import TaskRouter
from leancodex.packer import ContextPacker
from leancodex.runner import DeepSeekRunner
from leancodex.verifier import Verifier
from leancodex.fallback import FallbackEngine

def main():
    parser = argparse.ArgumentParser(prog="leancodex")
    parser.add_argument("--task-json", type=str, help="Path to task JSON file")
    parser.add_argument("--deepseek-model", type=str, default="auto")
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--json", action="store_true", help="Machine-readable output")
    args = parser.parse_args()

    # Load task
    if args.task_json:
        task = Task(**json.load(open(args.task_json)))
    else:
        task = Task(**json.load(sys.stdin))

    log = lambda msg: print(f"[LeanCodex] {msg}", file=sys.stderr) if args.verbose else None
    log(f"Task {task.id}: {task.instruction}")

    # Route
    router = TaskRouter()
    decision = router.decide(task.instruction, task.workspace)
    log(f"Routing: {decision.suggested_action} (risk={decision.risk_score})")

    if not decision.delegate_to_deepseek and decision.risk_score >= 7:
        result = TaskResult(
            status="needs_codex",
            summary=f"Task too risky (score={decision.risk_score}): {decision.reason}",
            risk_notes=[decision.reason]
        )
        print(json.dumps({"status": result.status, "summary": result.summary}))
        return 0

    # Pack context
    packer = ContextPacker()
    verifier = Verifier(task.workspace)

    # Execute via DeepSeek
    runner = DeepSeekRunner()
    all_lines = []
    changed_files = []

    print(json.dumps({"type": "started", "id": task.id}), file=sys.stdout)
    try:
        for line in runner.run(task.instruction, task.workspace, args.deepseek_model):
            all_lines.append(line)
            print(json.dumps({"type": line.type, **line.__dict__}), file=sys.stdout)
            if line.type == "file_change":
                changed_files.append(line.path)
    except RuntimeError as e:
        log(f"DeepSeek failed: {e}")
        result = TaskResult(
            status="needs_codex",
            summary=str(e),
            risk_notes=["DeepSeek execution failed"]
        )
        print(json.dumps({"status": result.status, "summary": result.summary}))
        return 1

    # Verify
    diff, all_changed = verifier.get_git_diff()
    forbidden_hit = verifier.check_forbidden(all_changed, task.forbidden_paths)

    # Run tests
    test_passed, test_output = verifier.run_tests()

    # Fallback decision
    fallback = FallbackEngine()
    esc = fallback.should_escalate(
        risk_score=decision.risk_score,
        changed_files=all_changed,
        test_passed=test_passed,
        deepseek_failed=False
    )

    if esc.escalate:
        result = TaskResult(
            status="needs_codex",
            summary=esc.reason,
            changed_files=all_changed,
            test_result="passed" if test_passed else "failed",
            risk_notes=[esc.reason] + (["forbidden paths hit"] if forbidden_hit else [])
        )
    else:
        result = TaskResult(
            status="success",
            summary=f"Task completed. {len(all_changed)} files changed.",
            changed_files=all_changed,
            test_result="passed" if test_passed else "failed",
            patch=diff
        )

    print(json.dumps({
        "status": result.status,
        "summary": result.summary,
        "changed_files": result.changed_files,
        "test_result": result.test_result,
        "risk_notes": result.risk_notes
    }))

    return 0

if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 2: Verify imports work**

Run: `cd /Users/f/GitHub/CodexSaver && python -c "from leancodex import cli, router, packer, runner, verifier, fallback"`
Expected: No import errors

- [ ] **Step 3: Test CLI help**

Run: `python src/cli.py --help`
Expected: Usage output

- [ ] **Step 4: Commit**

```bash
git add src/cli.py
git commit -m "feat: add CLI entry point wiring all components together"
```

---

### Task 8: README Quick Start Verification

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add installation section**

Add after Quick Start:

```md
## Installation

```bash
pip install .
./bin/leancodex --help
```

## Example Usage

```bash
echo '{"id":"test_001","instruction":"add unit tests for src/utils","workspace":".","constraints":["do not modify production logic"]}' | ./bin/leancodex --verbose
```
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: add installation and example usage to README"
```

---

## Spec Coverage Check

- [x] CLI wrapper (`bin/leancodex`) — Task 1
- [x] Task classification (rule-based) — Task 2
- [x] Risk scoring — Task 2
- [x] Context packing — Task 3
- [x] DeepSeek execution (CLI one-shot) — Task 4
- [x] JSONL streaming output — Task 4
- [x] Diff verification — Task 5
- [x] Test verification — Task 5
- [x] Forbidden path check — Task 5
- [x] Fallback/escalation — Task 6
- [x] CLI entry point wiring all components — Task 7
- [x] README with quick start — Task 8

## Placeholder Scan

No TBD/TODO placeholders found. All steps show actual code. All file paths are exact.

---

**Plan complete and saved to `docs/superpowers/plans/2026-05-06-leancodex-mvp.md`.**

**Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
