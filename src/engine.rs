use crate::config::{default_workers, load_env, WorkerConfig, DEFAULT_INTEGRATION_BRANCH};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STATUS_QUEUED: &str = "queued";
const STATUS_RUNNING: &str = "running";
const STATUS_COMPLETED: &str = "completed";
const STATUS_FAILED: &str = "failed";
const STATUS_CANCELLED: &str = "cancelled";
const STATUS_MISSING: &str = "missing";

#[derive(Clone)]
pub struct SubDispatchEngine {
    workspace: PathBuf,
    runs_dir: PathBuf,
    worktrees_dir: PathBuf,
    integration_branch: String,
    workers: BTreeMap<String, WorkerConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct RunState {
    id: String,
    goal: String,
    base_ref: String,
    base_commit: String,
    workspace: String,
    created_at: f64,
    #[serde(default)]
    workspace_dirty: bool,
    #[serde(default)]
    workspace_dirty_summary: Vec<String>,
    tasks: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TaskState {
    id: String,
    run_id: String,
    goal: String,
    instruction: String,
    worker: String,
    status: String,
    branch: String,
    worktree: String,
    base_commit: String,
    #[serde(default)]
    workspace_dirty: bool,
    #[serde(default)]
    workspace_dirty_summary: Vec<String>,
    #[serde(default)]
    read_scope: Vec<String>,
    #[serde(default)]
    write_scope: Vec<String>,
    #[serde(default)]
    forbidden_paths: Vec<String>,
    #[serde(default)]
    context: String,
    #[serde(default)]
    context_files: Vec<String>,
    created_at: f64,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    started_at: Option<f64>,
    #[serde(default)]
    finished_at: Option<f64>,
    #[serde(default)]
    exit_path: Option<String>,
    #[serde(default)]
    hook_events_path: Option<String>,
    #[serde(default)]
    hook_summary_path: Option<String>,
    #[serde(default)]
    command: Option<Vec<String>>,
    #[serde(default)]
    warning: Option<String>,
    #[serde(default)]
    worktree_removed: Option<bool>,
    #[serde(default)]
    worktree_deleted_at: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LaunchSpec {
    command: Vec<String>,
    cwd: String,
    stdout_path: String,
    stderr_path: String,
    exit_path: String,
}

impl SubDispatchEngine {
    pub fn new(workspace: PathBuf) -> Result<Self, String> {
        let workspace = absolute_path(&workspace)?;
        let env = load_env(&workspace)?;
        let workers = default_workers(&env)?;
        let integration_branch = env
            .get("SUBDISPATCH_INTEGRATION_BRANCH")
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| DEFAULT_INTEGRATION_BRANCH.to_string());
        let root = workspace.join(".subdispatch");
        Ok(Self {
            runs_dir: root.join("runs"),
            worktrees_dir: root.join("worktrees").join("tasks"),
            integration_branch,
            workspace,
            workers,
        })
    }

    pub fn list_workers(&self) -> Result<Value, String> {
        let running_counts = self.running_counts_by_worker()?;
        let queued_counts = self.queued_counts_by_worker()?;
        let workers = self
            .workers
            .values()
            .map(|worker| {
                let running = *running_counts.get(&worker.id).unwrap_or(&0);
                let queued = *queued_counts.get(&worker.id).unwrap_or(&0);
                let available_slots = worker.max_concurrency.saturating_sub(running);
                let executable = command_available(&worker.command);
                let enabled = worker.enabled && executable;
                let unavailable_reason = if !worker.enabled {
                    Some("worker disabled".to_string())
                } else if !executable {
                    Some(format!(
                        "command not found: {}",
                        worker.command.first().cloned().unwrap_or_default()
                    ))
                } else if available_slots == 0 {
                    Some("concurrency limit reached".to_string())
                } else {
                    None
                };
                json!({
                    "id": worker.id,
                    "runner": worker.command.first().cloned().unwrap_or_default(),
                    "command": worker.command,
                    "model": worker.model,
                    "worker_mode": worker.worker_mode,
                    "permission_mode": worker.permission_mode,
                    "sandbox": "none",
                    "risk": if worker.permission_mode == "bypassPermissions" { "high" } else { "medium" },
                    "description": worker.description,
                    "strengths": worker.strengths,
                    "cost": worker.cost,
                    "speed": worker.speed,
                    "enabled": enabled,
                    "max_concurrency": worker.max_concurrency,
                    "running": running,
                    "queued": queued,
                    "available_slots": if enabled { available_slots } else { 0 },
                    "unavailable_reason": unavailable_reason,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "status": "ok", "workers": workers }))
    }

    pub fn init_integration(&mut self) -> Result<Value, String> {
        self.ensure_git_repo()?;
        let branch = self.integration_branch.clone();
        if !self.branch_exists(&branch) {
            self.git(&["branch", &branch, "HEAD"])?;
        }
        let path = self.integration_worktree_path();
        if !path.exists() {
            fs::create_dir_all(path.parent().unwrap_or(&self.workspace)).map_err(|err| {
                format!(
                    "failed to create integration worktree parent {}: {err}",
                    path.display()
                )
            })?;
            self.git(&["worktree", "add", &path.display().to_string(), &branch])?;
        }
        Ok(json!({
            "status": "ok",
            "branch": branch,
            "worktree": path.display().to_string(),
            "head": self.git(&["rev-parse", &branch])?.trim(),
            "dirty": !self.git_status_short_in(&path)?.is_empty(),
        }))
    }

    pub fn start_run(&mut self, input: Value) -> Result<Value, String> {
        self.ensure_git_repo()?;
        let goal = required_string(&input, "goal")?;
        let tasks = input
            .get("tasks")
            .and_then(Value::as_array)
            .ok_or_else(|| "start_run requires tasks array".to_string())?;
        if tasks.is_empty() {
            return Err("start_run requires at least one task".to_string());
        }
        let explicit_base = input
            .get("base")
            .or_else(|| input.get("base_branch"))
            .is_some();
        let base_ref = input
            .get("base")
            .or_else(|| input.get("base_branch"))
            .and_then(Value::as_str)
            .unwrap_or(&self.integration_branch)
            .to_string();
        if !explicit_base {
            self.require_clean_integration_branch(&base_ref)?;
        }
        let base_commit = self.git(&["rev-parse", &base_ref])?.trim().to_string();
        let workspace_dirty_summary = self.workspace_dirty_summary()?;
        let workspace_dirty = !workspace_dirty_summary.is_empty();
        let run_id = input
            .get("run_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(new_run_id);
        let run_dir = self.run_dir(&run_id);
        if run_dir.exists() {
            return Err(format!("run already exists: {run_id}"));
        }
        fs::create_dir_all(run_dir.join("tasks"))
            .map_err(|err| format!("failed to create run directory: {err}"))?;
        fs::create_dir_all(self.worktrees_dir.join(&run_id))
            .map_err(|err| format!("failed to create worktree directory: {err}"))?;

        let mut run = RunState {
            id: run_id.clone(),
            goal: goal.clone(),
            base_ref,
            base_commit: base_commit.clone(),
            workspace: self.workspace.display().to_string(),
            created_at: now_secs(),
            workspace_dirty,
            workspace_dirty_summary: workspace_dirty_summary.clone(),
            tasks: Vec::new(),
        };

        for raw in tasks {
            let task_id = required_string(raw, "id")?;
            let instruction = required_string(raw, "instruction")?;
            let worker_id = raw
                .get("worker")
                .and_then(Value::as_str)
                .unwrap_or("claude-code")
                .to_string();
            if !self.workers.contains_key(&worker_id) {
                return Err(format!("Unknown worker: {worker_id}"));
            }
            let branch = raw
                .get("branch")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("agent/{run_id}/{task_id}"));
            let worktree = self.worktrees_dir.join(&run_id).join(&task_id);
            self.git(&["branch", &branch, &base_commit])?;
            self.git(&["worktree", "add", &worktree.display().to_string(), &branch])?;
            let task = TaskState {
                id: task_id.clone(),
                run_id: run_id.clone(),
                goal: goal.clone(),
                instruction,
                worker: worker_id,
                status: STATUS_QUEUED.to_string(),
                branch,
                worktree: worktree.display().to_string(),
                base_commit: base_commit.clone(),
                workspace_dirty,
                workspace_dirty_summary: workspace_dirty_summary.clone(),
                read_scope: string_array(raw, "read_scope"),
                write_scope: string_array(raw, "write_scope"),
                forbidden_paths: string_array(raw, "forbidden_paths"),
                context: raw
                    .get("context")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                context_files: string_array(raw, "context_files"),
                created_at: now_secs(),
                pid: None,
                exit_code: None,
                error: None,
                started_at: None,
                finished_at: None,
                exit_path: None,
                hook_events_path: None,
                hook_summary_path: None,
                command: None,
                warning: None,
                worktree_removed: None,
                worktree_deleted_at: None,
            };
            self.write_task(&task)?;
            run.tasks.push(task_id);
        }
        self.write_run(&run)?;
        self.schedule_queued_tasks(&run_id)?;
        self.task_summaries(&run_id).map(|tasks| {
            json!({
                "status": "ok",
                "run_id": run_id,
                "base_commit": base_commit,
                "tasks": tasks
            })
        })
    }

    pub fn poll_run(&mut self, run_id: &str) -> Result<Value, String> {
        self.refresh_run(run_id)?;
        self.schedule_queued_tasks(run_id)?;
        self.refresh_run(run_id)?;
        let tasks = self.task_summaries(run_id)?;
        let status = run_status(&tasks);
        Ok(json!({ "status": status, "run_id": run_id, "tasks": tasks }))
    }

    pub fn collect_task(&mut self, run_id: &str, task_id: &str) -> Result<Value, String> {
        self.refresh_task(run_id, task_id)?;
        let task = self.read_task(run_id, task_id)?;
        let task_dir = self.task_dir(run_id, task_id);
        let worktree = PathBuf::from(&task.worktree);
        let changed_files = self.changed_files(&task)?;
        let diff = self.task_diff(&task)?;
        let patch_path = task_dir.join("diff.patch");
        fs::write(&patch_path, &diff)
            .map_err(|err| format!("failed to write {}: {err}", patch_path.display()))?;
        let manifest_path = worktree.join(".subdispatch").join("result.json");
        let manifest = read_json_optional(&manifest_path)?;
        let artifact = json!({
            "run_id": run_id,
            "task_id": task_id,
            "status": task.status,
            "instruction": task.instruction,
            "worker": task.worker,
            "base_commit": task.base_commit,
            "branch": task.branch,
            "worktree": task.worktree,
            "workspace_dirty": task.workspace_dirty,
            "workspace_dirty_summary": task.workspace_dirty_summary,
            "changed_files": changed_files,
            "diff": diff,
            "patch_path": patch_path.display().to_string(),
            "manifest": manifest,
            "stdout_tail": tail(&task_dir.join("stdout.log"), 4000)?,
            "stderr_tail": tail(&task_dir.join("stderr.log"), 4000)?,
            "hook_summary": self.hook_summary(&task)?,
            "hook_events_tail": self.hook_events_tail(&task, 20)?,
            "transcript_tail": self.transcript_tail(&task, 8000)?,
            "scope_check": scope_check(&changed_files, &task.write_scope),
            "forbidden_path_check": forbidden_path_check(&changed_files, &task.forbidden_paths),
        });
        write_json(&task_dir.join("artifact.json"), &artifact)?;
        Ok(artifact)
    }

    pub fn delete_worktree(
        &mut self,
        run_id: &str,
        task_id: &str,
        force: bool,
        delete_branch: bool,
    ) -> Result<Value, String> {
        self.refresh_task(run_id, task_id)?;
        let mut task = self.read_task(run_id, task_id)?;
        if task.status == STATUS_RUNNING && !force {
            return Err("Refusing to delete running task worktree without force=true".to_string());
        }
        let worktree = absolute_path(Path::new(&task.worktree))?;
        let managed_root = absolute_path(&self.worktrees_dir.join(run_id))?;
        if !worktree.starts_with(&managed_root) {
            return Err(format!(
                "Refusing to delete unmanaged worktree: {}",
                worktree.display()
            ));
        }
        let mut removed = false;
        if worktree.exists() {
            self.git(&[
                "worktree",
                "remove",
                "--force",
                &worktree.display().to_string(),
            ])?;
            removed = true;
        }
        let mut branch_deleted = false;
        if delete_branch {
            self.git(&["branch", "-D", &task.branch])?;
            branch_deleted = true;
        }
        task.worktree_deleted_at = Some(now_secs());
        task.worktree_removed = Some(removed);
        self.write_task(&task)?;
        Ok(json!({
            "status": "ok",
            "run_id": run_id,
            "task_id": task_id,
            "worktree_removed": removed,
            "branch_deleted": branch_deleted,
            "artifact_dir": self.task_dir(run_id, task_id).display().to_string(),
        }))
    }

    pub fn activity_snapshot(&mut self) -> Result<Value, String> {
        let mut runs = Vec::new();
        if self.runs_dir.exists() {
            for entry in fs::read_dir(&self.runs_dir)
                .map_err(|err| format!("failed to read {}: {err}", self.runs_dir.display()))?
            {
                let entry = entry.map_err(|err| format!("failed to read run entry: {err}"))?;
                if entry.path().join("run.json").exists() {
                    let run: RunState = read_json(&entry.path().join("run.json"))?;
                    let _ = self.refresh_run(&run.id);
                    let tasks = self.task_summaries(&run.id)?;
                    runs.push(json!({
                        "id": run.id,
                        "goal": run.goal,
                        "base_commit": run.base_commit,
                        "created_at": run.created_at,
                        "workspace_dirty": run.workspace_dirty,
                        "workspace_dirty_summary": run.workspace_dirty_summary,
                        "status": run_status(&tasks),
                        "tasks": tasks,
                    }));
                }
            }
        }
        Ok(json!({
            "status": "ok",
            "workspace": self.workspace.display().to_string(),
            "workers": self.list_workers()?.get("workers").cloned().unwrap_or_else(|| json!([])),
            "runs": runs,
        }))
    }

    fn schedule_queued_tasks(&mut self, run_id: &str) -> Result<(), String> {
        self.refresh_run(run_id)?;
        let mut running_counts = self.running_counts_by_worker()?;
        let run = self.read_run(run_id)?;
        for task_id in run.tasks {
            let mut task = self.read_task(run_id, &task_id)?;
            if task.status != STATUS_QUEUED {
                continue;
            }
            let worker = self
                .workers
                .get(&task.worker)
                .cloned()
                .ok_or_else(|| format!("Unknown worker: {}", task.worker))?;
            if !worker.enabled || !command_available(&worker.command) {
                task.error = Some(format!("Worker unavailable: {}", worker.id));
                self.write_task(&task)?;
                continue;
            }
            if running_counts.get(&worker.id).copied().unwrap_or(0) >= worker.max_concurrency {
                continue;
            }
            self.start_task_process(&mut task, &worker)?;
            *running_counts.entry(worker.id).or_insert(0) += 1;
        }
        Ok(())
    }

    fn start_task_process(
        &self,
        task: &mut TaskState,
        worker: &WorkerConfig,
    ) -> Result<(), String> {
        let task_dir = self.task_dir(&task.run_id, &task.id);
        fs::create_dir_all(&task_dir)
            .map_err(|err| format!("failed to create {}: {err}", task_dir.display()))?;
        let prompt_path = task_dir.join("prompt.txt");
        let result_path = PathBuf::from(&task.worktree)
            .join(".subdispatch")
            .join("result.json");
        let launch_path = task_dir.join("launch.json");
        let exit_path = task_dir.join("exit.json");
        let hook_events_path = task_dir.join("hook_events.jsonl");
        let hook_summary_path = task_dir.join("hook_summary.json");
        fs::create_dir_all(result_path.parent().expect("result has parent"))
            .map_err(|err| format!("failed to create result directory: {err}"))?;
        self.install_claude_hooks(task, &hook_events_path, &hook_summary_path)?;
        let prompt = self.render_prompt(task, &result_path)?;
        fs::write(&prompt_path, &prompt)
            .map_err(|err| format!("failed to write {}: {err}", prompt_path.display()))?;
        let command = worker
            .command
            .iter()
            .map(|part| {
                substitute_template(part, task, worker, &prompt, &prompt_path, &result_path)
            })
            .collect::<Vec<_>>();
        let launch = LaunchSpec {
            command: command.clone(),
            cwd: task.worktree.clone(),
            stdout_path: task_dir.join("stdout.log").display().to_string(),
            stderr_path: task_dir.join("stderr.log").display().to_string(),
            exit_path: exit_path.display().to_string(),
        };
        write_json(
            &launch_path,
            &serde_json::to_value(&launch).map_err(|err| err.to_string())?,
        )?;
        let exe = env::current_exe()
            .map_err(|err| format!("failed to locate current executable: {err}"))?;
        let mut command_builder = Command::new(exe);
        command_builder
            .arg("supervise")
            .arg(&launch_path)
            .current_dir(&self.workspace)
            .envs(&worker.env)
            .env("SUBDISPATCH_RUN_ID", &task.run_id)
            .env("SUBDISPATCH_TASK_ID", &task.id)
            .env("SUBDISPATCH_RESULT_PATH", &result_path)
            .env("SUBDISPATCH_PROMPT_PATH", &prompt_path)
            .env("SUBDISPATCH_WORKER_MODE", &worker.worker_mode)
            .env("SUBDISPATCH_PERMISSION_MODE", &worker.permission_mode)
            .env("SUBDISPATCH_HOOK_EVENTS_PATH", &hook_events_path)
            .env("SUBDISPATCH_HOOK_SUMMARY_PATH", &hook_summary_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach_process_group(&mut command_builder);
        let mut child = command_builder
            .spawn()
            .map_err(|err| format!("failed to start supervisor: {err}"))?;
        let pid = child.id();
        thread::spawn(move || {
            let _ = child.wait();
        });
        task.status = STATUS_RUNNING.to_string();
        task.pid = Some(pid);
        task.started_at = Some(now_secs());
        task.command = Some(command);
        task.exit_path = Some(exit_path.display().to_string());
        task.hook_events_path = Some(hook_events_path.display().to_string());
        task.hook_summary_path = Some(hook_summary_path.display().to_string());
        self.write_task(task)
    }

    fn install_claude_hooks(
        &self,
        task: &TaskState,
        hook_events_path: &Path,
        hook_summary_path: &Path,
    ) -> Result<(), String> {
        let worktree = PathBuf::from(&task.worktree);
        let claude_dir = worktree.join(".claude");
        fs::create_dir_all(claude_dir.join("hooks"))
            .map_err(|err| format!("failed to create Claude hook directory: {err}"))?;
        let settings_path = claude_dir.join("settings.local.json");
        let backup_path = self
            .task_dir(&task.run_id, &task.id)
            .join("settings.local.json.backup");
        if settings_path.exists() && !backup_path.exists() {
            fs::copy(&settings_path, &backup_path).map_err(|err| {
                format!(
                    "failed to backup {} to {}: {err}",
                    settings_path.display(),
                    backup_path.display()
                )
            })?;
        }
        let exe = env::current_exe()
            .map_err(|err| format!("failed to locate current executable: {err}"))?;
        let command = format!(
            "{} hook-record --events {} --summary {} --run-id {} --task-id {}",
            shell_quote(&exe.display().to_string()),
            shell_quote(&hook_events_path.display().to_string()),
            shell_quote(&hook_summary_path.display().to_string()),
            shell_quote(&task.run_id),
            shell_quote(&task.id),
        );
        let mut hooks = serde_json::Map::new();
        for name in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Notification",
            "Stop",
            "SubagentStop",
        ] {
            hooks.insert(
                name.to_string(),
                json!([{ "matcher": "", "hooks": [{ "type": "command", "command": command }] }]),
            );
        }
        write_json(&settings_path, &json!({ "hooks": hooks }))?;
        write_json(
            hook_summary_path,
            &json!({
                "event_count": 0,
                "hook_events_path": hook_events_path.display().to_string(),
                "hook_summary_path": hook_summary_path.display().to_string(),
                "settings_path": settings_path.display().to_string(),
                "settings_backup_path": if backup_path.exists() { Some(backup_path.display().to_string()) } else { None::<String> },
                "recorder": "subdispatch hook-record",
            }),
        )
    }

    fn refresh_run(&mut self, run_id: &str) -> Result<(), String> {
        let run = self.read_run(run_id)?;
        for task_id in run.tasks {
            self.refresh_task(run_id, &task_id)?;
        }
        Ok(())
    }

    fn refresh_task(&mut self, run_id: &str, task_id: &str) -> Result<(), String> {
        let mut task = self.read_task(run_id, task_id)?;
        if task.status != STATUS_RUNNING {
            return Ok(());
        }
        let Some(pid) = task.pid else {
            task.status = STATUS_MISSING.to_string();
            task.error = Some("running task has no pid".to_string());
            return self.write_task(&task);
        };
        if let Some(exit_code) = self.recorded_exit_code(&task)? {
            task.exit_code = Some(exit_code);
            task.finished_at = Some(now_secs());
            task.status = if exit_code == 0 {
                STATUS_COMPLETED.to_string()
            } else {
                STATUS_FAILED.to_string()
            };
            return self.write_task(&task);
        }
        if process_is_running(pid) {
            return Ok(());
        }
        task.exit_code = Some(0);
        task.finished_at = Some(now_secs());
        task.status = STATUS_COMPLETED.to_string();
        task.warning =
            Some("process disappeared before SubDispatch recorded an exit code".to_string());
        self.write_task(&task)
    }

    fn recorded_exit_code(&self, task: &TaskState) -> Result<Option<i32>, String> {
        let path = task
            .exit_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.task_dir(&task.run_id, &task.id).join("exit.json"));
        if !path.exists() {
            return Ok(None);
        }
        let value: Value = read_json(&path)?;
        Ok(value
            .get("exit_code")
            .and_then(Value::as_i64)
            .map(|value| value as i32))
    }

    fn running_counts_by_worker(&self) -> Result<BTreeMap<String, usize>, String> {
        let mut counts = BTreeMap::new();
        for task in self.all_tasks()? {
            if task.status == STATUS_RUNNING {
                *counts.entry(task.worker).or_insert(0) += 1;
            }
        }
        Ok(counts)
    }

    fn queued_counts_by_worker(&self) -> Result<BTreeMap<String, usize>, String> {
        let mut counts = BTreeMap::new();
        for task in self.all_tasks()? {
            if task.status == STATUS_QUEUED {
                *counts.entry(task.worker).or_insert(0) += 1;
            }
        }
        Ok(counts)
    }

    fn all_tasks(&self) -> Result<Vec<TaskState>, String> {
        let mut tasks = Vec::new();
        if !self.runs_dir.exists() {
            return Ok(tasks);
        }
        for run_entry in fs::read_dir(&self.runs_dir)
            .map_err(|err| format!("failed to read {}: {err}", self.runs_dir.display()))?
        {
            let run_entry = run_entry.map_err(|err| format!("failed to read run entry: {err}"))?;
            let tasks_dir = run_entry.path().join("tasks");
            if !tasks_dir.exists() {
                continue;
            }
            for task_entry in fs::read_dir(&tasks_dir)
                .map_err(|err| format!("failed to read {}: {err}", tasks_dir.display()))?
            {
                let task_entry =
                    task_entry.map_err(|err| format!("failed to read task entry: {err}"))?;
                let task_path = task_entry.path().join("task.json");
                if task_path.exists() {
                    if let Ok(task) = read_json(&task_path) {
                        tasks.push(task);
                    }
                }
            }
        }
        Ok(tasks)
    }

    fn task_summaries(&self, run_id: &str) -> Result<Vec<Value>, String> {
        let run = self.read_run(run_id)?;
        let mut summaries = Vec::new();
        for task_id in run.tasks {
            let task = self.read_task(run_id, &task_id)?;
            let hook_summary = self.hook_summary(&task)?;
            let now = now_secs();
            let runtime_seconds = task.started_at.map(|started| {
                ((task.finished_at.unwrap_or(now) - started).max(0.0)).floor() as u64
            });
            let last_event_at = hook_summary.get("last_event_at").and_then(Value::as_f64);
            let idle_seconds = if task.status == STATUS_RUNNING {
                last_event_at
                    .or(task.started_at)
                    .map(|last_activity| ((now - last_activity).max(0.0)).floor() as u64)
            } else {
                None
            };
            let worktree = PathBuf::from(&task.worktree);
            let changed_files_count = if worktree.exists() {
                self.changed_files(&task)?.len()
            } else {
                0
            };
            summaries.push(json!({
                "id": task.id,
                "status": task.status,
                "worker": task.worker,
                "pid": task.pid,
                "exit_code": task.exit_code,
                "runtime_seconds": runtime_seconds,
                "branch": task.branch,
                "worktree": task.worktree,
                "workspace_dirty": task.workspace_dirty,
                "workspace_dirty_summary": task.workspace_dirty_summary,
                "worktree_exists": worktree.exists(),
                "branch_exists": self.branch_exists(&task.branch),
                "manifest_exists": worktree.join(".subdispatch").join("result.json").exists(),
                "changed_files_count": changed_files_count,
                "last_event_at": hook_summary.get("last_event_at").cloned(),
                "idle_seconds": idle_seconds,
                "last_event_name": hook_summary.get("last_event_name").cloned(),
                "event_count": hook_summary.get("event_count").and_then(Value::as_u64).unwrap_or(0),
                "transcript_path": hook_summary.get("transcript_path").cloned(),
                "agent_transcript_path": hook_summary.get("agent_transcript_path").cloned(),
                "last_tool_name": hook_summary.get("last_tool_name").cloned(),
                "last_assistant_message_tail": hook_summary.get("last_assistant_message_tail").cloned(),
                "error": task.error,
            }));
        }
        Ok(summaries)
    }

    fn render_prompt(&self, task: &TaskState, result_path: &Path) -> Result<String, String> {
        let mut lines = vec![
            "You are a SubDispatch child coding agent working in an isolated git worktree."
                .to_string(),
            format!("Goal: {}", task.goal),
            format!("Task: {}", task.instruction),
            format!("Read scope: {:?}", task.read_scope),
            format!("Write scope: {:?}", task.write_scope),
            format!("Forbidden paths: {:?}", task.forbidden_paths),
            format!("Write a JSON result manifest to: {}", result_path.display()),
            "Do not modify any worktree outside the current directory.".to_string(),
            "Do not read or modify secrets, home directory files, or unrelated repositories."
                .to_string(),
            "Do not run destructive commands such as rm -rf, git reset --hard, or force pushes."
                .to_string(),
            "Do not merge, push, or delete branches.".to_string(),
        ];
        if task.workspace_dirty {
            lines.push(String::new());
            lines.push("Important workspace state: this task worktree was created from the recorded base commit, but the primary workspace had uncommitted changes when the run started. Those changes are not present in this worktree unless they are also included below as primary-agent supplied context. Do not assume absent files or old code mean the primary workspace lacks newer uncommitted work.".to_string());
            lines.push("Primary workspace dirty summary:".to_string());
            for item in &task.workspace_dirty_summary {
                lines.push(format!("- {item}"));
            }
        }
        let context = self.task_context(task)?;
        if !context.is_empty() {
            lines.push(String::new());
            lines.push("Primary-agent supplied context follows. Treat it as authoritative even if the worktree files differ.".to_string());
            lines.push(context);
        }
        Ok(lines.join("\n"))
    }

    fn task_context(&self, task: &TaskState) -> Result<String, String> {
        let mut chunks = Vec::new();
        if !task.context.is_empty() {
            chunks.push(format!("## Inline context\n{}", task.context));
        }
        for raw_path in &task.context_files {
            let path = absolute_path(&self.workspace.join(raw_path))?;
            if !path.starts_with(&self.workspace) {
                chunks.push(format!(
                    "## Context file skipped: {raw_path}\nPath is outside the primary workspace."
                ));
                continue;
            }
            if !path.is_file() {
                chunks.push(format!("## Context file missing: {raw_path}"));
                continue;
            }
            let text = fs::read_to_string(&path)
                .map_err(|err| format!("failed to read context file {}: {err}", path.display()))?;
            chunks.push(format!("## Context file: {raw_path}\n{text}"));
        }
        Ok(chunks.join("\n\n"))
    }

    fn changed_files(&self, task: &TaskState) -> Result<Vec<String>, String> {
        let mut paths = Vec::<String>::new();
        let committed = self.git(&[
            "diff",
            "--name-only",
            &format!("{}...{}", task.base_commit, task.branch),
        ])?;
        for line in committed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            push_unique(&mut paths, line);
        }
        let worktree = PathBuf::from(&task.worktree);
        if worktree.exists() {
            let status = self.git_in(&worktree, &["status", "--porcelain"])?;
            for line in status.lines().filter(|line| !line.trim().is_empty()) {
                let mut path = line.get(3..).unwrap_or("").to_string();
                if let Some((_, dest)) = path.split_once(" -> ") {
                    path = dest.to_string();
                }
                for file_path in expand_status_path(&worktree, &path)? {
                    if !is_internal_artifact_path(&file_path) {
                        push_unique(&mut paths, &file_path);
                    }
                }
            }
        }
        paths.sort();
        Ok(paths)
    }

    fn task_diff(&self, task: &TaskState) -> Result<String, String> {
        let mut parts = Vec::new();
        let committed = self.git(&["diff", &format!("{}...{}", task.base_commit, task.branch)])?;
        if !committed.is_empty() {
            parts.push(committed);
        }
        let worktree = PathBuf::from(&task.worktree);
        if worktree.exists() {
            let dirty = self.git_in(&worktree, &["diff", "HEAD"])?;
            if !dirty.is_empty() {
                parts.push(dirty);
            }
            let status = self.git_in(&worktree, &["status", "--porcelain"])?;
            for line in status.lines() {
                if !line.starts_with("?? ") {
                    continue;
                }
                let path = line.get(3..).unwrap_or("");
                if is_internal_artifact_path(path) {
                    continue;
                }
                for file_path in expand_status_path(&worktree, path)? {
                    if is_internal_artifact_path(&file_path) {
                        continue;
                    }
                    parts.push(self.untracked_file_diff(&worktree, &file_path)?);
                }
            }
        }
        Ok(parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn untracked_file_diff(&self, worktree: &Path, path: &str) -> Result<String, String> {
        let output = Command::new("git")
            .args(["diff", "--no-index", "--", "/dev/null", path])
            .current_dir(worktree)
            .output()
            .map_err(|err| format!("failed to run git diff --no-index: {err}"))?;
        if ![0, 1].contains(&output.status.code().unwrap_or(1)) {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn hook_summary(&self, task: &TaskState) -> Result<Value, String> {
        let path = task
            .hook_summary_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                self.task_dir(&task.run_id, &task.id)
                    .join("hook_summary.json")
            });
        read_json_optional(&path).map(|value| value.unwrap_or_else(|| json!({})))
    }

    fn hook_events_tail(&self, task: &TaskState, limit: usize) -> Result<Vec<Value>, String> {
        let path = task
            .hook_events_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                self.task_dir(&task.run_id, &task.id)
                    .join("hook_events.jsonl")
            });
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        Ok(text
            .lines()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|_| json!({ "error": "invalid hook event json", "raw": line }))
            })
            .collect())
    }

    fn transcript_tail(&self, task: &TaskState, limit: usize) -> Result<String, String> {
        let summary = self.hook_summary(task)?;
        let Some(path) = summary
            .get("agent_transcript_path")
            .or_else(|| summary.get("transcript_path"))
            .and_then(Value::as_str)
        else {
            return Ok(String::new());
        };
        tail(&expand_home(path), limit)
    }

    fn branch_exists(&self, branch: &str) -> bool {
        self.git(&["rev-parse", "--verify", branch]).is_ok()
    }

    fn ensure_git_repo(&self) -> Result<(), String> {
        self.git(&["rev-parse", "--show-toplevel"])?;
        Ok(())
    }

    fn git(&self, args: &[&str]) -> Result<String, String> {
        run_command("git", args, &self.workspace)
    }

    fn git_in(&self, cwd: &Path, args: &[&str]) -> Result<String, String> {
        run_command("git", args, cwd)
    }

    fn require_clean_integration_branch(&self, branch: &str) -> Result<(), String> {
        if !self.branch_exists(branch) {
            return Err(format!(
                "integration branch {branch:?} does not exist. Run `subdispatch init-integration --workspace {}` or pass an explicit base.",
                self.workspace.display()
            ));
        }
        let worktree = self.find_worktree_for_branch(branch)?;
        let status = self.git_status_short_in(&worktree)?;
        if !status.is_empty() {
            return Err(format!(
                "integration branch {branch:?} has uncommitted changes in {}. Commit a checkpoint before delegating, or pass an explicit base.",
                worktree.display()
            ));
        }
        Ok(())
    }

    fn find_worktree_for_branch(&self, branch: &str) -> Result<PathBuf, String> {
        let output = self.git(&["worktree", "list", "--porcelain"])?;
        let mut current_path: Option<PathBuf> = None;
        for line in output.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                current_path = Some(PathBuf::from(path));
            } else if let Some(value) = line.strip_prefix("branch refs/heads/") {
                if value == branch {
                    if let Some(path) = current_path {
                        return Ok(path);
                    }
                }
            }
        }
        Err(format!(
            "integration branch {branch:?} has no checked-out worktree. Run `subdispatch init-integration --workspace {}`.",
            self.workspace.display()
        ))
    }

    fn integration_worktree_path(&self) -> PathBuf {
        let project = self
            .workspace
            .file_name()
            .and_then(|name| name.to_str())
            .map(safe_path_component)
            .unwrap_or_else(|| "workspace".to_string());
        self.workspace
            .join(".subdispatch")
            .join("worktrees")
            .join("integration")
            .join(project)
            .join(&self.integration_branch)
    }

    fn git_status_short_in(&self, cwd: &Path) -> Result<String, String> {
        self.git_in(cwd, &["status", "--short"])
    }

    fn workspace_dirty_summary(&self) -> Result<Vec<String>, String> {
        let status = self.git(&["status", "--short"])?;
        Ok(status
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter(|line| !is_sensitive_status_line(line))
            .take(80)
            .map(ToOwned::to_owned)
            .collect())
    }

    fn run_dir(&self, run_id: &str) -> PathBuf {
        self.runs_dir.join(run_id)
    }

    fn task_dir(&self, run_id: &str, task_id: &str) -> PathBuf {
        self.run_dir(run_id).join("tasks").join(task_id)
    }

    fn read_run(&self, run_id: &str) -> Result<RunState, String> {
        read_json(&self.run_dir(run_id).join("run.json"))
    }

    fn write_run(&self, run: &RunState) -> Result<(), String> {
        write_json(&self.run_dir(&run.id).join("run.json"), run)
    }

    fn read_task(&self, run_id: &str, task_id: &str) -> Result<TaskState, String> {
        let mut task: TaskState = read_json(&self.task_dir(run_id, task_id).join("task.json"))?;
        if !task.workspace_dirty && task.workspace_dirty_summary.is_empty() {
            if let Ok(run) = self.read_run(run_id) {
                if run.workspace_dirty {
                    task.workspace_dirty = true;
                    task.workspace_dirty_summary = run.workspace_dirty_summary;
                }
            }
        }
        Ok(task)
    }

    fn write_task(&self, task: &TaskState) -> Result<(), String> {
        write_json(
            &self.task_dir(&task.run_id, &task.id).join("task.json"),
            task,
        )
    }
}

pub fn supervise(launch_json: &Path) -> Result<(), String> {
    let launch: LaunchSpec = read_json(launch_json)?;
    let started_at = now_secs();
    let mut exit_code = 127;
    let mut error: Option<String> = None;
    if launch.command.is_empty() {
        error = Some("empty command".to_string());
    } else {
        let stdout_file = fs::File::create(&launch.stdout_path)
            .map_err(|err| format!("failed to create {}: {err}", launch.stdout_path))?;
        let stderr_file = fs::File::create(&launch.stderr_path)
            .map_err(|err| format!("failed to create {}: {err}", launch.stderr_path))?;
        match Command::new(&launch.command[0])
            .args(&launch.command[1..])
            .current_dir(&launch.cwd)
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            .status()
        {
            Ok(status) => exit_code = status.code().unwrap_or(1),
            Err(err) => error = Some(err.to_string()),
        }
    }
    write_json(
        Path::new(&launch.exit_path),
        &json!({
            "exit_code": exit_code,
            "error": error,
            "started_at": started_at,
            "finished_at": now_secs(),
        }),
    )
}

pub fn record_hook_event(
    events: &Path,
    summary: &Path,
    run_id: &str,
    task_id: &str,
    input: &str,
) -> Result<(), String> {
    let payload: Value = serde_json::from_str(input.trim()).unwrap_or_else(|err| {
        json!({
            "hook_event_name": "HookParseError",
            "error": err.to_string()
        })
    });
    let recorded_at = now_secs();
    let event = json!({
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
    });
    if let Some(parent) = events.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(events)
        .map_err(|err| format!("failed to open {}: {err}", events.display()))?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&event)
            .map_err(|err| format!("failed to serialize hook event: {err}"))?
    )
    .map_err(|err| format!("failed to write {}: {err}", events.display()))?;
    let previous = read_json_optional(summary)?.unwrap_or_else(|| json!({}));
    let last_message = event.get("last_assistant_message").and_then(Value::as_str);
    let tail = last_message.map(|text| tail_chars(text, 2000));
    write_json(
        summary,
        &json!({
            "run_id": run_id,
            "task_id": task_id,
            "event_count": previous.get("event_count").and_then(Value::as_u64).unwrap_or(0) + 1,
            "last_event_at": recorded_at,
            "last_event_name": event.get("hook_event_name").cloned().unwrap_or(Value::Null),
            "last_session_id": event.get("session_id").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| previous.get("last_session_id").cloned().unwrap_or(Value::Null)),
            "transcript_path": event.get("transcript_path").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| previous.get("transcript_path").cloned().unwrap_or(Value::Null)),
            "agent_transcript_path": event.get("agent_transcript_path").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| previous.get("agent_transcript_path").cloned().unwrap_or(Value::Null)),
            "last_tool_name": event.get("tool_name").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| previous.get("last_tool_name").cloned().unwrap_or(Value::Null)),
            "last_cwd": event.get("cwd").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| previous.get("last_cwd").cloned().unwrap_or(Value::Null)),
            "last_reason": event.get("reason").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| previous.get("last_reason").cloned().unwrap_or(Value::Null)),
            "last_assistant_message_tail": tail.map(Value::String).unwrap_or_else(|| previous.get("last_assistant_message_tail").cloned().unwrap_or(Value::Null)),
        }),
    )
}

fn substitute_template(
    part: &str,
    task: &TaskState,
    worker: &WorkerConfig,
    prompt: &str,
    prompt_path: &Path,
    result_path: &Path,
) -> String {
    part.replace("$prompt_path", &prompt_path.display().to_string())
        .replace("$result_path", &result_path.display().to_string())
        .replace("$permission_mode", &worker.permission_mode)
        .replace("$worker_mode", &worker.worker_mode)
        .replace("$task_id", &task.id)
        .replace("$run_id", &task.run_id)
        .replace("$worktree", &task.worktree)
        .replace("$model", worker.model.as_deref().unwrap_or(""))
        .replace("$prompt", prompt)
}

fn scope_check(changed_files: &[String], write_scope: &[String]) -> Value {
    if write_scope.is_empty() {
        return json!({ "ok": true, "violations": [] });
    }
    let violations = changed_files
        .iter()
        .filter(|path| !write_scope.iter().any(|scope| path_in_scope(path, scope)))
        .cloned()
        .collect::<Vec<_>>();
    json!({ "ok": violations.is_empty(), "violations": violations })
}

fn forbidden_path_check(changed_files: &[String], forbidden_paths: &[String]) -> Value {
    let violations = changed_files
        .iter()
        .filter(|path| {
            forbidden_paths
                .iter()
                .any(|scope| path_in_scope(path, scope))
        })
        .cloned()
        .collect::<Vec<_>>();
    json!({ "ok": violations.is_empty(), "violations": violations })
}

fn run_status(tasks: &[Value]) -> &'static str {
    let terminal = [
        STATUS_COMPLETED,
        STATUS_FAILED,
        STATUS_CANCELLED,
        STATUS_MISSING,
    ];
    if !tasks.is_empty()
        && tasks.iter().all(|task| {
            task.get("status")
                .and_then(Value::as_str)
                .is_some_and(|value| terminal.contains(&value))
        })
    {
        "completed"
    } else {
        "running"
    }
}

fn path_in_scope(path: &str, scope: &str) -> bool {
    let scope = scope.trim_end_matches('/');
    path == scope || path.starts_with(&format!("{scope}/"))
}

fn is_internal_artifact_path(path: &str) -> bool {
    path == ".subdispatch"
        || path.starts_with(".subdispatch/")
        || path == ".claude"
        || path.starts_with(".claude/")
        || path == ".pytest_cache"
        || path.starts_with(".pytest_cache/")
        || path == "uv.lock"
}

fn is_sensitive_status_line(line: &str) -> bool {
    let path = line.get(3..).unwrap_or(line).trim();
    path == ".env"
        || path.starts_with(".env.")
        || path.contains("/.env")
        || path.contains("secret")
        || path.contains("token")
        || path.contains("key")
}

fn expand_status_path(worktree: &Path, raw_path: &str) -> Result<Vec<String>, String> {
    let normalized = raw_path.trim_end_matches('/');
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    let full_path = worktree.join(normalized);
    if full_path.is_file() {
        return Ok(vec![normalized.to_string()]);
    }
    if !full_path.is_dir() {
        return Ok(vec![normalized.to_string()]);
    }

    let mut files = Vec::new();
    let mut queue = VecDeque::from([full_path]);
    while let Some(dir) = queue.pop_front() {
        for entry in
            fs::read_dir(&dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))?
        {
            let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(worktree)
                .map_err(|err| format!("failed to relativize {}: {err}", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            if is_internal_artifact_path(&relative) {
                continue;
            }
            if path.is_dir() {
                queue.push_back(path);
            } else if path.is_file() {
                files.push(relative);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn run_command(program: &str, args: &[&str], cwd: &Path) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|err| format!("failed to run {program}: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if stderr.is_empty() { stdout } else { stderr });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn command_available(command: &[String]) -> bool {
    let Some(exe) = command.first() else {
        return false;
    };
    if exe.contains('/') {
        return Path::new(exe).exists();
    }
    env::var_os("PATH")
        .and_then(|paths| {
            env::split_paths(&paths)
                .map(|path| path.join(exe))
                .find(|path| path.exists())
        })
        .is_some()
}

fn process_is_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("invalid JSON in {}: {err}", path.display()))
}

fn read_json_optional(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    match read_json(path) {
        Ok(value) => Ok(Some(value)),
        Err(err) => Ok(Some(json!({
            "error": "invalid json",
            "message": err,
            "path": path.display().to_string()
        }))),
    }
}

fn write_json<T: Serialize + ?Sized>(path: &Path, data: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(data)
        .map_err(|err| format!("failed to serialize JSON: {err}"))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn tail(path: &Path, limit: usize) -> Result<String, String> {
    if !path.exists() || !path.is_file() {
        return Ok(String::new());
    }
    let mut text = String::new();
    fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {err}", path.display()))?
        .read_to_string(&mut text)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    Ok(tail_chars(&text, limit))
}

fn tail_chars(text: &str, limit: usize) -> String {
    let mut chars = text.chars().rev().take(limit).collect::<Vec<_>>();
    chars.reverse();
    chars.into_iter().collect()
}

fn required_string(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("missing required string field: {field}"))
}

fn string_array(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn push_unique(paths: &mut Vec<String>, path: &str) {
    if !paths.iter().any(|existing| existing == path) {
        paths.push(path.to_string());
    }
}

fn new_run_id() -> String {
    format!("run_{}_{}", now_secs() as u64, std::process::id())
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs_f64()
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir()
            .map_err(|err| format!("failed to read current directory: {err}"))?
            .join(path))
    }
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn safe_path_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_./:".contains(ch))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn detach_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_checks_prefixes() {
        let changed = vec!["src/main.rs".to_string(), "README.md".to_string()];
        let check = scope_check(&changed, &["src".to_string()]);
        assert_eq!(check["ok"], false);
        assert_eq!(check["violations"], json!(["README.md"]));
    }

    #[test]
    fn internal_artifacts_are_ignored() {
        assert!(is_internal_artifact_path(".claude/settings.local.json"));
        assert!(is_internal_artifact_path(".subdispatch/result.json"));
        assert!(!is_internal_artifact_path("src/main.rs"));
    }

    #[test]
    fn sensitive_status_lines_are_hidden_from_prompt_context() {
        assert!(is_sensitive_status_line(" M .env"));
        assert!(is_sensitive_status_line("?? config/token.txt"));
        assert!(is_sensitive_status_line(" M secrets/api_key.txt"));
        assert!(!is_sensitive_status_line(" M src/engine.rs"));
    }

    #[test]
    fn run_status_is_completed_only_when_all_tasks_are_terminal() {
        let tasks = vec![
            json!({ "status": "completed" }),
            json!({ "status": "failed" }),
            json!({ "status": "missing" }),
        ];
        assert_eq!(run_status(&tasks), "completed");

        let tasks = vec![
            json!({ "status": "completed" }),
            json!({ "status": "running" }),
        ];
        assert_eq!(run_status(&tasks), "running");
    }

    #[test]
    fn status_directories_expand_to_files() {
        let root = env::temp_dir().join(format!("subdispatch-test-{}", now_secs()));
        let report_dir = root.join("docs").join("agent-reports");
        fs::create_dir_all(&report_dir).unwrap();
        fs::write(report_dir.join("report.md"), "hello").unwrap();
        fs::create_dir_all(root.join(".subdispatch")).unwrap();
        fs::write(root.join(".subdispatch").join("result.json"), "{}").unwrap();

        let files = expand_status_path(&root, "docs/agent-reports/").unwrap();
        assert_eq!(files, vec!["docs/agent-reports/report.md".to_string()]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_task_dirty_state_can_be_recovered_from_run_state() {
        let root = env::temp_dir().join(format!("subdispatch-engine-test-{}", now_secs()));
        let engine = SubDispatchEngine {
            workspace: root.clone(),
            runs_dir: root.join(".subdispatch").join("runs"),
            worktrees_dir: root.join(".subdispatch").join("worktrees"),
            integration_branch: DEFAULT_INTEGRATION_BRANCH.to_string(),
            workers: BTreeMap::new(),
        };
        let run_id = "run_dirty";
        let task_id = "task_dirty";
        let run = RunState {
            id: run_id.to_string(),
            goal: "goal".to_string(),
            base_ref: "HEAD".to_string(),
            base_commit: "abc".to_string(),
            workspace: root.display().to_string(),
            created_at: now_secs(),
            workspace_dirty: true,
            workspace_dirty_summary: vec!["M src/engine.rs".to_string()],
            tasks: vec![task_id.to_string()],
        };
        let task = TaskState {
            id: task_id.to_string(),
            run_id: run_id.to_string(),
            goal: "goal".to_string(),
            instruction: "task".to_string(),
            worker: "minimax".to_string(),
            status: STATUS_QUEUED.to_string(),
            branch: "agent/run_dirty/task_dirty".to_string(),
            worktree: root.join("worktree").display().to_string(),
            base_commit: "abc".to_string(),
            workspace_dirty: false,
            workspace_dirty_summary: Vec::new(),
            read_scope: Vec::new(),
            write_scope: Vec::new(),
            forbidden_paths: Vec::new(),
            context: String::new(),
            context_files: Vec::new(),
            created_at: now_secs(),
            pid: None,
            exit_code: None,
            error: None,
            started_at: None,
            finished_at: None,
            exit_path: None,
            hook_events_path: None,
            hook_summary_path: None,
            command: None,
            warning: None,
            worktree_removed: None,
            worktree_deleted_at: None,
        };
        engine.write_run(&run).unwrap();
        engine.write_task(&task).unwrap();

        let recovered = engine.read_task(run_id, task_id).unwrap();
        assert!(recovered.workspace_dirty);
        assert_eq!(recovered.workspace_dirty_summary, vec!["M src/engine.rs"]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn integration_worktree_path_uses_managed_directory() {
        let root = env::temp_dir().join(format!("subdispatch-path-test-{}", now_secs()));
        let engine = SubDispatchEngine {
            workspace: root.clone(),
            runs_dir: root.join(".subdispatch").join("runs"),
            worktrees_dir: root.join(".subdispatch").join("worktrees"),
            integration_branch: "worktree_main".to_string(),
            workers: BTreeMap::new(),
        };
        assert_eq!(
            engine.integration_worktree_path(),
            root.join(".subdispatch")
                .join("worktrees")
                .join("integration")
                .join(safe_path_component(
                    root.file_name().unwrap().to_str().unwrap()
                ))
                .join("worktree_main")
        );
    }

    #[test]
    fn safe_path_component_replaces_path_unsafe_chars() {
        assert_eq!(
            safe_path_component("Sub Dispatch/alpha"),
            "Sub_Dispatch_alpha"
        );
    }

    #[test]
    fn tail_handles_utf8_boundaries() {
        assert_eq!(tail_chars("hello中文日志", 4), "中文日志");
    }
}
