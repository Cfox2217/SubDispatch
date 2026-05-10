mod config;
mod engine;
mod installer;
mod mcp;
mod web;

use clap::{Parser, Subcommand};
use engine::SubDispatchEngine;
use serde_json::Value;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "subdispatch")]
#[command(version)]
#[command(about = "Local parallel child-agent scaffold for primary LLMs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create .env and .env.example templates.
    InitEnv {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    /// Install SubDispatch MCP config for this project or globally.
    Install {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        project: bool,
        #[arg(long)]
        global: bool,
    },
    /// Diagnose local SubDispatch readiness.
    Doctor {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// List configured workers and available capacity.
    Workers {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Start a run from a JSON file.
    StartRun {
        json_file: PathBuf,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Poll one run.
    PollRun {
        run_id: String,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Collect one task's artifact bundle.
    CollectTask {
        run_id: String,
        task_id: String,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Delete one managed task worktree.
    DeleteWorktree {
        run_id: String,
        task_id: String,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        delete_branch: bool,
    },
    /// Run the MCP stdio server.
    Mcp {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Serve the local Setup and Activity UI.
    Serve {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8765")]
        bind: String,
    },
    /// Internal supervisor used to detach child-agent execution.
    #[command(hide = true)]
    Supervise { launch_json: PathBuf },
    /// Internal Claude hook recorder. Reads one hook payload from stdin.
    #[command(hide = true)]
    HookRecord {
        #[arg(long)]
        events: PathBuf,
        #[arg(long)]
        summary: PathBuf,
        #[arg(long)]
        run_id: String,
        #[arg(long)]
        task_id: String,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.command {
        Command::InitEnv {
            workspace,
            overwrite,
        } => {
            print_json(&config::init_env(&workspace, overwrite)?)?;
        }
        Command::Install {
            workspace,
            project,
            global,
        } => {
            let install_project = project || !global;
            let install_global = global;
            print_json(&installer::install(
                &workspace,
                install_project,
                install_global,
            )?)?;
        }
        Command::Doctor { workspace } => {
            print_json(&installer::doctor(&workspace)?)?;
        }
        Command::Workers { workspace } => {
            let engine = SubDispatchEngine::new(workspace)?;
            print_json(&engine.list_workers()?)?;
        }
        Command::StartRun {
            json_file,
            workspace,
        } => {
            let text = fs::read_to_string(&json_file)
                .map_err(|err| format!("failed to read {}: {err}", json_file.display()))?;
            let input: Value = serde_json::from_str(&text)
                .map_err(|err| format!("invalid JSON in {}: {err}", json_file.display()))?;
            let mut engine = SubDispatchEngine::new(workspace)?;
            print_json(&engine.start_run(input)?)?;
        }
        Command::PollRun { run_id, workspace } => {
            let mut engine = SubDispatchEngine::new(workspace)?;
            print_json(&engine.poll_run(&run_id)?)?;
        }
        Command::CollectTask {
            run_id,
            task_id,
            workspace,
        } => {
            let mut engine = SubDispatchEngine::new(workspace)?;
            print_json(&engine.collect_task(&run_id, &task_id)?)?;
        }
        Command::DeleteWorktree {
            run_id,
            task_id,
            workspace,
            force,
            delete_branch,
        } => {
            let mut engine = SubDispatchEngine::new(workspace)?;
            print_json(&engine.delete_worktree(&run_id, &task_id, force, delete_branch)?)?;
        }
        Command::Mcp { workspace } => {
            mcp::serve_stdio(workspace)?;
        }
        Command::Serve { workspace, bind } => {
            web::serve(workspace, &bind)?;
        }
        Command::Supervise { launch_json } => {
            engine::supervise(&launch_json)?;
        }
        Command::HookRecord {
            events,
            summary,
            run_id,
            task_id,
        } => {
            let mut input = String::new();
            io::stdin()
                .read_to_string(&mut input)
                .map_err(|err| format!("failed to read hook stdin: {err}"))?;
            engine::record_hook_event(&events, &summary, &run_id, &task_id, &input)?;
        }
    }
    Ok(())
}

fn print_json(value: &Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value)
            .map_err(|err| format!("failed to render JSON: {err}"))?
    );
    Ok(())
}
