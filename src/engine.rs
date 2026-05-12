use crate::config::{default_workers, load_env, WorkerConfig};
use crate::prompts::{load_prompt_config, render_child_prompt, PromptConfig};
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
    tasks_dir: PathBuf,
    worktrees_dir: PathBuf,
    workers: BTreeMap<String, WorkerConfig>,
    prompts: PromptConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TaskState {
    id: String,
    goal: String,
    instruction: String,
    worker: String,
    status: String,
    branch: String,
    worktree: String,
    base_commit: String,
    #[serde(default)]
    slot_id: Option<String>,
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
    #[serde(default)]
    collected_at: Option<f64>,
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
        let prompts = load_prompt_config(&workspace)?;
        let workers = default_workers(&env)?;
        let root = workspace.join(".subdispatch");
        Ok(Self {
            tasks_dir: root.join("tasks"),
            worktrees_dir: root.join("worktrees").join("slots"),
            workspace,
            workers,
            prompts,
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
                    "delegation_trust": worker.delegation_trust,
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

    pub fn start_task(&mut self, input: Value) -> Result<Value, String> {
        let _lock = self.acquire_state_lock()?;
        self.start_task_locked(input)
    }

    fn start_task_locked(&mut self, input: Value) -> Result<Value, String> {
        self.ensure_git_repo()?;
        self.require_clean_workspace()?;
        let instruction = required_string(&input, "instruction")?;
        let goal = input
            .get("goal")
            .and_then(Value::as_str)
            .unwrap_or(&instruction)
            .to_string();
        let base_ref = input
            .get("base")
            .or_else(|| input.get("base_branch"))
            .and_then(Value::as_str)
            .unwrap_or("HEAD")
            .to_string();
        let base_commit = self.git(&["rev-parse", &base_ref])?.trim().to_string();
        let task_id = input
            .get("task_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(new_task_id);
        let task_dir = self.task_dir(&task_id);
        if task_dir.exists() {
            return Err(format!("task already exists: {task_id}"));
        }
        let worker_id = input
            .get("worker")
            .and_then(Value::as_str)
            .unwrap_or("claude-code")
            .to_string();
        if !self.workers.contains_key(&worker_id) {
            return Err(format!("Unknown worker: {worker_id}"));
        }
        let branch = input
            .get("branch")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("agent/{task_id}"));
        let read_scope = string_array(&input, "read_scope");
        let write_scope = string_array(&input, "write_scope");
        let forbidden_paths = string_array(&input, "forbidden_paths");
        validate_scope_contract(&read_scope, &write_scope, &forbidden_paths)?;
        fs::create_dir_all(&task_dir)
            .map_err(|err| format!("failed to create task directory: {err}"))?;
        let task = TaskState {
            id: task_id.clone(),
            goal,
            instruction,
            worker: worker_id,
            status: STATUS_QUEUED.to_string(),
            branch,
            worktree: String::new(),
            base_commit: base_commit.clone(),
            slot_id: None,
            read_scope,
            write_scope,
            forbidden_paths,
            context: input
                .get("context")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            context_files: string_array(&input, "context_files"),
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
            collected_at: None,
        };
        self.write_task(&task)?;
        self.schedule_queued_tasks()?;
        let task = self.read_task(&task_id)?;
        Ok(json!({
            "status": "ok",
            "task_id": task_id,
            "base_commit": base_commit,
            "task": self.task_summary(&task)?
        }))
    }

    pub fn poll_tasks(&mut self, input: Value) -> Result<Value, String> {
        let _lock = self.acquire_state_lock()?;
        self.refresh_all_tasks()?;
        self.schedule_queued_tasks()?;
        self.refresh_all_tasks()?;
        let task_ids = string_array(&input, "task_ids");
        let status_filter = input.get("status").and_then(Value::as_str);
        let active_only = input
            .get("active_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let tasks = self
            .all_tasks()?
            .into_iter()
            .filter(|task| task_ids.is_empty() || task_ids.contains(&task.id))
            .filter(|task| status_filter.is_none() || Some(task.status.as_str()) == status_filter)
            .filter(|task| {
                !active_only || matches!(task.status.as_str(), STATUS_QUEUED | STATUS_RUNNING)
            })
            .map(|task| self.task_summary(&task))
            .collect::<Result<Vec<_>, _>>()?;
        let status = tasks_status(&tasks);
        Ok(json!({ "status": status, "tasks": tasks }))
    }

    pub fn collect_task(&mut self, task_id: &str) -> Result<Value, String> {
        let _lock = self.acquire_state_lock()?;
        self.refresh_task(task_id)?;
        let mut task = self.read_task(task_id)?;
        let task_dir = self.task_dir(task_id);
        let artifact_path = task_dir.join("artifact.json");
        if task.collected_at.is_some() && artifact_path.exists() {
            return read_json(&artifact_path);
        }
        let worktree = PathBuf::from(&task.worktree);
        let changed_files = self.changed_files(&task)?;
        let diff = self.task_diff(&task)?;
        let patch_path = task_dir.join("diff.patch");
        fs::write(&patch_path, &diff)
            .map_err(|err| format!("failed to write {}: {err}", patch_path.display()))?;
        let manifest_path = worktree.join(".subdispatch").join("result.json");
        let manifest = read_json_optional(&manifest_path)?;
        let artifact = json!({
            "task_id": task_id,
            "status": task.status.clone(),
            "instruction": task.instruction.clone(),
            "worker": task.worker.clone(),
            "base_commit": task.base_commit.clone(),
            "branch": task.branch.clone(),
            "worktree": task.worktree.clone(),
            "slot_id": task.slot_id.clone(),
            "changed_files": changed_files,
            "diff": diff,
            "patch_path": patch_path.display().to_string(),
            "manifest": manifest,
            "stdout_tail": tail(&task_dir.join("stdout.log"), 4000)?,
            "stderr_tail": tail(&task_dir.join("stderr.log"), 4000)?,
            "hook_summary": self.hook_summary(&task)?,
            "hook_events_tail": self.hook_events_tail(&task, 8)?,
            "transcript_tool_results_tail": self.transcript_tool_results_tail(&task, 4, 2000)?,
            "forbidden_path_attempts_tail": self.forbidden_path_attempts_tail(&task, 8)?,
            "transcript_tail": self.transcript_tail(&task, 2000)?,
            "scope_check": scope_check(&changed_files, &task.write_scope),
            "forbidden_path_check": forbidden_path_check(&changed_files, &task.forbidden_paths),
        });
        write_json(&artifact_path, &artifact)?;
        task.collected_at = Some(now_secs());
        self.write_task(&task)?;
        Ok(artifact)
    }

    pub fn delete_worktree(
        &mut self,
        task_id: &str,
        force: bool,
        delete_branch: bool,
    ) -> Result<Value, String> {
        let _lock = self.acquire_state_lock()?;
        self.refresh_task(task_id)?;
        let mut task = self.read_task(task_id)?;
        if task.status == STATUS_RUNNING && !force {
            return Err(
                "Refusing to delete a running slot worktree without force=true".to_string(),
            );
        }
        if task.collected_at.is_none() && !force {
            return Err("Refusing to delete a slot worktree before collect_task has captured task evidence. Run collect_task first or use force=true.".to_string());
        }
        if !force {
            if let Some(other_task_id) = self.slot_held_by_other_task(&task)? {
                return Err(format!(
                    "Refusing to delete slot worktree while task {other_task_id} still uses the same slot. Use force=true only for manual recovery."
                ));
            }
        }
        let worktree = absolute_path(Path::new(&task.worktree))?;
        let managed_root = absolute_path(&self.worktrees_dir)?;
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
            "task_id": task_id,
            "worktree_removed": removed,
            "branch_deleted": branch_deleted,
            "slot_id": task.slot_id,
            "artifact_dir": self.task_dir(task_id).display().to_string(),
        }))
    }

    pub fn activity_snapshot(&mut self) -> Result<Value, String> {
        let _lock = self.acquire_state_lock()?;
        self.refresh_all_tasks()?;
        self.schedule_queued_tasks()?;
        self.refresh_all_tasks()?;
        let tasks = self
            .all_tasks()?
            .into_iter()
            .map(|task| self.task_summary(&task))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "status": "ok",
            "workspace": self.workspace.display().to_string(),
            "workers": self.list_workers()?.get("workers").cloned().unwrap_or_else(|| json!([])),
            "tasks": tasks,
        }))
    }

    fn schedule_queued_tasks(&mut self) -> Result<(), String> {
        self.refresh_all_tasks()?;
        let mut running_counts = self.running_counts_by_worker()?;
        let mut occupied_slots = self.occupied_slots_by_worker()?;
        for mut task in self.all_tasks()? {
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
            let Some(slot_index) = self.next_free_slot(&worker, &occupied_slots) else {
                continue;
            };
            self.prepare_task_slot(&mut task, &worker, slot_index)?;
            self.start_task_process(&mut task, &worker)?;
            *running_counts.entry(worker.id.clone()).or_insert(0) += 1;
            occupied_slots
                .entry(worker.id)
                .or_default()
                .push(slot_index);
        }
        Ok(())
    }

    fn prepare_task_slot(
        &self,
        task: &mut TaskState,
        worker: &WorkerConfig,
        slot_index: usize,
    ) -> Result<(), String> {
        fs::create_dir_all(&self.worktrees_dir)
            .map_err(|err| format!("failed to create worktree directory: {err}"))?;
        let slot_id = format!("{}/slot-{}", worker.id, slot_index);
        let worktree = self.slot_worktree_path(&worker.id, slot_index);
        if self.branch_exists(&task.branch) {
            return Err(format!("task branch already exists: {}", task.branch));
        }
        if worktree.exists() {
            self.ensure_reusable_slot_worktree(&worktree)?;
            self.git_in(&worktree, &["reset", "--hard"])?;
            self.git_in(&worktree, &["clean", "-fd"])?;
        } else {
            let base_branch = format!("subdispatch/slot/{}/slot-{}", worker.id, slot_index);
            if !self.branch_exists(&base_branch) {
                self.git(&["branch", &base_branch, &task.base_commit])?;
            }
            self.git(&[
                "worktree",
                "add",
                &worktree.display().to_string(),
                &base_branch,
            ])?;
        }
        self.git_in(
            &worktree,
            &["checkout", "-B", &task.branch, &task.base_commit],
        )?;
        self.git_in(&worktree, &["reset", "--hard", &task.base_commit])?;
        self.git_in(&worktree, &["clean", "-fd"])?;
        task.slot_id = Some(slot_id);
        task.worktree = worktree.display().to_string();
        Ok(())
    }

    fn ensure_reusable_slot_worktree(&self, worktree: &Path) -> Result<(), String> {
        let managed_root = absolute_path(&self.worktrees_dir)?;
        let worktree = absolute_path(worktree)?;
        if !worktree.starts_with(&managed_root) {
            return Err(format!(
                "refusing to reuse unmanaged slot worktree: {}",
                worktree.display()
            ));
        }
        Ok(())
    }

    fn slot_worktree_path(&self, worker_id: &str, slot_index: usize) -> PathBuf {
        self.worktrees_dir
            .join(worker_id)
            .join(format!("slot-{slot_index}"))
    }

    fn next_free_slot(
        &self,
        worker: &WorkerConfig,
        occupied_slots: &BTreeMap<String, Vec<usize>>,
    ) -> Option<usize> {
        let occupied = occupied_slots.get(&worker.id);
        (0..worker.max_concurrency).find(|slot| {
            !occupied
                .map(|slots| slots.iter().any(|occupied| occupied == slot))
                .unwrap_or(false)
        })
    }

    fn start_task_process(
        &self,
        task: &mut TaskState,
        worker: &WorkerConfig,
    ) -> Result<(), String> {
        let task_dir = self.task_dir(&task.id);
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
        let backup_path = self.task_dir(&task.id).join("settings.local.json.backup");
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
            "{} hook-record --events {} --summary {} --task-id {}",
            shell_quote(&exe.display().to_string()),
            shell_quote(&hook_events_path.display().to_string()),
            shell_quote(&hook_summary_path.display().to_string()),
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

    fn refresh_all_tasks(&mut self) -> Result<(), String> {
        for task in self.all_tasks()? {
            self.refresh_task(&task.id)?;
        }
        Ok(())
    }

    fn refresh_task(&mut self, task_id: &str) -> Result<(), String> {
        let mut task = self.read_task(task_id)?;
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
            .unwrap_or_else(|| self.task_dir(&task.id).join("exit.json"));
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

    fn occupied_slots_by_worker(&self) -> Result<BTreeMap<String, Vec<usize>>, String> {
        let mut slots = BTreeMap::<String, Vec<usize>>::new();
        for task in self.all_tasks()? {
            if !task_holds_slot(&task) {
                continue;
            }
            let Some(slot_index) = task_slot_index(&task) else {
                continue;
            };
            slots.entry(task.worker).or_default().push(slot_index);
        }
        Ok(slots)
    }

    fn slot_held_by_other_task(&self, task: &TaskState) -> Result<Option<String>, String> {
        let Some(slot_id) = task.slot_id.as_deref() else {
            return Ok(None);
        };
        for other in self.all_tasks()? {
            if other.id == task.id || other.slot_id.as_deref() != Some(slot_id) {
                continue;
            }
            if task_holds_slot(&other) {
                return Ok(Some(other.id));
            }
        }
        Ok(None)
    }

    fn all_tasks(&self) -> Result<Vec<TaskState>, String> {
        let mut tasks = Vec::new();
        if !self.tasks_dir.exists() {
            return Ok(tasks);
        }
        for task_entry in fs::read_dir(&self.tasks_dir)
            .map_err(|err| format!("failed to read {}: {err}", self.tasks_dir.display()))?
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
        Ok(tasks)
    }

    fn task_summary(&self, task: &TaskState) -> Result<Value, String> {
        let hook_summary = self.hook_summary(task)?;
        let now = now_secs();
        let runtime_seconds = task
            .started_at
            .map(|started| ((task.finished_at.unwrap_or(now) - started).max(0.0)).floor() as u64);
        let last_event_at = hook_summary.get("last_event_at").and_then(Value::as_f64);
        let idle_seconds = if task.status == STATUS_RUNNING {
            last_event_at
                .or(task.started_at)
                .map(|last_activity| ((now - last_activity).max(0.0)).floor() as u64)
        } else {
            None
        };
        let worktree = PathBuf::from(&task.worktree);
        let task_dir = self.task_dir(&task.id);
        let artifact_path = task_dir.join("artifact.json");
        let patch_path = task_dir.join("diff.patch");
        let worktree_manifest_path = worktree.join(".subdispatch").join("result.json");
        let changed_files_count = if task.collected_at.is_some() {
            artifact_changed_files_count(&artifact_path).unwrap_or(0)
        } else if worktree.exists() {
            self.changed_files(task)?.len()
        } else {
            artifact_changed_files_count(&artifact_path).unwrap_or(0)
        };
        let manifest_exists =
            worktree_manifest_path.exists() || artifact_manifest_exists(&artifact_path);
        Ok(json!({
            "id": task.id,
            "task_id": task.id,
            "goal": task.goal,
            "instruction": task.instruction,
            "status": task.status,
            "worker": task.worker,
            "created_at": task.created_at,
            "pid": task.pid,
            "exit_code": task.exit_code,
            "runtime_seconds": runtime_seconds,
            "branch": task.branch,
            "worktree": task.worktree,
            "slot_id": task.slot_id,
            "collected_at": task.collected_at,
            "worktree_exists": worktree.exists(),
            "branch_exists": self.branch_exists(&task.branch),
            "manifest_exists": manifest_exists,
            "artifact_exists": artifact_path.exists(),
            "patch_exists": patch_path.exists(),
            "changed_files_count": changed_files_count,
            "last_event_at": hook_summary.get("last_event_at").cloned(),
            "idle_seconds": idle_seconds,
            "last_event_name": hook_summary.get("last_event_name").cloned(),
            "event_count": hook_summary.get("event_count").and_then(Value::as_u64).unwrap_or(0),
            "recent_events": self.hook_events_tail(task, 8)?,
            "transcript_path": hook_summary.get("transcript_path").cloned(),
            "agent_transcript_path": hook_summary.get("agent_transcript_path").cloned(),
            "last_tool_name": hook_summary.get("last_tool_name").cloned(),
            "last_file_path": hook_summary.get("last_file_path").cloned(),
            "last_assistant_message_tail": hook_summary.get("last_assistant_message_tail").cloned(),
            "error": task.error,
        }))
    }

    fn render_prompt(&self, task: &TaskState, result_path: &Path) -> Result<String, String> {
        let context = self.task_context(task)?;
        Ok(render_child_prompt(
            &self.prompts,
            &task.goal,
            &task.instruction,
            &task.read_scope,
            &task.write_scope,
            &task.forbidden_paths,
            result_path,
            &context,
        ))
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
            .unwrap_or_else(|| self.task_dir(&task.id).join("hook_summary.json"));
        read_json_optional(&path).map(|value| value.unwrap_or_else(|| json!({})))
    }

    fn hook_events_tail(&self, task: &TaskState, limit: usize) -> Result<Vec<Value>, String> {
        let path = task
            .hook_events_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.task_dir(&task.id).join("hook_events.jsonl"));
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
            .map(compact_hook_event_line)
            .collect())
    }

    fn transcript_tail(&self, task: &TaskState, limit: usize) -> Result<String, String> {
        let Some(path) = self.transcript_path(task)? else {
            return Ok(String::new());
        };
        tail(&expand_home(&path), limit)
    }

    fn transcript_tool_results_tail(
        &self,
        task: &TaskState,
        count: usize,
        chars_per_result: usize,
    ) -> Result<Vec<Value>, String> {
        let Some(path) = self.transcript_path(task)? else {
            return Ok(Vec::new());
        };
        let path = expand_home(&path);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let tool_uses = transcript_tool_uses(&text);
        let results = text
            .lines()
            .filter_map(|line| compact_transcript_tool_result_line(line, &tool_uses))
            .rev()
            .collect::<Vec<_>>();
        let mut selected = results
            .iter()
            .filter(|value| is_verification_tool_result(value))
            .take(count)
            .cloned()
            .collect::<Vec<_>>();
        if selected.is_empty() {
            selected = results.into_iter().take(count).collect();
        }
        Ok(selected
            .into_iter()
            .rev()
            .map(|mut value| {
                trim_tool_result_content(&mut value, chars_per_result);
                value
            })
            .collect())
    }

    fn forbidden_path_attempts_tail(
        &self,
        task: &TaskState,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        if task.forbidden_paths.is_empty() {
            return Ok(Vec::new());
        }
        let path = task
            .hook_events_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.task_dir(&task.id).join("hook_events.jsonl"));
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        Ok(text
            .lines()
            .filter_map(|line| compact_forbidden_attempt_line(line, &task.forbidden_paths))
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect())
    }

    fn transcript_path(&self, task: &TaskState) -> Result<Option<String>, String> {
        let summary = self.hook_summary(task)?;
        Ok(summary
            .get("agent_transcript_path")
            .and_then(Value::as_str)
            .or_else(|| summary.get("transcript_path").and_then(Value::as_str))
            .map(ToOwned::to_owned))
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

    fn require_clean_workspace(&self) -> Result<(), String> {
        let status = self.git_status_short_in(&self.workspace)?;
        if !status.is_empty() {
            return Err("workspace has uncommitted changes. Commit a checkpoint before delegating so child worktrees start from a real HEAD.".to_string());
        }
        Ok(())
    }

    fn git_status_short_in(&self, cwd: &Path) -> Result<String, String> {
        self.git_in(cwd, &["status", "--short"])
    }

    fn task_dir(&self, task_id: &str) -> PathBuf {
        self.tasks_dir.join(task_id)
    }

    fn read_task(&self, task_id: &str) -> Result<TaskState, String> {
        read_json(&self.task_dir(task_id).join("task.json"))
    }

    fn write_task(&self, task: &TaskState) -> Result<(), String> {
        write_json(&self.task_dir(&task.id).join("task.json"), task)
    }

    fn acquire_state_lock(&self) -> Result<StateLock, String> {
        StateLock::acquire(&self.workspace.join(".subdispatch").join("state.lock"))
    }
}

struct StateLock {
    path: PathBuf,
}

impl StateLock {
    fn acquire(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        let timeout = Duration::from_secs(30);
        let started = SystemTime::now();
        loop {
            match fs::create_dir(path) {
                Ok(()) => {
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let waited = started.elapsed().unwrap_or_default();
                    if waited >= timeout {
                        return Err(format!(
                            "timed out waiting for SubDispatch state lock: {}",
                            path.display()
                        ));
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(err) => {
                    return Err(format!(
                        "failed to acquire SubDispatch state lock {}: {err}",
                        path.display()
                    ));
                }
            }
        }
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
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
    let tool_input = event
        .pointer("/raw/tool_input")
        .or_else(|| event.get("tool_input"));
    let file_path = compact_event_file_path(&event, tool_input);
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
            "task_id": task_id,
            "event_count": previous.get("event_count").and_then(Value::as_u64).unwrap_or(0) + 1,
            "last_event_at": recorded_at,
            "last_event_name": event.get("hook_event_name").cloned().unwrap_or(Value::Null),
            "last_session_id": event.get("session_id").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| previous.get("last_session_id").cloned().unwrap_or(Value::Null)),
            "transcript_path": event.get("transcript_path").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| previous.get("transcript_path").cloned().unwrap_or(Value::Null)),
            "agent_transcript_path": event.get("agent_transcript_path").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| previous.get("agent_transcript_path").cloned().unwrap_or(Value::Null)),
            "last_tool_name": event.get("tool_name").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| previous.get("last_tool_name").cloned().unwrap_or(Value::Null)),
            "last_file_path": if file_path.is_null() { previous.get("last_file_path").cloned().unwrap_or(Value::Null) } else { file_path },
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

fn validate_scope_contract(
    read_scope: &[String],
    write_scope: &[String],
    forbidden_paths: &[String],
) -> Result<(), String> {
    let conflicts = read_scope
        .iter()
        .chain(write_scope.iter())
        .filter(|scope| {
            !is_result_manifest_path(scope)
                && forbidden_paths
                    .iter()
                    .any(|forbidden| scopes_overlap(scope, forbidden))
        })
        .cloned()
        .collect::<Vec<_>>();
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "scope contract conflict: paths cannot be both allowed and forbidden: {}",
            conflicts.join(", ")
        ))
    }
}

fn scopes_overlap(left: &str, right: &str) -> bool {
    let left = left.trim_end_matches('/');
    let right = right.trim_end_matches('/');
    left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
}

fn tasks_status(tasks: &[Value]) -> &'static str {
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

fn task_holds_slot(task: &TaskState) -> bool {
    task.slot_id.is_some()
        && !task.worktree_removed.unwrap_or(false)
        && match task.status.as_str() {
            STATUS_QUEUED | STATUS_RUNNING => true,
            STATUS_COMPLETED | STATUS_FAILED | STATUS_CANCELLED | STATUS_MISSING => {
                task.collected_at.is_none()
            }
            _ => true,
        }
}

fn task_slot_index(task: &TaskState) -> Option<usize> {
    task.slot_id
        .as_deref()
        .and_then(|slot| slot.rsplit_once('/'))
        .and_then(|(_, slot)| slot.strip_prefix("slot-"))
        .and_then(|value| value.parse::<usize>().ok())
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

fn compact_hook_event_line(line: &str) -> Value {
    let Ok(event) = serde_json::from_str::<Value>(line) else {
        return json!({ "error": "invalid hook event json" });
    };
    let tool_input = event
        .pointer("/raw/tool_input")
        .or_else(|| event.get("tool_input"));
    let tool_response = event
        .pointer("/raw/tool_response")
        .or_else(|| event.get("tool_response"));
    json!({
        "recorded_at": event.get("recorded_at").cloned().unwrap_or(Value::Null),
        "hook_event_name": event.get("hook_event_name").cloned().unwrap_or(Value::Null),
        "tool_name": event.get("tool_name").cloned().unwrap_or(Value::Null),
        "file_path": compact_event_file_path(&event, tool_input),
        "command": tool_input.and_then(|value| value.get("command")).and_then(Value::as_str),
        "duration_ms": event.pointer("/raw/duration_ms").and_then(Value::as_u64),
        "stdout_tail": tool_response
            .and_then(|value| value.get("stdout"))
            .and_then(Value::as_str)
            .map(|text| tail_chars(text, 800)),
        "stderr_tail": tool_response
            .and_then(|value| value.get("stderr"))
            .and_then(Value::as_str)
            .map(|text| tail_chars(text, 800)),
        "last_assistant_message_tail": event
            .get("last_assistant_message")
            .and_then(Value::as_str)
            .map(|text| tail_chars(text, 800)),
    })
}

fn compact_event_file_path(event: &Value, tool_input: Option<&Value>) -> Value {
    event
        .get("file_path")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            tool_input
                .and_then(|value| value.get("file_path"))
                .filter(|value| !value.is_null())
                .cloned()
        })
        .unwrap_or(Value::Null)
}

#[derive(Debug, Clone, Default)]
struct TranscriptToolUse {
    name: String,
    command: Option<String>,
}

fn transcript_tool_uses(text: &str) -> BTreeMap<String, TranscriptToolUse> {
    let mut tool_uses = BTreeMap::new();
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(items) = event.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            if item.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let Some(id) = item.get("id").and_then(Value::as_str) else {
                continue;
            };
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool_use")
                .to_string();
            let command = item
                .pointer("/input/command")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            tool_uses.insert(id.to_string(), TranscriptToolUse { name, command });
        }
    }
    tool_uses
}

fn compact_transcript_tool_result_line(
    line: &str,
    tool_uses: &BTreeMap<String, TranscriptToolUse>,
) -> Option<Value> {
    let event = serde_json::from_str::<Value>(line).ok()?;
    if let Some(items) = event.pointer("/message/content").and_then(Value::as_array) {
        for item in items {
            if item.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let tool_use_id = item.get("tool_use_id").and_then(Value::as_str);
            let tool_use = tool_use_id.and_then(|id| tool_uses.get(id));
            return Some(json!({
                "timestamp": event.get("timestamp").cloned().unwrap_or(Value::Null),
                "tool_name": tool_use.map(|value| value.name.clone()),
                "command": tool_use.and_then(|value| value.command.clone()),
                "tool_use_id": item.get("tool_use_id").cloned().unwrap_or(Value::Null),
                "is_error": item.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                "content_tail": transcript_content_to_text(item.get("content")),
                "source": "message.content",
            }));
        }
    }
    event.get("toolUseResult").map(|result| {
        json!({
            "timestamp": event.get("timestamp").cloned().unwrap_or(Value::Null),
            "tool_use_id": Value::Null,
            "is_error": false,
            "content_tail": transcript_content_to_text(Some(result)),
            "source": "toolUseResult",
        })
    })
}

fn transcript_content_to_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| item.to_string())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn is_verification_tool_result(value: &Value) -> bool {
    if value.get("tool_name").and_then(Value::as_str) != Some("Bash") {
        return false;
    }
    let command = value
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if !is_verification_command(&command) {
        return false;
    }
    if value
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    let text = value
        .get("content_tail")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    text.contains("test result:") || text.contains("exit code")
}

fn is_verification_command(command: &str) -> bool {
    command.contains("test")
        || command.contains("cargo check")
        || command.contains("cargo build")
        || command.contains("clippy")
        || command.contains("fmt")
        || command.contains("lint")
}

fn trim_tool_result_content(value: &mut Value, limit: usize) {
    if let Some(content) = value.get_mut("content_tail") {
        if let Some(text) = content.as_str() {
            *content = Value::String(tail_chars(text, limit));
        }
    }
}

fn compact_forbidden_attempt_line(line: &str, forbidden_paths: &[String]) -> Option<Value> {
    let event = serde_json::from_str::<Value>(line).ok()?;
    if event.get("hook_event_name").and_then(Value::as_str) != Some("PreToolUse") {
        return None;
    }
    let tool_name = event.get("tool_name").and_then(Value::as_str)?;
    let tool_input = event
        .pointer("/raw/tool_input")
        .or_else(|| event.get("tool_input"));
    let path = tool_input
        .and_then(|input| input.get("file_path"))
        .and_then(Value::as_str)
        .map(relative_event_path)?;
    if is_result_manifest_path(&path) {
        return None;
    }
    let matched = forbidden_paths
        .iter()
        .find(|scope| path_in_scope(&path, scope))
        .cloned()?;
    Some(json!({
        "recorded_at": event.get("recorded_at").cloned().unwrap_or(Value::Null),
        "tool_name": tool_name,
        "file_path": path,
        "forbidden_path": matched,
    }))
}

fn relative_event_path(path: &str) -> String {
    if let Some(relative) =
        path.split_once("/.subdispatch/worktrees/slots/")
            .and_then(|(_, rest)| {
                let mut parts = rest.splitn(3, '/');
                let _worker = parts.next()?;
                let _slot = parts.next()?;
                parts.next().map(ToOwned::to_owned)
            })
    {
        return relative;
    }
    path.split_once("/.subdispatch/worktrees/tasks/")
        .and_then(|(_, rest)| {
            rest.split_once('/')
                .map(|(_, relative)| relative.to_string())
        })
        .unwrap_or_else(|| path.to_string())
}

fn is_result_manifest_path(path: &str) -> bool {
    path == ".subdispatch/result.json"
}

fn artifact_changed_files_count(path: &Path) -> Option<usize> {
    read_json_optional(path)
        .ok()
        .flatten()
        .and_then(|artifact| artifact.get("changed_files")?.as_array().map(Vec::len))
}

fn artifact_manifest_exists(path: &Path) -> bool {
    read_json_optional(path)
        .ok()
        .flatten()
        .and_then(|artifact| artifact.get("manifest").cloned())
        .is_some_and(|manifest| !manifest.is_null())
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
    atomic_write(path, format!("{text}\n").as_bytes())
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("path has no file name: {}", path.display()))?;
    let tmp_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        unique_suffix()
    ));
    {
        let mut file = fs::File::create(&tmp_path)
            .map_err(|err| format!("failed to create {}: {err}", tmp_path.display()))?;
        file.write_all(contents)
            .map_err(|err| format!("failed to write {}: {err}", tmp_path.display()))?;
        file.sync_all()
            .map_err(|err| format!("failed to sync {}: {err}", tmp_path.display()))?;
    }
    fs::rename(&tmp_path, path).map_err(|err| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "failed to replace {} with {}: {err}",
            path.display(),
            tmp_path.display()
        )
    })
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
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

fn new_task_id() -> String {
    format!("task_{}_{}", now_secs() as u64, std::process::id())
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

    fn wait_for_task_status(engine: &mut SubDispatchEngine, task_id: &str, status: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            engine.poll_tasks(json!({})).unwrap();
            let task = engine.read_task(task_id).unwrap();
            if task.status == status {
                return;
            }
            if std::time::Instant::now() >= deadline {
                panic!("task {task_id} did not reach {status}; got {}", task.status);
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn scope_checks_prefixes() {
        let changed = vec!["src/main.rs".to_string(), "README.md".to_string()];
        let check = scope_check(&changed, &["src".to_string()]);
        assert_eq!(check["ok"], false);
        assert_eq!(check["violations"], json!(["README.md"]));
    }

    #[test]
    fn scope_contract_rejects_allowed_forbidden_overlap() {
        let err = validate_scope_contract(&["src/task.rs".to_string()], &[], &["src".to_string()])
            .unwrap_err();
        assert!(err.contains("src/task.rs"));

        let err = validate_scope_contract(
            &[],
            &["Cargo.toml".to_string()],
            &["Cargo.toml".to_string()],
        )
        .unwrap_err();
        assert!(err.contains("Cargo.toml"));
    }

    #[test]
    fn scope_contract_allows_result_manifest_exception() {
        validate_scope_contract(
            &[],
            &[".subdispatch/result.json".to_string()],
            &[".subdispatch".to_string()],
        )
        .unwrap();
    }

    #[test]
    fn internal_artifacts_are_ignored() {
        assert!(is_internal_artifact_path(".claude/settings.local.json"));
        assert!(is_internal_artifact_path(".subdispatch/result.json"));
        assert!(!is_internal_artifact_path("src/main.rs"));
    }

    #[test]
    fn tasks_status_is_completed_only_when_all_tasks_are_terminal() {
        let tasks = vec![
            json!({ "status": "completed" }),
            json!({ "status": "failed" }),
            json!({ "status": "missing" }),
        ];
        assert_eq!(tasks_status(&tasks), "completed");

        let tasks = vec![
            json!({ "status": "completed" }),
            json!({ "status": "running" }),
        ];
        assert_eq!(tasks_status(&tasks), "running");
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
    fn state_lock_serializes_callers() {
        let root = env::temp_dir().join(format!("subdispatch-lock-test-{}", now_secs()));
        fs::create_dir_all(&root).unwrap();
        let lock_path = root.join(".subdispatch").join("state.lock");
        let first_lock = StateLock::acquire(&lock_path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let second_lock_path = lock_path.clone();
        let handle = thread::spawn(move || {
            let started = std::time::Instant::now();
            let _second_lock = StateLock::acquire(&second_lock_path).unwrap();
            tx.send(started.elapsed()).unwrap();
        });

        thread::sleep(Duration::from_millis(100));
        assert!(rx.try_recv().is_err());
        drop(first_lock);

        let waited = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(waited >= Duration::from_millis(75));
        handle.join().unwrap();
        assert!(!lock_path.exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn write_json_replaces_existing_file_atomically() {
        let root = env::temp_dir().join(format!("subdispatch-atomic-json-test-{}", now_secs()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("task.json");
        fs::write(&path, "previous").unwrap();

        write_json(&path, &json!({ "status": "ok" })).unwrap();

        let value: Value = read_json(&path).unwrap();
        assert_eq!(value["status"], "ok");
        let leftovers = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect::<Vec<_>>();
        assert!(leftovers.is_empty(), "{leftovers:?}");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_start_task_serializes_state_writes() {
        let root = env::temp_dir().join(format!("subdispatch-concurrent-test-{}", now_secs()));
        fs::create_dir_all(&root).unwrap();
        run_command("git", &["init"], &root).unwrap();
        run_command("git", &["config", "user.email", "test@example.com"], &root).unwrap();
        run_command("git", &["config", "user.name", "SubDispatch Test"], &root).unwrap();
        fs::write(root.join("README.md"), "initial\n").unwrap();
        fs::write(root.join(".gitignore"), ".subdispatch/\n").unwrap();
        run_command("git", &["add", "README.md", ".gitignore"], &root).unwrap();
        run_command("git", &["commit", "-m", "initial"], &root).unwrap();

        let mut handles = Vec::new();
        for index in 0..4 {
            let workspace = root.clone();
            handles.push(thread::spawn(move || {
                let mut workers = BTreeMap::new();
                workers.insert(
                    "worker".to_string(),
                    WorkerConfig {
                        id: "worker".to_string(),
                        command: vec!["sh".to_string(), "-c".to_string(), "true".to_string()],
                        max_concurrency: 1,
                        model: None,
                        enabled: false,
                        env: BTreeMap::new(),
                        worker_mode: "trusted-worktree".to_string(),
                        permission_mode: "bypassPermissions".to_string(),
                        description: "test".to_string(),
                        strengths: Vec::new(),
                        cost: "test".to_string(),
                        speed: "test".to_string(),
                        delegation_trust: "medium".to_string(),
                    },
                );
                let mut engine = SubDispatchEngine {
                    workspace: workspace.clone(),
                    tasks_dir: workspace.join(".subdispatch").join("tasks"),
                    worktrees_dir: workspace
                        .join(".subdispatch")
                        .join("worktrees")
                        .join("slots"),
                    workers,
                    prompts: PromptConfig::default(),
                };
                engine
                    .start_task(json!({
                        "task_id": format!("task_{index}"),
                        "worker": "worker",
                        "instruction": "do nothing"
                    }))
                    .unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let engine = SubDispatchEngine {
            workspace: root.clone(),
            tasks_dir: root.join(".subdispatch").join("tasks"),
            worktrees_dir: root.join(".subdispatch").join("worktrees").join("slots"),
            workers: BTreeMap::new(),
            prompts: PromptConfig::default(),
        };
        let tasks = engine.all_tasks().unwrap();
        assert_eq!(tasks.len(), 4);
        for task in tasks {
            assert!(matches!(
                task.status.as_str(),
                STATUS_QUEUED | STATUS_FAILED
            ));
        }
        assert!(!root.join(".subdispatch").join("state.lock").exists());
        let tmp_files = fs::read_dir(root.join(".subdispatch").join("tasks"))
            .unwrap()
            .flat_map(|entry| fs::read_dir(entry.unwrap().path()).unwrap())
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect::<Vec<_>>();
        assert!(tmp_files.is_empty(), "{tmp_files:?}");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn slot_is_reused_only_after_collect() {
        let root = env::temp_dir().join(format!("subdispatch-slot-reuse-test-{}", now_secs()));
        fs::create_dir_all(&root).unwrap();
        run_command("git", &["init"], &root).unwrap();
        run_command("git", &["config", "user.email", "test@example.com"], &root).unwrap();
        run_command("git", &["config", "user.name", "SubDispatch Test"], &root).unwrap();
        fs::write(root.join("README.md"), "initial\n").unwrap();
        fs::write(root.join(".gitignore"), ".subdispatch/\n").unwrap();
        run_command("git", &["add", "README.md", ".gitignore"], &root).unwrap();
        run_command("git", &["commit", "-m", "initial"], &root).unwrap();

        let mut workers = BTreeMap::new();
        workers.insert(
            "worker".to_string(),
            WorkerConfig {
                id: "worker".to_string(),
                command: vec!["sh".to_string(), "-c".to_string(), "true".to_string()],
                max_concurrency: 1,
                model: None,
                enabled: true,
                env: BTreeMap::new(),
                worker_mode: "trusted-worktree".to_string(),
                permission_mode: "bypassPermissions".to_string(),
                description: "test".to_string(),
                strengths: Vec::new(),
                cost: "test".to_string(),
                speed: "test".to_string(),
                delegation_trust: "medium".to_string(),
            },
        );
        let mut engine = SubDispatchEngine {
            workspace: root.clone(),
            tasks_dir: root.join(".subdispatch").join("tasks"),
            worktrees_dir: root.join(".subdispatch").join("worktrees").join("slots"),
            workers,
            prompts: PromptConfig::default(),
        };

        engine
            .start_task(json!({
                "task_id": "first",
                "worker": "worker",
                "instruction": "do nothing"
            }))
            .unwrap();
        engine
            .start_task(json!({
                "task_id": "second",
                "worker": "worker",
                "instruction": "do nothing"
            }))
            .unwrap();

        let first = engine.read_task("first").unwrap();
        assert_eq!(first.slot_id.as_deref(), Some("worker/slot-0"));
        let slot_path = first.worktree.clone();
        wait_for_task_status(&mut engine, "first", STATUS_COMPLETED);
        engine.poll_tasks(json!({})).unwrap();
        assert_eq!(engine.read_task("second").unwrap().status, STATUS_QUEUED);

        engine.collect_task("first").unwrap();
        engine.poll_tasks(json!({})).unwrap();
        let second = engine.read_task("second").unwrap();
        assert_eq!(second.slot_id.as_deref(), Some("worker/slot-0"));
        assert_eq!(second.worktree, slot_path);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delete_worktree_refuses_slot_used_by_new_task() {
        let root = env::temp_dir().join(format!("subdispatch-slot-delete-test-{}", now_secs()));
        fs::create_dir_all(&root).unwrap();
        run_command("git", &["init"], &root).unwrap();
        run_command("git", &["config", "user.email", "test@example.com"], &root).unwrap();
        run_command("git", &["config", "user.name", "SubDispatch Test"], &root).unwrap();
        fs::write(root.join("README.md"), "initial\n").unwrap();
        fs::write(root.join(".gitignore"), ".subdispatch/\n").unwrap();
        run_command("git", &["add", "README.md", ".gitignore"], &root).unwrap();
        run_command("git", &["commit", "-m", "initial"], &root).unwrap();

        let mut workers = BTreeMap::new();
        workers.insert(
            "worker".to_string(),
            WorkerConfig {
                id: "worker".to_string(),
                command: vec!["sh".to_string(), "-c".to_string(), "true".to_string()],
                max_concurrency: 1,
                model: None,
                enabled: true,
                env: BTreeMap::new(),
                worker_mode: "trusted-worktree".to_string(),
                permission_mode: "bypassPermissions".to_string(),
                description: "test".to_string(),
                strengths: Vec::new(),
                cost: "test".to_string(),
                speed: "test".to_string(),
                delegation_trust: "medium".to_string(),
            },
        );
        let mut engine = SubDispatchEngine {
            workspace: root.clone(),
            tasks_dir: root.join(".subdispatch").join("tasks"),
            worktrees_dir: root.join(".subdispatch").join("worktrees").join("slots"),
            workers,
            prompts: PromptConfig::default(),
        };

        engine
            .start_task(json!({
                "task_id": "first",
                "worker": "worker",
                "instruction": "do nothing"
            }))
            .unwrap();
        wait_for_task_status(&mut engine, "first", STATUS_COMPLETED);
        engine.collect_task("first").unwrap();
        engine
            .start_task(json!({
                "task_id": "second",
                "worker": "worker",
                "instruction": "do nothing"
            }))
            .unwrap();

        let err = engine.delete_worktree("first", false, false).unwrap_err();
        assert!(err.contains("same slot"));
        assert!(PathBuf::from(engine.read_task("second").unwrap().worktree).exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completed_task_holds_slot_until_collected() {
        let mut first = test_task(Path::new("/tmp/subdispatch-slot-test"), None);
        first.worker = "worker".to_string();
        first.slot_id = Some("worker/slot-0".to_string());
        first.status = STATUS_COMPLETED.to_string();
        first.collected_at = None;
        assert!(task_holds_slot(&first));
        assert_eq!(task_slot_index(&first), Some(0));

        first.collected_at = Some(now_secs());
        assert!(!task_holds_slot(&first));

        let mut running = first.clone();
        running.status = STATUS_RUNNING.to_string();
        assert!(task_holds_slot(&running));
    }

    #[test]
    fn start_task_rejects_dirty_workspace() {
        let root = env::temp_dir().join(format!("subdispatch-dirty-test-{}", now_secs()));
        fs::create_dir_all(&root).unwrap();
        run_command("git", &["init"], &root).unwrap();
        run_command("git", &["config", "user.email", "test@example.com"], &root).unwrap();
        run_command("git", &["config", "user.name", "SubDispatch Test"], &root).unwrap();
        fs::write(root.join("README.md"), "initial\n").unwrap();
        run_command("git", &["add", "README.md"], &root).unwrap();
        run_command("git", &["commit", "-m", "initial"], &root).unwrap();
        fs::write(root.join("README.md"), "dirty\n").unwrap();

        let mut engine = SubDispatchEngine {
            workspace: root.clone(),
            tasks_dir: root.join(".subdispatch").join("tasks"),
            worktrees_dir: root.join(".subdispatch").join("worktrees").join("slots"),
            workers: BTreeMap::new(),
            prompts: PromptConfig::default(),
        };
        let err = engine
            .start_task(json!({
                "instruction": "do nothing"
            }))
            .unwrap_err();
        assert!(err.contains("workspace has uncommitted changes"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn start_task_rejects_scope_contract_conflict() {
        let root = env::temp_dir().join(format!("subdispatch-scope-test-{}", now_secs()));
        fs::create_dir_all(&root).unwrap();
        run_command("git", &["init"], &root).unwrap();
        run_command("git", &["config", "user.email", "test@example.com"], &root).unwrap();
        run_command("git", &["config", "user.name", "SubDispatch Test"], &root).unwrap();
        fs::write(root.join("README.md"), "initial\n").unwrap();
        run_command("git", &["add", "README.md"], &root).unwrap();
        run_command("git", &["commit", "-m", "initial"], &root).unwrap();

        let mut workers = BTreeMap::new();
        workers.insert(
            "worker".to_string(),
            WorkerConfig {
                id: "worker".to_string(),
                command: vec!["sh".to_string(), "-c".to_string(), "true".to_string()],
                max_concurrency: 1,
                model: None,
                enabled: true,
                env: BTreeMap::new(),
                worker_mode: "trusted-worktree".to_string(),
                permission_mode: "bypassPermissions".to_string(),
                description: "test".to_string(),
                strengths: Vec::new(),
                cost: "test".to_string(),
                speed: "test".to_string(),
                delegation_trust: "medium".to_string(),
            },
        );
        let mut engine = SubDispatchEngine {
            workspace: root.clone(),
            tasks_dir: root.join(".subdispatch").join("tasks"),
            worktrees_dir: root.join(".subdispatch").join("worktrees").join("slots"),
            workers,
            prompts: PromptConfig::default(),
        };
        let err = engine
            .start_task(json!({
                "task_id": "conflict",
                "worker": "worker",
                "instruction": "do nothing",
                "read_scope": ["src/task.rs"],
                "forbidden_paths": ["src"]
            }))
            .unwrap_err();
        assert!(err.contains("scope contract conflict"));
        assert!(!root
            .join(".subdispatch")
            .join("worktrees")
            .join("slots")
            .join("conflict")
            .exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tail_handles_utf8_boundaries() {
        assert_eq!(tail_chars("hello中文日志", 4), "中文日志");
    }

    #[test]
    fn transcript_tool_result_line_is_compacted() {
        let mut tool_uses = BTreeMap::new();
        tool_uses.insert(
            "call_1".to_string(),
            TranscriptToolUse {
                name: "Bash".to_string(),
                command: Some("cargo test".to_string()),
            },
        );
        let line = r#"{"timestamp":"2026-05-11T16:10:36.537Z","message":{"content":[{"type":"tool_result","tool_use_id":"call_1","content":"Exit code 101\nintentional_subdispatch_failure_probe ... FAILED","is_error":true}]}}"#;
        let compact = compact_transcript_tool_result_line(line, &tool_uses).unwrap();
        assert_eq!(compact["tool_use_id"], "call_1");
        assert_eq!(compact["tool_name"], "Bash");
        assert_eq!(compact["command"], "cargo test");
        assert_eq!(compact["is_error"], true);
        assert_eq!(compact["source"], "message.content");
        assert!(compact["content_tail"]
            .as_str()
            .unwrap()
            .contains("Exit code 101"));
    }

    #[test]
    fn transcript_tool_result_content_is_trimmed() {
        let mut value = json!({ "content_tail": "0123456789中文日志" });
        trim_tool_result_content(&mut value, 4);
        assert_eq!(value["content_tail"], "中文日志");
    }

    #[test]
    fn transcript_tool_results_tail_prefers_verification_results() {
        let root = env::temp_dir().join(format!("subdispatch-tool-results-test-{}", now_secs()));
        let transcript_path = root.join("transcript.jsonl");
        let task_dir = root.join(".subdispatch").join("tasks").join("task");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            &transcript_path,
            [
                r#"{"timestamp":"t0","message":{"content":[{"type":"tool_use","id":"read","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#,
                r#"{"timestamp":"t1","message":{"content":[{"type":"tool_result","tool_use_id":"read","content":"large file content","is_error":false}]}}"#,
                r#"{"timestamp":"t1b","message":{"content":[{"type":"tool_use","id":"test","name":"Bash","input":{"command":"cargo test 2>&1"}}]}}"#,
                r#"{"timestamp":"t2","message":{"content":[{"type":"tool_result","tool_use_id":"test","content":"Exit code 101\nfailed test","is_error":true}]}}"#,
                r#"{"timestamp":"t2b","message":{"content":[{"type":"tool_use","id":"test2","name":"Bash","input":{"command":"cargo test 2>&1"}}]}}"#,
                r#"{"timestamp":"t3","message":{"content":[{"type":"tool_result","tool_use_id":"test2","content":"running 5 tests\ntest result: ok. 5 passed","is_error":false}]}}"#,
                r#"{"timestamp":"t3b","message":{"content":[{"type":"tool_use","id":"write","name":"Write","input":{"file_path":"result.json"}}]}}"#,
                r#"{"timestamp":"t4","message":{"content":[{"type":"tool_result","tool_use_id":"write","content":"wrote manifest","is_error":false}]}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let engine = SubDispatchEngine {
            workspace: root.clone(),
            tasks_dir: root.join(".subdispatch").join("tasks"),
            worktrees_dir: root.join(".subdispatch").join("worktrees").join("slots"),
            workers: BTreeMap::new(),
            prompts: PromptConfig::default(),
        };
        let task = test_task(&root, Some(task_dir.join("hook_summary.json")));
        write_json(
            &task_dir.join("hook_summary.json"),
            &json!({ "transcript_path": transcript_path.display().to_string() }),
        )
        .unwrap();

        let results = engine.transcript_tool_results_tail(&task, 4, 2000).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["tool_use_id"], "test");
        assert_eq!(results[1]["tool_use_id"], "test2");
        assert!(results[0]["content_tail"]
            .as_str()
            .unwrap()
            .contains("Exit code 101"));
        assert!(results[1]["content_tail"]
            .as_str()
            .unwrap()
            .contains("test result: ok"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transcript_tool_results_tail_ignores_non_verification_bash() {
        let root = env::temp_dir().join(format!("subdispatch-nonverify-test-{}", now_secs()));
        let transcript_path = root.join("transcript.jsonl");
        let task_dir = root.join(".subdispatch").join("tasks").join("task");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            &transcript_path,
            [
                r#"{"timestamp":"t1","message":{"content":[{"type":"tool_use","id":"mkdir","name":"Bash","input":{"command":"mkdir -p .subdispatch"}}]}}"#,
                r#"{"timestamp":"t2","message":{"content":[{"type":"tool_result","tool_use_id":"mkdir","content":"(Bash completed with no output)","is_error":false}]}}"#,
                r#"{"timestamp":"t3","message":{"content":[{"type":"tool_use","id":"test","name":"Bash","input":{"command":"cargo test"}}]}}"#,
                r#"{"timestamp":"t4","message":{"content":[{"type":"tool_result","tool_use_id":"test","content":"test result: ok. 1 passed","is_error":false}]}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let engine = SubDispatchEngine {
            workspace: root.clone(),
            tasks_dir: root.join(".subdispatch").join("tasks"),
            worktrees_dir: root.join(".subdispatch").join("worktrees").join("slots"),
            workers: BTreeMap::new(),
            prompts: PromptConfig::default(),
        };
        let task = test_task(&root, Some(task_dir.join("hook_summary.json")));
        write_json(
            &task_dir.join("hook_summary.json"),
            &json!({ "transcript_path": transcript_path.display().to_string() }),
        )
        .unwrap();

        let results = engine.transcript_tool_results_tail(&task, 4, 2000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["tool_use_id"], "test");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forbidden_path_attempts_report_transient_pretool_edits() {
        let line = r#"{"recorded_at":1.0,"hook_event_name":"PreToolUse","tool_name":"Edit","raw":{"tool_input":{"file_path":"/repo/.subdispatch/worktrees/slots/glm/slot-0/Cargo.toml"}}}"#;
        let attempts = compact_forbidden_attempt_line(line, &["Cargo.toml".to_string()]).unwrap();
        assert_eq!(attempts["tool_name"], "Edit");
        assert_eq!(attempts["file_path"], "Cargo.toml");
        assert_eq!(attempts["forbidden_path"], "Cargo.toml");
    }

    #[test]
    fn forbidden_path_attempts_ignore_result_manifest_write() {
        let line = r#"{"recorded_at":1.0,"hook_event_name":"PreToolUse","tool_name":"Write","raw":{"tool_input":{"file_path":"/repo/.subdispatch/worktrees/slots/glm/slot-0/.subdispatch/result.json"}}}"#;
        assert!(compact_forbidden_attempt_line(line, &[".subdispatch".to_string()]).is_none());
    }

    #[test]
    fn transcript_path_falls_back_when_agent_path_is_null() {
        let root = env::temp_dir().join(format!("subdispatch-transcript-test-{}", now_secs()));
        let engine = SubDispatchEngine {
            workspace: root.clone(),
            tasks_dir: root.join(".subdispatch").join("tasks"),
            worktrees_dir: root.join(".subdispatch").join("worktrees").join("slots"),
            workers: BTreeMap::new(),
            prompts: PromptConfig::default(),
        };
        let task = test_task(
            &root,
            Some(
                root.join(".subdispatch")
                    .join("tasks")
                    .join("task")
                    .join("hook_summary.json"),
            ),
        );
        write_json(
            &PathBuf::from(task.hook_summary_path.as_ref().unwrap()),
            &json!({
                "agent_transcript_path": null,
                "transcript_path": "/tmp/transcript.jsonl"
            }),
        )
        .unwrap();

        assert_eq!(
            engine.transcript_path(&task).unwrap(),
            Some("/tmp/transcript.jsonl".to_string())
        );

        fs::remove_dir_all(root).unwrap();
    }

    fn test_task(root: &Path, hook_summary_path: Option<PathBuf>) -> TaskState {
        TaskState {
            id: "task".to_string(),
            goal: "goal".to_string(),
            instruction: "instruction".to_string(),
            worker: "worker".to_string(),
            status: STATUS_COMPLETED.to_string(),
            branch: "agent/task".to_string(),
            worktree: root.join("worktree").display().to_string(),
            base_commit: "base".to_string(),
            slot_id: None,
            read_scope: Vec::new(),
            write_scope: Vec::new(),
            forbidden_paths: Vec::new(),
            context: String::new(),
            context_files: Vec::new(),
            created_at: now_secs(),
            pid: None,
            exit_code: Some(0),
            error: None,
            started_at: None,
            finished_at: None,
            exit_path: None,
            hook_events_path: None,
            hook_summary_path: hook_summary_path.map(|path| path.display().to_string()),
            command: None,
            warning: None,
            worktree_removed: None,
            worktree_deleted_at: None,
            collected_at: None,
        }
    }
}
