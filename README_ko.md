# SubDispatch

[中文](README.md) | [English](README_en.md) | [日本語](README_ja.md) | [한국어](README_ko.md) | [Français](README_fr.md)

SubDispatch는 primary LLM이 자식 코딩 에이전트를 병렬로 실행할 수 있도록 하는 로컬 스캐폴딩 도구입니다. Primary LLM은 계획, 리뷰, 병합 결정 및 충돌 해결을 담당합니다. SubDispatch는 격리된 실행, 상태 폴링, 산출물 수집 및 worktree 정리만 제공합니다.
Rust 단일 바이너리로 CLI, MCP stdio, worker 디스패치, git worktree 관리,
Claude hook 기록 및 로컬 Setup/Activity UI를 제공합니다.

## 왜 다섯 가지 언어인가

이 프로젝트의 핵심은 일을 위임하는 것이기 때문입니다. 병렬 에이전트 디스패처를 만들어 놓고 README를 한 언어로만 두는 건, 팀을 꾸려 놓고 안내문은 한 사람이 밤새 손글씨로 쓰는 것과 비슷합니다. 중국어를 기본 입구로 두고, 영어, 일본어, 한국어, 프랑스어를 함께 제공해 SubDispatch가 최소한 여권은 챙긴 척할 수 있게 했습니다.

런타임 의존성은 의도적으로 최소한으로 유지됩니다:

- `git`
- 사용자가 구성한 외부 code-agent CLI, 기본값 `claude`
- 워크스페이스 `.env`의 모델 API 자격 증명

Python이나 Node 런타임은 필요하지 않습니다.

## 비목표

- 자동 작업 planning
- 자동 리뷰
- 자동 병합 또는 체리피킹
- 충돌 해결
- 다중 provider 추상화

## 코어 모델

SubDispatch는 두 가지 엔터티를 추적합니다:

- `Worker`: 구성된 외부 코딩 에이전트 명령어. 기본값은 `claude-code`입니다.
- `Task`: 자체 브랜치 및 git worktree에서 실행되는 자식 에이전트.

각 작업은 기본 커밋, 브랜치, worktree 경로, 프로세스 ID, 로그, 결과 manifest 경로 및 산출물 디렉터리를 기록합니다.

## 구성

SubDispatch는 워크스페이스 루트의 `.env`에서 프로젝트 로컬 구성을 읽습니다. `.env`는 git에서 무시됩니다. `.env.example`은 지원되는 키를 문서화합니다.

Rust CLI로 로컬 파일 생성:

```bash
subdispatch init-env
```

그런 다음 `.env`를 직접 편집합니다. SubDispatch는 기본 `claude-code` worker를 지원합니다:

- `SUBDISPATCH_WORKER_MODE`
- `SUBDISPATCH_CLAUDE_ENABLED`
- `SUBDISPATCH_CLAUDE_PERMISSION_MODE`
- `SUBDISPATCH_CLAUDE_COMMAND`
- `SUBDISPATCH_CLAUDE_MODEL`
- `SUBDISPATCH_CLAUDE_MAX_CONCURRENCY`
- `ANTHROPIC_API_KEY`
- `ANTHROPIC_BASE_URL`

기본 worker 모드는 Claude Code `bypassPermissions`와 함께 `trusted-worktree`입니다.
이는 primary agent가 실행 소유권을 child agent에게 위임하는 위임 코딩 루프를 위한 의도적인 설정입니다.
이는 보안 샌드박스가 아닙니다. SubDispatch는 실행 전 격리가 아닌 격리된 git worktree, 명시적인 작업 범위,
로그 및 작업 후 산출물 검토에 의존합니다.

프롬프트 구성은 `.subdispatch/prompts.json`에 별도로 저장됩니다. 이 파일은 선택 사항이며,
존재하지 않을 경우 내장 기본값이 사용됩니다. Web UI의 Prompts 페이지에서 편집할 수 있습니다:

- MCP 도구 설명
- 자식 에이전트 프롬프트 템플릿, 안전 규칙 및 manifest 스키마
- worker 선택 및 collect/review 가이드

Worker 메타데이터는 Setup/.env에서만 구성되며, 이는 `description`,
`strengths`, `cost`, `speed` 및 `delegation_trust`를 단일 진실 공급자로 유지합니다.
`delegation_trust`는 primary agent를 위한 라우팅 힌트이며, 안전 보장이 아닙니다.

프롬프트 변경은 새로운 MCP 도구 목록 및 새로 시작된 자식 작업에 적용됩니다.
기존 작업은 다시 작성되지 않습니다.

## 인터페이스

### `list_workers`

사용 가능한 worker 및 현재 용량을 반환합니다:

- worker ID
- runner 명령어
- 구성된 모델
- 최대 동시성
- 실행 중 수
- 대기열 수
- 사용 가능한 슬롯
- 위임 신뢰도
- 사용할 수 없는 이유(해당하는 경우)

MCP 도구 이름은 `list_workers`이며, CLI 명령어는
`subdispatch workers --workspace <path>`입니다.

### `start_task`

primary LLM이 제공한 하나의 자식 작업을 시작합니다. SubDispatch는 해당 작업에 대한 브랜치와 worktree를 생성하고,
작업 프롬프트를 기록하며, 용량이 사용 가능할 때 구성된 worker를 시작합니다. Worker 동시성 제한을 초과한 작업은 대기 상태를 유지합니다.

위임에는 깨끗한(clean) 커밋된 체크포인트가 필요합니다. Primary agent는 자체 브랜치/worktree 전략을 담당하며,
`start_task`를 호출하기 전에 진행 중인 변경 사항을 커밋해야 합니다. SubDispatch는 숨겨진 통합 브랜치를 관리하지 않습니다.
워크스페이스에 커밋되지 않은 변경 사항이 있으면 `start_task`는 오류를 반환하고 자식 worktree를 생성하지 않습니다.
`base`/`base_branch`가 생략되면 작업은 현재 `HEAD`에서 시작됩니다.

병렬성은 명시적인 동작입니다: primary agent가 여러 번 `start_task`를 호출하고, 사용 가능한 슬롯과 작업 적합성을 기반으로 worker를 선택한 다음,
각 결과를 개별적으로 poll, collect, review하고 자체적으로 병합 방법을 결정합니다.

작업에는 primary agent가 제공하는 선택적 `context` 또는 `context_files`가 포함될 수 있습니다.
이는 자식 worktree의 기본 커밋에 존재하지 않는 커밋되지 않은 diff, 임시 감사 메모 또는 기타 정보를 자식 agent에게 전달하는 올바른 방법입니다.

`read_scope`/`write_scope`는 `forbidden_paths`와 중복될 수 없습니다. SubDispatch는
작업 worktree 생성 전에 이러한 모순적인 범위 계약을 거부합니다. 자식 작업이 작성할 것으로 예상되는 유일한 내부 `.subdispatch` 파일은 관리되는 결과 manifest 경로입니다.

### `poll_tasks`

`task_ids`, `status` 또는 `active_only`로 필터링할 수 있는 작업의 사실 상태를 반환합니다.
폴링은 프로세스 상태를 새로 고치고 worker 슬롯이 열리면 대기 중인 작업을 시작합니다.

작업 상태:

- `queued`
- `running`
- `completed`
- `failed`
- `cancelled`
- `missing`

### `collect_task`

하나의 작업 산출물을 수집합니다. SubDispatch는 worker manifest를 신뢰하지 않고
Git에서 변경된 파일과 diff를 계산합니다. 자식 에이전트가 커밋할 필요가 없으므로
커밋되지 않은 worktree 변경 사항도 포함합니다.

반환되는 산출물:

- 원본 지시사항
- worker manifest(있는 경우)
- stdout/stderr 마지막 부분
- Claude transcript에서 압축된 검증 명령 결과
- 작업 범위 hook에서 관찰한 금지된 경로 시도
- 변경된 파일
- diff
- 패치 경로
- 기본 커밋
- 작업 브랜치
- 쓰기 범위 확인
- 금지된 경로 확인

worker manifest는 자식 에이전트의 자체 보고서일 뿐입니다. Git diff, 범위 확인,
`transcript_tool_results_tail` 및 `forbidden_path_attempts_tail`이 더 강력한 검토 증거입니다.

### `delete_worktree`

SubDispatch가 관리하는 하나의 작업 worktree를 삭제합니다. 강제 실행이 아닌 경우 실행 중인 작업을 삭제 거부합니다.
기본적으로 브랜치와 산출물 디렉터리를 보존합니다.

## 하드 제약 조건

- 자식 에이전트는 primary worktree에서 절대 실행되지 않습니다.
- 모든 작업에는 자체 브랜치가 있습니다.
- 모든 작업에는 자체 worktree가 있습니다.
- 모든 작업은 기본 커밋을 기록합니다.
- `start_task`는 더러운(커밋되지 않은 변경사항이 있는) primary 워크스페이스를 거부합니다.
- `collect_task`는 진실 공급자로 Git을 사용합니다.
- Worktree 삭제시 대상이 SubDispatch worktree 루트 아래에 있는지 확인합니다.
- 산출물은 기본적으로 보존됩니다.
- Worker 동시성 제한이 적용됩니다.

## Rust CLI

로컬 개발 시:

```bash
cargo run -- workers --workspace .
cargo run -- mcp --workspace .
cargo run -- serve --workspace . --bind 127.0.0.1:8765
```

패키징된 바이너리 사용법:

```bash
subdispatch workers --workspace .
subdispatch mcp --workspace .
subdispatch serve --workspace . --bind 127.0.0.1:8765
```

Web UI는 의도적으로 작업 생성 콘솔이 아닙니다. Setup 확인, `.env` 초기화,
worker 용량, 작업 상태, 변경된 파일 수 및 Claude hook 활동만 제공합니다.
Primary LLM은 여전히 MCP 또는 CLI를 통해 작업을 생성합니다.

## 설치 및 배포

전역 MCP 엔트리와 번들된 skill을 한 번 설치합니다:

```bash
subdispatch install-skill
subdispatch install --global
```

그런 다음 각 프로젝트에서 초기화합니다:

```bash
cd /path/to/project
subdispatch init-env --workspace .
subdispatch doctor --workspace .
```

로컬 배포 아카이브 생성:

```bash
scripts/release.sh
```

패키징 세부사항은 [docs/rust-release.md](docs/rust-release.md)를,
Python MVP 제거 기록은 [docs/python-removal-plan.md](docs/python-removal-plan.md)를 참조하세요.
