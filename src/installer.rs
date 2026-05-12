use crate::config::{default_workers, load_env};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SECTION_HEADER: &str = "[mcp_servers.subdispatch]";
const SKILL_NAME: &str = "subdispatch-delegation";
const SKILL_CONTENT: &str = include_str!("../skills/subdispatch-delegation/SKILL.md");

pub fn install(workspace: &Path, project: bool, global: bool) -> Result<Value, String> {
    let mut actions = Vec::new();
    let exe = current_exe_string()?;
    if project {
        actions.push(install_config(
            &workspace.join(".codex").join("config.toml"),
            &exe,
            Some(workspace),
        )?);
    }
    if global {
        actions.push(install_config(
            &home_dir()?.join(".codex").join("config.toml"),
            &exe,
            None,
        )?);
    }
    Ok(json!({
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "binary": exe,
        "actions": actions,
        "next_step": "Run `subdispatch init-env --workspace .` in each project, edit .env, then run `subdispatch doctor --workspace .`."
    }))
}

pub fn doctor(workspace: &Path) -> Result<Value, String> {
    let env_path = workspace.join(".env");
    let settings = load_env(workspace)?;
    let workers = default_workers(&settings)?;
    let project_config = workspace.join(".codex").join("config.toml");
    let global_config = home_dir()?.join(".codex").join("config.toml");
    let project_mcp_installed = config_has_subdispatch(&project_config)?;
    let global_mcp_installed = config_has_subdispatch(&global_config)?;
    let git_available = command_available("git");
    let git_repo = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(workspace)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    let claude_available = command_available("claude");
    let worker_reports = workers
        .values()
        .map(|worker| {
            let executable =
                command_available(worker.command.first().map(String::as_str).unwrap_or(""));
            json!({
                "id": worker.id,
                "model": worker.model,
                "command": worker.command,
                "enabled": worker.enabled,
                "command_available": executable,
                "max_concurrency": worker.max_concurrency,
                "description": worker.description,
                "strengths": worker.strengths,
                "cost": worker.cost,
                "speed": worker.speed,
                "delegation_trust": worker.delegation_trust,
            })
        })
        .collect::<Vec<_>>();
    let ready = git_available
        && git_repo
        && env_path.exists()
        && (project_mcp_installed || global_mcp_installed)
        && workers.values().any(|worker| {
            worker.enabled
                && command_available(worker.command.first().map(String::as_str).unwrap_or(""))
        });
    Ok(json!({
        "status": if ready { "ok" } else { "needs_attention" },
        "workspace": workspace.display().to_string(),
        "binary": current_exe_string()?,
        "checks": {
            "git_available": git_available,
            "git_repo": git_repo,
            "claude_available": claude_available,
            "env_exists": env_path.exists(),
            "env_path": env_path.display().to_string(),
            "project_mcp_installed": project_mcp_installed,
            "project_config": project_config.display().to_string(),
            "global_mcp_installed": global_mcp_installed,
            "global_config": global_config.display().to_string(),
        },
        "workers": worker_reports,
        "next_step": recommended_next_step(env_path.exists(), project_mcp_installed || global_mcp_installed, ready)
    }))
}

pub fn install_skill() -> Result<Value, String> {
    let skill_dir = home_dir()?.join(".codex").join("skills").join(SKILL_NAME);
    fs::create_dir_all(&skill_dir)
        .map_err(|err| format!("failed to create {}: {err}", skill_dir.display()))?;
    let skill_path = skill_dir.join("SKILL.md");
    fs::write(&skill_path, SKILL_CONTENT)
        .map_err(|err| format!("failed to write {}: {err}", skill_path.display()))?;
    Ok(json!({
        "status": "ok",
        "skill": SKILL_NAME,
        "path": skill_path.display().to_string(),
        "next_step": "Restart or reload your agent session so the new skill is discovered."
    }))
}

fn install_config(
    config_path: &Path,
    binary: &str,
    workspace: Option<&Path>,
) -> Result<Value, String> {
    let existing = if config_path.exists() {
        fs::read_to_string(config_path)
            .map_err(|err| format!("failed to read {}: {err}", config_path.display()))?
    } else {
        String::new()
    };
    let section = render_section(binary, workspace);
    let updated = replace_section(&existing, &section);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs::write(config_path, updated)
        .map_err(|err| format!("failed to write {}: {err}", config_path.display()))?;
    Ok(json!({
        "config_path": config_path.display().to_string(),
        "installed": true,
        "command": binary,
        "args": render_args(workspace)
    }))
}

fn render_section(binary: &str, workspace: Option<&Path>) -> String {
    let args = match workspace {
        Some(workspace) => format!(
            "[\"mcp\", \"--workspace\", \"{}\"]",
            escape_toml_string(&workspace.display().to_string())
        ),
        None => "[\"mcp\"]".to_string(),
    };
    format!(
        "{SECTION_HEADER}\ncommand = \"{}\"\nargs = {args}\nstartup_timeout_sec = 10\ntool_timeout_sec = 120\n",
        escape_toml_string(binary),
    )
}

fn render_args(workspace: Option<&Path>) -> Value {
    match workspace {
        Some(workspace) => json!(["mcp", "--workspace", workspace.display().to_string()]),
        None => json!(["mcp"]),
    }
}

fn replace_section(existing: &str, section: &str) -> String {
    let mut output = Vec::new();
    let lines = existing.lines().collect::<Vec<_>>();
    let mut index = 0;
    let mut replaced = false;
    while index < lines.len() {
        if lines[index].trim() == SECTION_HEADER {
            if !output
                .last()
                .is_some_and(|line: &&str| line.trim().is_empty())
                && !output.is_empty()
            {
                output.push("");
            }
            for line in section.trim_end().lines() {
                output.push(line);
            }
            replaced = true;
            index += 1;
            while index < lines.len() {
                let trimmed = lines[index].trim_start();
                if trimmed.starts_with('[') {
                    break;
                }
                index += 1;
            }
            continue;
        }
        output.push(lines[index]);
        index += 1;
    }
    if !replaced {
        if !output.is_empty() && !output.last().is_some_and(|line| line.trim().is_empty()) {
            output.push("");
        }
        for line in section.trim_end().lines() {
            output.push(line);
        }
    }
    let mut text = output.join("\n");
    text.push('\n');
    text
}

fn config_has_subdispatch(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    Ok(text.lines().any(|line| line.trim() == SECTION_HEADER))
}

fn command_available(command: &str) -> bool {
    if command.is_empty() {
        return false;
    }
    let path = Path::new(command);
    if command.contains('/') {
        return path.exists();
    }
    env::var_os("PATH")
        .and_then(|paths| {
            env::split_paths(&paths)
                .map(|dir| dir.join(command))
                .find(|candidate| candidate.exists())
        })
        .is_some()
}

fn current_exe_string() -> Result<String, String> {
    env::current_exe()
        .map_err(|err| format!("failed to locate current executable: {err}"))
        .map(|path| path.display().to_string())
}

fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())
}

fn escape_toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn recommended_next_step(env_exists: bool, mcp_installed: bool, ready: bool) -> &'static str {
    if ready {
        "SubDispatch is ready. Use MCP tools from the primary agent or run `subdispatch serve --workspace .`."
    } else if !env_exists {
        "Run `subdispatch init-env --workspace .`, edit .env, then rerun doctor."
    } else if !mcp_installed {
        "Run `subdispatch install --global`, or use `subdispatch install --project --workspace .` for project-local MCP config."
    } else {
        "Check worker command availability and .env worker settings."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_only_subdispatch_section() {
        let existing = "[x]\na = 1\n\n[mcp_servers.subdispatch]\ncommand = \"old\"\n\n[y]\nb = 2\n";
        let updated = replace_section(existing, "[mcp_servers.subdispatch]\ncommand = \"new\"\n");
        assert!(updated.contains("[x]\na = 1"));
        assert!(updated.contains("command = \"new\""));
        assert!(updated.contains("[y]\nb = 2"));
        assert!(!updated.contains("command = \"old\""));
    }

    #[test]
    fn global_section_does_not_pin_workspace() {
        let section = render_section("/usr/local/bin/subdispatch", None);
        assert!(section.contains("args = [\"mcp\"]"));
        assert!(!section.contains("--workspace"));
    }

    #[test]
    fn project_section_pins_workspace() {
        let section = render_section("/usr/local/bin/subdispatch", Some(Path::new("/repo")));
        assert!(section.contains("args = [\"mcp\", \"--workspace\", \"/repo\"]"));
    }
}
