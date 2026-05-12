use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerConfig {
    pub id: String,
    pub command: Vec<String>,
    pub max_concurrency: usize,
    pub model: Option<String>,
    pub enabled: bool,
    pub env: BTreeMap<String, String>,
    pub worker_mode: String,
    pub permission_mode: String,
    pub description: String,
    pub strengths: Vec<String>,
    pub cost: String,
    pub speed: String,
    pub delegation_trust: String,
}

pub fn load_env(workspace: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut values: BTreeMap<String, String> = env::vars().collect();
    let env_path = workspace.join(".env");
    if !env_path.exists() {
        return Ok(values);
    }
    let text = fs::read_to_string(&env_path)
        .map_err(|err| format!("failed to read {}: {err}", env_path.display()))?;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let (key, value) = line.split_once('=').expect("contains checked");
        let key = key.trim();
        let mut value = value.trim().to_string();
        if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            value = value[1..value.len().saturating_sub(1)].to_string();
        }
        values.insert(key.to_string(), value);
    }
    Ok(values)
}

pub fn default_workers(
    settings: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, WorkerConfig>, String> {
    let configured = settings
        .get("SUBDISPATCH_WORKERS")
        .map(|value| csv_list(value))
        .unwrap_or_default();
    if configured.is_empty() {
        let worker = default_claude_worker(settings)?;
        return Ok(BTreeMap::from([(worker.id.clone(), worker)]));
    }
    let mut workers = BTreeMap::new();
    for worker_id in configured {
        let worker = worker_from_env(&worker_id, settings)?;
        workers.insert(worker.id.clone(), worker);
    }
    Ok(workers)
}

fn default_claude_worker(settings: &BTreeMap<String, String>) -> Result<WorkerConfig, String> {
    let model = settings.get("SUBDISPATCH_CLAUDE_MODEL").cloned();
    let command_text = settings
        .get("SUBDISPATCH_CLAUDE_COMMAND")
        .map(String::as_str)
        .unwrap_or("claude -p $prompt --permission-mode $permission_mode --output-format text");
    let mut command = split_command(command_text)?;
    if model.is_some() && !command_text.contains("$model") {
        command.push("--model".to_string());
        command.push("$model".to_string());
    }
    let mut worker_env = BTreeMap::new();
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_AUTH_TOKEN",
    ] {
        if let Some(value) = settings.get(key).filter(|value| !value.is_empty()) {
            worker_env.insert(key.to_string(), value.clone());
        }
    }
    Ok(WorkerConfig {
        id: "claude-code".to_string(),
        command,
        max_concurrency: parse_usize(settings, "SUBDISPATCH_CLAUDE_MAX_CONCURRENCY", 1)?,
        model,
        enabled: settings
            .get("SUBDISPATCH_CLAUDE_ENABLED")
            .map(|value| value != "0")
            .unwrap_or(true),
        env: worker_env,
        worker_mode: settings
            .get("SUBDISPATCH_WORKER_MODE")
            .cloned()
            .unwrap_or_else(|| "trusted-worktree".to_string()),
        permission_mode: settings
            .get("SUBDISPATCH_CLAUDE_PERMISSION_MODE")
            .cloned()
            .unwrap_or_else(|| "bypassPermissions".to_string()),
        description: settings
            .get("SUBDISPATCH_CLAUDE_DESCRIPTION")
            .cloned()
            .unwrap_or_else(|| "Default Claude Code worker for general coding tasks.".to_string()),
        strengths: csv_list(
            settings
                .get("SUBDISPATCH_CLAUDE_STRENGTHS")
                .map(String::as_str)
                .unwrap_or("general coding,repository edits,tests,documentation"),
        ),
        cost: settings
            .get("SUBDISPATCH_CLAUDE_COST")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        speed: settings
            .get("SUBDISPATCH_CLAUDE_SPEED")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        delegation_trust: settings
            .get("SUBDISPATCH_CLAUDE_DELEGATION_TRUST")
            .cloned()
            .unwrap_or_else(|| "medium".to_string()),
    })
}

fn worker_from_env(
    worker_id: &str,
    settings: &BTreeMap<String, String>,
) -> Result<WorkerConfig, String> {
    let prefix = format!("SUBDISPATCH_WORKER_{}_", env_key(worker_id));
    let model = settings.get(&(prefix.clone() + "MODEL")).cloned();
    let fallback_command = settings
        .get("SUBDISPATCH_CLAUDE_COMMAND")
        .map(String::as_str)
        .unwrap_or("claude -p $prompt --permission-mode $permission_mode --output-format text");
    let command_key = prefix.clone() + "COMMAND";
    let command_text = settings
        .get(&command_key)
        .map(String::as_str)
        .unwrap_or(fallback_command);
    let mut command = split_command(command_text)?;
    if model.is_some() && !command_text.contains("$model") {
        command.push("--model".to_string());
        command.push("$model".to_string());
    }
    let mut worker_env = BTreeMap::new();
    let env_prefix = prefix.clone() + "ENV_";
    for (key, value) in settings {
        if let Some(name) = key.strip_prefix(&env_prefix) {
            if !value.is_empty() {
                worker_env.insert(name.to_string(), value.clone());
            }
        }
    }
    Ok(WorkerConfig {
        id: worker_id.to_string(),
        command,
        max_concurrency: parse_usize(settings, &(prefix.clone() + "MAX_CONCURRENCY"), 1)?,
        model,
        enabled: settings
            .get(&(prefix.clone() + "ENABLED"))
            .map(|value| value != "0")
            .unwrap_or(true),
        env: worker_env,
        worker_mode: settings
            .get(&(prefix.clone() + "MODE"))
            .or_else(|| settings.get("SUBDISPATCH_WORKER_MODE"))
            .cloned()
            .unwrap_or_else(|| "trusted-worktree".to_string()),
        permission_mode: settings
            .get(&(prefix.clone() + "PERMISSION_MODE"))
            .or_else(|| settings.get("SUBDISPATCH_CLAUDE_PERMISSION_MODE"))
            .cloned()
            .unwrap_or_else(|| "bypassPermissions".to_string()),
        description: settings
            .get(&(prefix.clone() + "DESCRIPTION"))
            .cloned()
            .unwrap_or_else(|| format!("{worker_id} Claude Code worker.")),
        strengths: csv_list(
            settings
                .get(&(prefix.clone() + "STRENGTHS"))
                .map(String::as_str)
                .unwrap_or("general coding"),
        ),
        cost: settings
            .get(&(prefix.clone() + "COST"))
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        speed: settings
            .get(&(prefix.clone() + "SPEED"))
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        delegation_trust: settings
            .get(&(prefix + "DELEGATION_TRUST"))
            .cloned()
            .unwrap_or_else(|| "medium".to_string()),
    })
}

fn parse_usize(
    settings: &BTreeMap<String, String>,
    key: &str,
    default: usize,
) -> Result<usize, String> {
    match settings.get(key) {
        Some(value) => {
            let parsed = value
                .parse::<usize>()
                .map_err(|err| format!("{key} must be a positive integer: {err}"))?;
            if parsed == 0 {
                return Err(format!("{key} must be greater than 0"));
            }
            Ok(parsed)
        }
        None => Ok(default),
    }
}

pub fn split_command(command: &str) -> Result<Vec<String>, String> {
    shell_words::split(command)
        .map_err(|err| format!("invalid command template {command:?}: {err}"))
}

pub fn csv_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub fn env_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

pub fn init_env(workspace: &Path, overwrite: bool) -> Result<Value, String> {
    fs::create_dir_all(workspace)
        .map_err(|err| format!("failed to create {}: {err}", workspace.display()))?;
    let env_path = workspace.join(".env");
    let example_path = workspace.join(".env.example");
    let example_changed = overwrite || !example_path.exists();
    let mut env_created = false;
    if example_changed {
        fs::write(&example_path, ENV_TEMPLATE)
            .map_err(|err| format!("failed to write {}: {err}", example_path.display()))?;
    }
    if overwrite || !env_path.exists() {
        fs::write(&env_path, ENV_TEMPLATE)
            .map_err(|err| format!("failed to write {}: {err}", env_path.display()))?;
        env_created = true;
    }
    Ok(json!({
        "status": "ok",
        "env_path": absolute_display(&env_path)?,
        "env_created": env_created,
        "example_path": absolute_display(&example_path)?,
        "example_changed": example_changed,
        "next_step": "Edit .env, then run `subdispatch workers`."
    }))
}

fn absolute_display(path: &Path) -> Result<String, String> {
    let resolved: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|err| format!("failed to read current directory: {err}"))?
            .join(path)
    };
    Ok(resolved.display().to_string())
}

pub const ENV_TEMPLATE: &str = r#"# SubDispatch local configuration.
# Copy this file to .env and edit values for this workspace.
# .env is intentionally git-ignored.

SUBDISPATCH_CLAUDE_ENABLED=1
SUBDISPATCH_WORKER_MODE=trusted-worktree
SUBDISPATCH_CLAUDE_PERMISSION_MODE=bypassPermissions

SUBDISPATCH_CLAUDE_DESCRIPTION=Default Claude Code worker for general coding tasks.
SUBDISPATCH_CLAUDE_STRENGTHS=general coding,repository edits,tests,documentation
SUBDISPATCH_CLAUDE_COST=unknown
SUBDISPATCH_CLAUDE_SPEED=unknown
SUBDISPATCH_CLAUDE_DELEGATION_TRUST=medium

SUBDISPATCH_CLAUDE_COMMAND=claude -p $prompt --permission-mode $permission_mode --output-format text
# SUBDISPATCH_CLAUDE_MODEL=claude-sonnet-4-5
SUBDISPATCH_CLAUDE_MAX_CONCURRENCY=1

# ANTHROPIC_API_KEY=
# ANTHROPIC_BASE_URL=
# ANTHROPIC_AUTH_TOKEN=

# SUBDISPATCH_WORKERS=glm,minimax

# SUBDISPATCH_WORKER_GLM_ENABLED=1
# SUBDISPATCH_WORKER_GLM_MODEL=glm-5.1
# SUBDISPATCH_WORKER_GLM_MAX_CONCURRENCY=2
# SUBDISPATCH_WORKER_GLM_DESCRIPTION=Balanced worker for Chinese/English coding tasks, repo edits, and reasoning-heavy implementation.
# SUBDISPATCH_WORKER_GLM_STRENGTHS=general coding,Chinese context,reasoning,tests,documentation
# SUBDISPATCH_WORKER_GLM_COST=medium
# SUBDISPATCH_WORKER_GLM_SPEED=medium
# SUBDISPATCH_WORKER_GLM_DELEGATION_TRUST=high
# SUBDISPATCH_WORKER_GLM_PERMISSION_MODE=bypassPermissions
# SUBDISPATCH_WORKER_GLM_COMMAND=claude -p $prompt --permission-mode $permission_mode --output-format text
# SUBDISPATCH_WORKER_GLM_ENV_ANTHROPIC_BASE_URL=https://open.bigmodel.cn/api/anthropic
# SUBDISPATCH_WORKER_GLM_ENV_ANTHROPIC_API_KEY=

# SUBDISPATCH_WORKER_MINIMAX_ENABLED=1
# SUBDISPATCH_WORKER_MINIMAX_MODEL=MiniMax-M2.7-highspeed
# SUBDISPATCH_WORKER_MINIMAX_MAX_CONCURRENCY=3
# SUBDISPATCH_WORKER_MINIMAX_DESCRIPTION=Fast lower-cost worker for parallel simple edits, docs, search, and small scoped changes.
# SUBDISPATCH_WORKER_MINIMAX_STRENGTHS=parallel throughput,simple edits,documentation,code search,boilerplate
# SUBDISPATCH_WORKER_MINIMAX_COST=low
# SUBDISPATCH_WORKER_MINIMAX_SPEED=fast
# SUBDISPATCH_WORKER_MINIMAX_DELEGATION_TRUST=high
# SUBDISPATCH_WORKER_MINIMAX_PERMISSION_MODE=bypassPermissions
# SUBDISPATCH_WORKER_MINIMAX_COMMAND=claude -p $prompt --permission-mode $permission_mode --output-format text
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_BASE_URL=https://api.minimaxi.com/anthropic
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_AUTH_TOKEN=
# SUBDISPATCH_WORKER_MINIMAX_ENV_API_TIMEOUT_MS=3000000
# SUBDISPATCH_WORKER_MINIMAX_ENV_CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_MODEL=MiniMax-M2.7-highspeed
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_DEFAULT_SONNET_MODEL=MiniMax-M2.7-highspeed
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_DEFAULT_OPUS_MODEL=MiniMax-M2.7-highspeed
# SUBDISPATCH_WORKER_MINIMAX_ENV_ANTHROPIC_DEFAULT_HAIKU_MODEL=MiniMax-M2.7-highspeed
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_commands() {
        let command = split_command("python -c 'print(1)'").unwrap();
        assert_eq!(command, vec!["python", "-c", "print(1)"]);
    }

    #[test]
    fn worker_env_does_not_leak_into_serialized_worker() {
        let mut settings = BTreeMap::new();
        settings.insert("SUBDISPATCH_WORKERS".to_string(), "glm".to_string());
        settings.insert(
            "SUBDISPATCH_WORKER_GLM_MODEL".to_string(),
            "glm-5.1".to_string(),
        );
        settings.insert(
            "SUBDISPATCH_WORKER_GLM_MAX_CONCURRENCY".to_string(),
            "2".to_string(),
        );
        settings.insert(
            "SUBDISPATCH_WORKER_GLM_ENV_ANTHROPIC_API_KEY".to_string(),
            "secret".to_string(),
        );
        let workers = default_workers(&settings).unwrap();
        let worker = workers.get("glm").unwrap();
        assert_eq!(worker.max_concurrency, 2);
        assert_eq!(worker.env.get("ANTHROPIC_API_KEY").unwrap(), "secret");
    }

    #[test]
    fn max_concurrency_must_be_positive() {
        let mut settings = BTreeMap::new();
        settings.insert(
            "SUBDISPATCH_WORKER_GLM_MAX_CONCURRENCY".to_string(),
            "0".to_string(),
        );
        let err = worker_from_env("glm", &settings).unwrap_err();
        assert!(err.contains("must be greater than 0"));
    }
}
