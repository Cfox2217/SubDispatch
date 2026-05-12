# SubDispatch

[中文](README.md) | [English](README_en.md) | [日本語](README_ja.md) | [한국어](README_ko.md) | [Français](README_fr.md)

SubDispatch は、主 LLM が子コーディングエージェントを並列実行するためのローカルスキャフォールドです。主 LLM は計画、レビュー、マージ判断、競合解決を担当します。SubDispatch は隔離実行、状態ポーリング、成果物収集、worktree クリーンアップのみを提供します。Rust 単一バイナリとして CLI、MCP stdio、worker ディスパッチ、git worktree 管理、Claude hook 記録、ローカル Setup/Activity UI を提供します。

## なぜ五つの言語があるのか

このプロジェクトの売りは「作業を委任すること」だからです。並列エージェントディスパッチャを作っておきながら README だけ一言語に閉じ込めるのは、チームを雇ったのに案内板を一人で手書きするようなものです。中国語をデフォルトの入口にしつつ、英語、日本語、韓国語、フランス語も置いて、SubDispatch が少しだけ国際的な顔をできるようにしています。

ランタイム依存関係は意図的に最小限に抑えています：

- `git`
- 設定済みの外部 code-agent CLI、デフォルトは `claude`
- ワークスペース `.env` 内のモデル API 認証情報

Python や Node ランタイムは不要です。

## 非目標

- 自動タスク計画
- 自動レビュー
- 自動マージまたはチェリーピック
- 競合解決
- マルチプロバイダー抽象化

## コアモデル

SubDispatch は2つのエンティティを追跡します：

- `Worker`：設定済みの外部コーディングエージェントコマンド。デフォルトは `claude-code` です。
- `Task`：独立したブランチと git worktree で実行される子エージェント。

各タスクは、ベースコミット、ブランチ、worktree パス、プロセス ID、ログ、結果 manifest パス、成果物ディレクトリを記録します。

## 設定

SubDispatch は、ワークスペースルートの `.env` からプロジェクトローカル設定を読み込みます。`.env` は git-ignored です。`.env.example` がサポートするキーを文書化しています。

Rust CLI でローカルファイルを作成します：

```bash
subdispatch init-env
```

次に `.env` を直接編集します。SubDispatch はデフォルトの `claude-code` worker をサポートしています：

- `SUBDISPATCH_WORKER_MODE`
- `SUBDISPATCH_CLAUDE_ENABLED`
- `SUBDISPATCH_CLAUDE_PERMISSION_MODE`
- `SUBDISPATCH_CLAUDE_COMMAND`
- `SUBDISPATCH_CLAUDE_MODEL`
- `SUBDISPATCH_CLAUDE_MAX_CONCURRENCY`
- `ANTHROPIC_API_KEY`
- `ANTHROPIC_BASE_URL`

デフォルトの worker モードは `trusted-worktree` で、Claude Code `bypassPermissions` を使います。これは、主エージェントが実行の所有権を子エージェントに移す委任コーディングループ向けに意図した設定です。セキュリティサンドボックスではありません。SubDispatch は実行前の封じ込めではなく、隔離された git worktree、明示的なタスクスコープ、ログ、実行後の成果物レビューに依存します。

プロンプト設定は `.subdispatch/prompts.json` に別途保存されます。このファイルはオプションであり、存在しない場合は組み込みデフォルトが使用されます。Web UI の Prompts ページでは以下を編集できます：

- MCP ツールの説明
- 子エージェントプロンプトテンプレート、安全ルール、manifest schema
- worker 選択と collect/review ガイダンス

Worker メタデータは Setup/.env でのみ設定され、`description`、`strengths`、`cost`、`speed`、`delegation_trust` を単一の情報源として保持します。`delegation_trust` は主エージェントへのルーティングヒントであり、安全保証ではありません。

プロンプトの変更は新しい MCP ツールリストと新しく起動した子タスクに適用されます。既存のタスクは書き換えられません。

## インターフェース

### `list_workers`

利用可能な worker と現在の容量を返します：

- worker ID
- runner コマンド
- 設定されたモデル
- 最大同時実行数
- 実行中数
- キュー済み数
- 利用可能スロット
- 委譲信頼度
- 不可能理由（該当する場合）

MCP はこのインターフェースを `list_workers` として公開します。CLI コマンドは `subdispatch workers --workspace <path>` です。

### `start_task`

主 LLM が提供する子タスクを1つ起動します。SubDispatch はブランチと worktree を作成し、タスクプロンプトを書き出し、容量が利用可能であれば設定された worker を起動します。worker 同時実行制限を超えるタスクはキューに残ります。

委譲にはクリーンなコミット済みチェックポイントが必要です。主エージェントは独自のブランチ/worktree 戦略を所有し、`start_task` を呼び出す前に進行中の変更をコミットする必要があります。SubDispatch は隠れた統合ブランチを管理しません。ワークスペースに未コミット変更がある場合、`start_task` はエラーを返して子 worktree を作成しません。`base`/`base_branch` を省略すると、タスクは現在の `HEAD` から開始されます。

並列性は明示的です：主エージェントは `start_task` を複数回呼び出し、利用可能なスロットとタスク適合度に基づいて worker を選択し、各結果を独立してレビューします。

タスクには主エージェントが供給するオプションの `context` または `context_files` を含めることができます。これは、子エージェントに未コミット diff、一時的な監査メモ、子 worktree のベースコミットに存在しないその他のコンテキストを与える正しい方法です。

`read_scope`/`write_scope` は `forbidden_paths` と重複してはなりません。SubDispatch は、タスク worktree を作成する前に矛盾したスコープコントラクトを拒否します。管理された結果 manifest パスは、子タスクに期待される唯一の内部 `.subdispatch` 書き込みです。

### `poll_tasks`

グローバルなタスクの事実状態を返します。オプションで `task_ids`、`status`、`active_only` でフィルタリングできます。ポーリングはプロセス状態を更新し、worker スロットが開くとキュー済みタスクを起動します。

タスク状態：

- `queued`
- `running`
- `completed`
- `failed`
- `cancelled`
- `missing`

### `collect_task`

1つのタスク成果物を収集します。SubDispatch は、worker manifest を信頼するのではなく、Git から変更ファイルと差分を計算します。子エージェントはコミット必需的ではないため、未コミットの worktree 変更を含みます。

返される成果物には以下が含まれます：

- 元の指示
- worker manifest（存在する場合）
- stdout/stderr 尾部
- Claude transcript からの圧縮検証コマンド結果
- タスクスコープ hook が観察した禁止パス試行
- 変更ファイル
- 差分
- パッチパス
- ベースコミット
- タスクブランチ
- 書き込みスコープチェック
- 禁止パスチェック

manifest は worker 自己報告として扱います。Git diff、scope checks、`transcript_tool_results_tail`、`forbidden_path_attempts_tail` の方が強力なレビュー証拠です。

### `delete_worktree`

SubDispatch が管理するタスク worktree を1つ削除します。実行中のタスクを強制しない限り削除を拒否します。デフォルトではブランチと成果物ディレクトリを保持します。

## ハード制約

- 子エージェントは主 worktree で決して実行されません。
- 各タスクは独自のブランチを持ちます。
- 各タスクは独自の worktree を持ちます。
- 各タスクはベースコミットを記録します。
- `start_task` はダーティな主ワークスペースを拒否します。
- `collect_task` は Git を事実の情報源として使用します。
- Worktree 削除はターゲットが SubDispatch worktree root 下にあることを検証します。
- 成果物はデフォルトで保持されます。
- Worker 同時実行制限は強制されます。

## Rust CLI

ローカル開発時：

```bash
cargo run -- workers --workspace .
cargo run -- mcp --workspace .
cargo run -- serve --workspace . --bind 127.0.0.1:8765
```

パッケージ済みバイナリの使用は同じです（`cargo run --` なし）：

```bash
subdispatch workers --workspace .
subdispatch mcp --workspace .
subdispatch serve --workspace . --bind 127.0.0.1:8765
```

Web UI は意図的にタスクコンソールではありません。Setup チェック、`.env` 初期化、worker 容量、タスクステータス、変更ファイル数、Claude hook アクティビティを提供します。主 LLM は引き続き MCP または CLI を通じてタスクを作成します。

## インストールとリリース

グローバル MCP エントリとバンドル済み skill を一度だけインストールします：

```bash
subdispatch install-skill
subdispatch install --global
```

次に、各プロジェクトを初期化します：

```bash
cd /path/to/project
subdispatch init-env --workspace .
subdispatch doctor --workspace .
```

ローカルリリースアーカイブを作成します：

```bash
scripts/release.sh
```

パッケージ化の詳細は [docs/rust-release.md](docs/rust-release.md)、Python MVP 削除記録は [docs/python-removal-plan.md](docs/python-removal-plan.md) を参照してください。
