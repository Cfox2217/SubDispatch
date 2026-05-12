use crate::config;
use crate::engine::SubDispatchEngine;
use crate::installer;
use crate::prompts;
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

pub fn serve(workspace: PathBuf, bind: &str) -> Result<(), String> {
    let listener =
        TcpListener::bind(bind).map_err(|err| format!("failed to bind {bind}: {err}"))?;
    eprintln!("SubDispatch UI listening on http://{bind}");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(err) = handle_connection(stream, workspace.clone()) {
                    eprintln!("{err}");
                }
            }
            Err(err) => eprintln!("failed to accept HTTP connection: {err}"),
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream, workspace: PathBuf) -> Result<(), String> {
    let request = read_http_request(&mut stream)?;
    let Some(first_line) = request.lines().next() else {
        return Ok(());
    };
    let parts = first_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 2 {
        return write_response(&mut stream, 400, "text/plain; charset=utf-8", "Bad request");
    }
    let method = parts[0];
    let path = parts[1];
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            write_response(&mut stream, 200, "text/html; charset=utf-8", INDEX_HTML)
        }
        ("GET", "/api/snapshot") => {
            let mut engine = SubDispatchEngine::new(workspace)?;
            let snapshot = engine.activity_snapshot()?;
            write_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &serde_json::to_string_pretty(&snapshot).map_err(|err| err.to_string())?,
            )
        }
        ("GET", "/api/setup") => {
            let setup = installer::doctor(&workspace)?;
            write_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &serde_json::to_string_pretty(&setup).map_err(|err| err.to_string())?,
            )
        }
        ("POST", "/api/init-env") => {
            let result = config::init_env(&workspace, false)?;
            write_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &serde_json::to_string_pretty(&result).map_err(|err| err.to_string())?,
            )
        }
        ("GET", "/api/env") => {
            let result = read_env_for_ui(&workspace)?;
            write_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &serde_json::to_string_pretty(&result).map_err(|err| err.to_string())?,
            )
        }
        ("POST", "/api/env") => {
            let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
            let result = save_env_from_ui(&workspace, body)?;
            write_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &serde_json::to_string_pretty(&result).map_err(|err| err.to_string())?,
            )
        }
        ("GET", "/api/prompts") => {
            let result = prompts::prompt_config_for_ui(&workspace)?;
            write_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &serde_json::to_string_pretty(&result).map_err(|err| err.to_string())?,
            )
        }
        ("POST", "/api/prompts") => {
            let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
            let result = prompts::save_prompt_config_from_ui(&workspace, body)?;
            write_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &serde_json::to_string_pretty(&result).map_err(|err| err.to_string())?,
            )
        }
        _ => write_response(&mut stream, 404, "text/plain; charset=utf-8", "Not found"),
    }
}

fn read_http_request(stream: &mut TcpStream) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let size = stream
            .read(&mut buffer)
            .map_err(|err| format!("failed to read HTTP request: {err}"))?;
        if size == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..size]);
        if let Some(header_end) = find_header_end(&bytes) {
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    if name.eq_ignore_ascii_case("content-length") {
                        value.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            let body_start = header_end + 4;
            if bytes.len() >= body_start + content_length {
                break;
            }
        }
        if bytes.len() > 192 * 1024 {
            return Err("HTTP request is too large".to_string());
        }
    }
    String::from_utf8(bytes).map_err(|err| format!("HTTP request is not valid UTF-8: {err}"))
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn read_env_for_ui(workspace: &PathBuf) -> Result<serde_json::Value, String> {
    let env_path = workspace.join(".env");
    let text = if env_path.exists() {
        fs::read_to_string(&env_path)
            .map_err(|err| format!("failed to read {}: {err}", env_path.display()))?
    } else {
        String::new()
    };
    Ok(json!({
        "status": "ok",
        "env_path": env_path.display().to_string(),
        "text": text,
        "allowed_prefixes": ["SUBDISPATCH_", "ANTHROPIC_", "API_TIMEOUT_MS", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"]
    }))
}

fn save_env_from_ui(workspace: &PathBuf, body: &str) -> Result<serde_json::Value, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|err| format!("invalid JSON request body: {err}"))?;
    let text = value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing text field".to_string())?;
    validate_env_text(text)?;
    let env_path = workspace.join(".env");
    fs::write(&env_path, text)
        .map_err(|err| format!("failed to write {}: {err}", env_path.display()))?;
    Ok(json!({
        "status": "ok",
        "env_path": env_path.display().to_string(),
        "bytes": text.len()
    }))
}

fn validate_env_text(text: &str) -> Result<(), String> {
    if text.len() > 64 * 1024 {
        return Err(".env content is too large".to_string());
    }
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, _)) = line.split_once('=') else {
            return Err(format!("line {} is not KEY=VALUE", index + 1));
        };
        let key = key.trim();
        let allowed = key.starts_with("SUBDISPATCH_")
            || key.starts_with("ANTHROPIC_")
            || key == "API_TIMEOUT_MS"
            || key == "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC";
        if !allowed {
            return Err(format!("line {} uses unsupported key: {key}", index + 1));
        }
        if !key
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        {
            return Err(format!("line {} has invalid key: {key}", index + 1));
        }
    }
    Ok(())
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.as_bytes().len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|err| format!("failed to write HTTP response: {err}"))
}

#[allow(dead_code)]
fn read_text(path: &PathBuf) -> Result<String, String> {
    fs::read_to_string(path).map_err(|err| format!("failed to read {}: {err}", path.display()))
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>SubDispatch</title>
  <style>
    :root {
      color-scheme: light;
      --color-cloud-white: #ffffff;
      --color-canvas-fog: #fafaf9;
      --color-slate-text: #0c0a09;
      --color-ash-gray: #78716c;
      --color-stone-border: #e5e7eb;
      --color-platinum-outline: #d6d3d1;
      --color-steel-gray: #a8a29e;
      --color-hover-stone: #c9c5c2;
      --color-ghost-ink: #1c1917;
      --color-chartwell-blue: #3ba6f1;
      --color-sky-tint: #c1e1f7;
      --bg: var(--color-canvas-fog);
      --surface: var(--color-cloud-white);
      --surface-2: #fbfbfa;
      --surface-3: #f3f4f6;
      --line: var(--color-stone-border);
      --line-soft: var(--color-stone-border);
      --text: var(--color-slate-text);
      --muted: var(--color-ash-gray);
      --green: var(--color-ash-gray);
      --blue: var(--color-chartwell-blue);
      --red: #b91c1c;
      --amber: var(--color-ash-gray);
      --accent: var(--color-chartwell-blue);
      --apple-blue: #007aff;
      --terminal-bg: #0b0f14;
      --terminal-panel: #111820;
      --terminal-line: #253241;
      --terminal-text: #d7e0ea;
      --terminal-muted: #7f8b98;
      --shadow: rgba(0, 0, 0, 0.05) 0px 4px 16px 0px;
      --shadow-subtle: rgba(0, 0, 0, 0.05) 0px 1px 2px 0px;
      --shadow-xl: rgba(17, 12, 46, 0.12) 0px 12px 45px 0px;
      --font-inter: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      --font-roobert: roobert, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }
    :root[data-theme="dark"] {
      color-scheme: dark;
      --bg: #111111;
      --surface: #181818;
      --surface-2: #202020;
      --surface-3: #272727;
      --line: #343434;
      --line-soft: #2b2b2b;
      --text: #fafaf9;
      --muted: #a8a29e;
      --green: #a8a29e;
      --blue: #3ba6f1;
      --red: #f87171;
      --amber: #a8a29e;
      --accent: #3ba6f1;
      --apple-blue: #0a84ff;
      --terminal-bg: #0b0f14;
      --terminal-panel: #111820;
      --terminal-line: #253241;
      --terminal-text: #d7e0ea;
      --terminal-muted: #7f8b98;
      --shadow: rgba(0, 0, 0, 0.18) 0px 4px 16px 0px;
      --shadow-subtle: rgba(0, 0, 0, 0.18) 0px 1px 2px 0px;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      min-height: 100vh;
      color: var(--text);
      background: var(--bg);
      font-family: var(--font-inter);
      font-size: 14px;
      line-height: 1.5;
    }
    body::before {
      content: "";
      position: fixed;
      inset: 0;
      z-index: -1;
      pointer-events: none;
      background:
        linear-gradient(180deg, color-mix(in srgb, var(--color-sky-tint) 18%, transparent), transparent 260px),
        radial-gradient(circle at 50% -120px, color-mix(in srgb, var(--color-sky-tint) 22%, transparent), transparent 360px);
      opacity: 0.9;
    }
    :root[data-theme="dark"] body::before { opacity: 0.18; }
    .topbar {
      position: sticky;
      top: 10px;
      z-index: 20;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 16px;
      width: min(1180px, calc(100% - 24px));
      margin: 10px auto 0;
      padding: 7px 10px;
      border: 1px solid var(--line);
      border-radius: 9999px;
      background: color-mix(in srgb, var(--surface) 94%, transparent);
      box-shadow: var(--shadow-subtle);
      backdrop-filter: blur(18px);
    }
    .brand { display: flex; align-items: center; gap: 10px; min-width: 0; }
    .mark {
      display: grid;
      place-items: center;
      width: 36px;
      height: 36px;
      border: 1px solid color-mix(in srgb, var(--accent) 70%, var(--line));
      border-radius: 9999px;
      color: var(--surface);
      background: var(--accent);
      font-weight: 600;
      font-size: 13px;
      box-shadow: var(--shadow-subtle);
    }
    .brand-copy { display: block; }
    h1 { margin: 0; font-family: var(--font-roobert); font-size: 18px; font-weight: 500; letter-spacing: 0; line-height: 1.25; }
    h2 { font-family: var(--font-roobert); font-size: 18px; font-weight: 500; letter-spacing: 0; line-height: 1.25; }
    .subtitle { margin: 0; color: var(--muted); font-size: 12px; letter-spacing: 0; }
    .tabs { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; justify-content: flex-end; }
    button, .tab {
      min-width: 46px;
      height: 34px;
      border: 1px solid var(--line);
      border-radius: 9999px;
      padding: 0 12px;
      color: var(--muted);
      background: transparent;
      font: 500 13px/1 var(--font-inter);
      cursor: pointer;
      transition: border-color 140ms ease, background 140ms ease, color 140ms ease, box-shadow 140ms ease;
    }
    button:hover, .tab:hover {
      border-color: color-mix(in srgb, var(--accent) 40%, var(--line));
      color: var(--text);
      background: color-mix(in srgb, var(--color-sky-tint) 28%, transparent);
    }
    .tab.active {
      border-color: color-mix(in srgb, var(--apple-blue) 84%, #ffffff);
      color: #ffffff;
      background: linear-gradient(180deg, color-mix(in srgb, var(--apple-blue) 92%, #ffffff), var(--apple-blue));
      box-shadow: rgba(0, 122, 255, 0.24) 0px 4px 12px 0px, inset rgba(255, 255, 255, 0.24) 0px 1px 0px;
    }
    .tab.active:hover {
      color: #ffffff;
      background: linear-gradient(180deg, color-mix(in srgb, var(--apple-blue) 96%, #ffffff), color-mix(in srgb, var(--apple-blue) 86%, #000000));
    }
    .tab.is-on {
      border-color: color-mix(in srgb, var(--apple-blue) 80%, var(--line));
      color: #ffffff;
      background: var(--apple-blue);
      box-shadow: rgba(0, 122, 255, 0.2) 0px 3px 10px 0px;
    }
    .icon-tab {
      display: inline-grid;
      place-items: center;
      width: 34px;
      min-width: 34px;
      padding: 0;
      font-size: 17px;
      line-height: 1;
    }
    .icon-tab svg { width: 16px; height: 16px; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; fill: none; }
    .time {
      color: var(--muted);
      font-size: 12px;
      letter-spacing: 0;
      overflow: hidden;
      white-space: nowrap;
    }
    main {
      width: min(1180px, calc(100% - 24px));
      margin: 20px auto 32px;
    }
    .summary {
      display: grid;
      grid-template-columns: repeat(4, minmax(0, 1fr));
      gap: 12px;
      margin-bottom: 14px;
    }
    .metric {
      border: 1px solid var(--line-soft);
      border-radius: 10px;
      padding: 18px 20px;
      background: var(--surface);
      box-shadow: var(--shadow);
      min-width: 0;
    }
    .metric span { display: block; color: var(--muted); font-size: 12px; letter-spacing: 0; }
    .metric strong { display: block; margin-top: 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 20px; font-weight: 600; letter-spacing: 0; }
    .page { display: none; }
    .page.active { display: block; }
    .terminal-toolbar {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
      margin-bottom: 14px;
      padding: 16px 20px;
      border: 1px solid var(--line-soft);
      border-radius: 10px;
      background: var(--surface);
      box-shadow: var(--shadow);
    }
    .terminal-toolbar h2 { margin: 0; }
    .toolbar-meta { color: var(--muted); font-size: 12px; letter-spacing: 0; }
    .terminal-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
      gap: 12px;
    }
    .empty-row {
      padding: 24px;
      border: 1px dashed var(--line);
      border-radius: 10px;
      background: var(--surface);
      color: var(--muted);
      font-size: 12px;
      box-shadow: var(--shadow);
    }
    .terminal {
      min-height: 332px;
      border: 1px solid var(--line);
      border-radius: 10px;
      background: var(--terminal-bg);
      overflow: hidden;
      box-shadow: var(--shadow-subtle);
      backdrop-filter: blur(12px);
      transition: border-color 140ms ease, box-shadow 140ms ease, transform 140ms ease;
    }
    .terminal.idle { border-color: var(--line); }
    .terminal.free { border-color: var(--line); }
    .terminal.running {
      border-color: var(--line);
      box-shadow: var(--shadow-subtle);
    }
    .terminal.completed { border-color: var(--line); box-shadow: var(--shadow-subtle); }
    .terminal.failed, .terminal.missing {
      border-color: var(--line);
      box-shadow: var(--shadow-subtle);
    }
    .terminal-head {
      position: relative;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      min-height: 46px;
      padding: 12px 16px 12px 64px;
      border-bottom: 1px solid var(--terminal-line);
      color: var(--terminal-text);
      background: var(--terminal-panel);
    }
    .terminal-head::before {
      content: "";
      position: absolute;
      left: 16px;
      top: 18px;
      width: 8px;
      height: 8px;
      border-radius: 999px;
      background: #ff5f57;
      box-shadow: 14px 0 0 #ffbd2e, 28px 0 0 #28c840;
      opacity: 0.82;
    }
    .agent-title { min-width: 0; }
    .agent-title strong {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      max-width: 100%;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-size: 14px;
      font-weight: 600;
    }
    .agent-title .agent-id {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      color: var(--terminal-muted);
      font-weight: 500;
    }
    .badge {
      display: inline-flex;
      align-items: center;
      height: 22px;
      padding: 0 10px;
      border-radius: 999px;
      border: 1px solid var(--line);
      color: var(--terminal-muted);
      white-space: nowrap;
      font-size: 12px;
      background: transparent;
    }
    .badge.idle { color: #f0fdf4; border-color: #238652; background: #166534; }
    .badge.running { color: #ffffff; border-color: #0a84ff; background: #0a84ff; }
    .badge.completed { color: #f0fdf4; border-color: #238652; background: #166534; }
    .badge.failed, .badge.missing { color: #ffffff; border-color: #b91c1c; background: #991b1b; }
    .terminal-body {
      position: relative;
      padding: 14px 16px;
      height: 286px;
      overflow: auto;
      background: var(--terminal-bg);
      color: var(--terminal-text);
      scrollbar-color: #4b5563 #0b0f14;
      scrollbar-width: thin;
    }
    .terminal-body::-webkit-scrollbar { width: 10px; }
    .terminal-body::-webkit-scrollbar-track { background: #0b0f14; }
    .terminal-body::-webkit-scrollbar-thumb {
      border: 2px solid #0b0f14;
      border-radius: 999px;
      background: #4b5563;
    }
    .terminal-body::-webkit-scrollbar-thumb:hover { background: #64748b; }
    .line { margin: 0 0 6px; color: var(--terminal-text); white-space: pre-wrap; overflow-wrap: anywhere; font: 12px/1.55 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
    .line::before { content: "$ "; color: color-mix(in srgb, var(--accent) 82%, #ffffff); }
    .line .ts { color: var(--terminal-muted); }
    .line .ok { color: #82d994; }
    .line .run { color: #86cfff; }
    .line .warn { color: #f0c36b; }
    .line .bad { color: var(--red); }
    .line .dim { color: var(--terminal-muted); }
    .claude {
      display: grid;
      place-items: center;
      height: 164px;
      color: var(--terminal-muted);
      border: 1px dashed var(--terminal-line);
      border-radius: 8px;
      background: color-mix(in srgb, var(--terminal-panel) 66%, transparent);
      text-align: center;
    }
    .claude strong { display: block; font-size: 18px; color: var(--terminal-text); }
    .setup-grid { display: grid; grid-template-columns: minmax(300px, 520px) 1fr; gap: 12px; }
    .setup-card {
      border: 1px solid var(--line-soft);
      border-radius: 10px;
      padding: 24px;
      background: var(--surface);
      box-shadow: var(--shadow);
    }
    .setup-card h2 { margin: 0 0 12px; }
    .setup-status {
      display: grid;
      gap: 6px;
      margin: 0 0 14px;
      padding: 12px;
      border: 1px solid var(--line);
      border-radius: 10px;
      background: color-mix(in srgb, var(--surface-2) 78%, var(--surface));
    }
    .setup-status-row {
      display: flex;
      justify-content: space-between;
      gap: 16px;
      color: var(--muted);
      font-size: 12px;
    }
    .setup-status-row strong { color: var(--text); font-weight: 500; }
    .form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
    .form-grid .wide { grid-column: 1 / -1; }
    label { display: grid; gap: 5px; color: var(--muted); font-size: 12px; }
    input, select {
      height: 36px;
      border: 1px solid var(--color-platinum-outline);
      border-radius: 4px;
      padding: 0 12px;
      color: var(--text);
      background: var(--surface);
      font: inherit;
    }
    :root[data-theme="dark"] input,
    :root[data-theme="dark"] select,
    :root[data-theme="dark"] textarea {
      border-color: #3a3a3a;
      background: #202020;
    }
    input:focus, select:focus, textarea:focus {
      outline: 2px solid color-mix(in srgb, var(--accent) 28%, transparent);
      border-color: color-mix(in srgb, var(--accent) 66%, var(--line));
    }
    .profile-tabs { display: flex; gap: 8px; margin: 10px 0 12px; flex-wrap: wrap; }
    .profile-tab.active { border-color: var(--accent); color: var(--text); background: color-mix(in srgb, var(--color-sky-tint) 42%, transparent); }
    .prompt-grid { display: grid; grid-template-columns: minmax(300px, 360px) 1fr; gap: 12px; }
    .prompt-menu { display: grid; gap: 8px; align-content: start; }
    .prompt-menu button { width: 100%; justify-content: flex-start; text-align: left; border-radius: 10px; }
    .prompt-menu button.active { border-color: var(--accent); color: var(--text); background: color-mix(in srgb, var(--color-sky-tint) 42%, transparent); }
    .prompt-editor { display: grid; gap: 14px; margin-bottom: 18px; }
    .prompt-editor textarea { min-height: 0; font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
    .prompt-editor textarea.large { min-height: 0; }
    .prompt-note { margin: 0 0 12px; color: var(--muted); font-size: 12px; }
    .button-row {
      display: flex;
      gap: 8px;
      align-items: center;
      flex-wrap: wrap;
      clear: both;
      padding-top: 14px;
      border-top: 1px solid var(--line-soft);
    }
    .button-row .primary-action { width: auto; min-width: 120px; margin-top: 0; }
    .primary-action {
      width: 100%;
      height: 38px;
      margin-top: 12px;
      border-color: var(--accent);
      color: var(--color-cloud-white);
      background: var(--accent);
    }
    .primary-action:hover {
      color: var(--color-cloud-white);
      background: color-mix(in srgb, var(--accent) 86%, var(--color-slate-text));
      border-color: color-mix(in srgb, var(--accent) 86%, var(--color-slate-text));
    }
    .config-preview {
      min-height: 310px;
      margin: 0;
      overflow: auto;
      white-space: pre-wrap;
    }
    textarea {
      display: block;
      width: 100%;
      min-height: 0;
      resize: vertical;
      overflow: hidden;
      border: 1px solid var(--color-platinum-outline);
      border-radius: 4px;
      padding: 10px;
      color: var(--text);
      background: var(--surface);
      font: inherit;
    }
    .toast {
      position: fixed;
      right: 18px;
      bottom: 18px;
      z-index: 40;
      width: min(420px, calc(100% - 36px));
      padding: 14px 16px;
      border: 1px solid color-mix(in srgb, var(--accent) 38%, var(--line));
      border-radius: 10px;
      color: var(--text);
      background: color-mix(in srgb, var(--surface) 96%, transparent);
      box-shadow: var(--shadow-xl);
      backdrop-filter: blur(16px);
      opacity: 0;
      transform: translateY(8px);
      pointer-events: none;
      transition: opacity 160ms ease, transform 160ms ease;
    }
    .toast.show { opacity: 1; transform: translateY(0); }
    .toast strong { display: block; margin-bottom: 3px; font-size: 13px; font-weight: 600; }
    .toast span { display: block; color: var(--muted); font-size: 12px; line-height: 1.45; }
    code {
      display: block;
      overflow-wrap: anywhere;
      padding: 12px;
      border: 1px solid var(--line);
      border-radius: 10px;
      color: var(--text);
      background: color-mix(in srgb, var(--surface-2) 74%, var(--surface));
      font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    }
    @media (max-width: 760px) {
      .topbar {
        position: static;
        width: calc(100% - 24px);
        flex-direction: row;
        align-items: flex-start;
      }
      .tabs { display: flex; justify-content: flex-start; }
      main { width: calc(100% - 24px); margin: 12px; padding-right: 0; }
      .summary { grid-template-columns: repeat(2, minmax(0, 1fr)); }
      .setup-grid { grid-template-columns: 1fr; }
      .prompt-grid { grid-template-columns: 1fr; }
      .terminal-grid { grid-template-columns: 1fr; }
    }
  </style>
</head>
<body>
  <nav class="topbar">
    <div class="brand">
      <div class="mark">SD</div>
      <div class="brand-copy">
        <h1>SubDispatch</h1>
        <p id="updated" class="subtitle">加载中</p>
      </div>
    </div>
    <div class="tabs">
      <button class="tab active" data-page="activityPage">活动</button>
      <button class="tab" data-page="setupPage">配置</button>
      <button class="tab" data-page="promptsPage">提示词</button>
      <button class="tab icon-tab" id="inactiveToggle" aria-label="隐藏空闲终端" title="隐藏空闲终端"></button>
      <button class="tab icon-tab" id="cleanToggle" aria-label="清空终端显示" title="清空终端显示"></button>
      <button class="tab icon-tab" id="themeToggle" aria-label="切换颜色主题" title="切换颜色主题">☀</button>
    </div>
  </nav>

  <main>
    <section class="page active" id="activityPage">
      <section class="summary">
        <div class="metric"><span>运行中</span><strong id="runningCount">0</strong></div>
        <div class="metric"><span>空闲槽位</span><strong id="idleCount">0</strong></div>
        <div class="metric"><span>最近任务</span><strong id="recentTask">-</strong></div>
        <div class="metric"><span>Hook 事件</span><strong id="eventCount">0</strong></div>
      </section>
      <section class="terminal-toolbar">
        <div>
          <h2>代理终端</h2>
          <span class="toolbar-meta" id="terminalMeta">实时 child-agent hook 状态</span>
        </div>
      </section>
      <section class="terminal-grid" id="terminalWall"></section>
    </section>

    <section class="page" id="setupPage">
      <div class="setup-grid">
        <div class="setup-card">
          <h2>API 配置</h2>
          <div id="setup"></div>
          <div class="profile-tabs">
            <button class="profile-tab active" data-profile="glm">GLM</button>
            <button class="profile-tab" data-profile="minimax">MiniMax</button>
            <button class="profile-tab" data-profile="deepseek">DeepSeek</button>
          </div>
          <div class="form-grid">
            <label>Provider<input id="setupProvider" value="glm"></label>
            <label>Model<input id="setupModel" value="glm-5.1"></label>
            <label>最大并发数<input id="setupConcurrency" value="2"></label>
            <label>成本提示<select id="setupCost"><option>low</option><option selected>medium</option><option>high</option><option>unknown</option></select></label>
            <label>委派可信度<select id="setupDelegationTrust"><option>high</option><option selected>medium</option><option>low</option><option>experimental</option></select></label>
            <label class="wide">Anthropic 兼容 base URL<input id="setupBaseUrl" value="https://open.bigmodel.cn/api/anthropic"></label>
            <label class="wide">Auth token / API key<input id="setupToken" type="password" placeholder="粘贴新 key 以替换；留空则保留现有值"></label>
            <label class="wide">启动命令<input id="setupCommand" value="claude -p $prompt --permission-mode $permission_mode --output-format text"></label>
            <label class="wide">能力标签<input id="setupStrengths" value="general coding,reasoning,tests,documentation"></label>
          </div>
          <button id="applyProfile" class="primary-action">应用到 .env</button>
          <p id="envStatus" class="subtitle"></p>
        </div>
        <div class="setup-card">
          <h2>配置预览</h2>
          <code id="configPreview" class="config-preview"></code>
        </div>
      </div>
    </section>

    <section class="page" id="promptsPage">
      <div class="prompt-grid">
        <div class="setup-card">
          <h2>提示词控制台</h2>
          <p class="prompt-note" id="promptStatus">正在加载 prompt 配置</p>
          <div class="prompt-menu">
            <button class="prompt-tab active" data-prompt-section="mcp">MCP 工具</button>
            <button class="prompt-tab" data-prompt-section="child">子代理</button>
            <button class="prompt-tab" data-prompt-section="review">审查策略</button>
          </div>
        </div>
        <div class="setup-card">
          <div id="promptEditor" class="prompt-editor"></div>
          <div class="button-row">
            <button id="savePrompts" class="primary-action">保存提示词</button>
            <button id="resetPromptSection">恢复本节默认值</button>
          </div>
        </div>
      </div>
    </section>

  </main>
  <div id="toast" class="toast" role="status" aria-live="polite"></div>

  <script>
    const setupEl = document.getElementById('setup');
    const configPreview = document.getElementById('configPreview');
    const envStatus = document.getElementById('envStatus');
    const terminalWallEl = document.getElementById('terminalWall');
    const updatedEl = document.getElementById('updated');
    const promptEditor = document.getElementById('promptEditor');
    const promptStatus = document.getElementById('promptStatus');
    const toastEl = document.getElementById('toast');
    const logs = new Map();
    const lastSeen = new Map();
    const frozen = new Set();
    const hiddenTerminals = new Set();
    let hideInactive = false;
    let currentEnv = '';
    let promptConfig = null;
    let promptDefaults = null;
    let activePromptSection = 'mcp';
    let activeProfile = 'glm';
    const profileDefaults = {
      glm: {
        provider: 'glm',
        model: 'glm-5.1',
        concurrency: '2',
        baseUrl: 'https://open.bigmodel.cn/api/anthropic',
        cost: 'medium',
        delegationTrust: 'high',
        strengths: 'general coding,Chinese context,reasoning,tests,documentation'
      },
      minimax: {
        provider: 'minimax',
        model: 'MiniMax-M2.7-highspeed',
        concurrency: '3',
        baseUrl: 'https://api.minimaxi.com/anthropic',
        cost: 'low',
        delegationTrust: 'high',
        strengths: 'parallel throughput,simple edits,documentation,code search,boilerplate'
      },
      deepseek: {
        provider: 'deepseek',
        model: 'deepseek-chat',
        concurrency: '2',
        baseUrl: 'https://api.deepseek.com/anthropic',
        cost: 'low',
        delegationTrust: 'medium',
        strengths: 'code search,small refactors,tests,documentation'
      }
    };

    for (const tab of document.querySelectorAll('[data-page]')) {
      tab.addEventListener('click', () => showPage(tab.dataset.page));
    }
    renderInactiveToggle();
    document.getElementById('inactiveToggle').addEventListener('click', () => {
      hideInactive = !hideInactive;
      renderInactiveToggle();
      refresh(false);
    });
    renderCleanButton();
    document.getElementById('cleanToggle').addEventListener('click', () => {
      clearTerminalDisplay();
      refresh(false);
    });
    document.getElementById('themeToggle').addEventListener('click', toggleTheme);
    for (const tab of document.querySelectorAll('[data-profile]')) {
      tab.addEventListener('click', () => selectProfile(tab.dataset.profile));
    }
    for (const tab of document.querySelectorAll('[data-prompt-section]')) {
      tab.addEventListener('click', () => selectPromptSection(tab.dataset.promptSection));
    }
    for (const id of ['setupProvider', 'setupModel', 'setupConcurrency', 'setupBaseUrl', 'setupToken', 'setupCommand', 'setupStrengths', 'setupCost', 'setupDelegationTrust']) {
      document.getElementById(id).addEventListener('input', updateConfigPreview);
      document.getElementById(id).addEventListener('change', updateConfigPreview);
    }
    document.getElementById('applyProfile').addEventListener('click', async () => {
      envStatus.textContent = '正在应用 profile';
      const nextEnv = mergeProfileIntoEnv(currentEnv, readProfileForm());
      const response = await fetch('/api/env', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text: nextEnv })
      });
      envStatus.textContent = response.ok ? '已应用并保存' : '应用失败';
      if (response.ok) {
        currentEnv = nextEnv;
        await refresh(true);
      }
    });
    document.getElementById('savePrompts').addEventListener('click', savePrompts);
    document.getElementById('resetPromptSection').addEventListener('click', () => {
      if (!promptConfig || !promptDefaults) return;
      copyPromptSection(promptDefaults, promptConfig, activePromptSection);
      renderPromptEditor();
      promptStatus.textContent = '已在本地恢复本节默认值，保存后生效';
    });

    function showPage(id) {
      for (const page of document.querySelectorAll('.page')) page.classList.toggle('active', page.id === id);
      for (const tab of document.querySelectorAll('[data-page]')) {
        tab.classList.toggle('active', tab.dataset.page === id);
      }
      if (id === 'promptsPage') {
        requestAnimationFrame(() => autosizeTextareas(promptEditor));
      }
    }
    function applyTheme(theme) {
      document.documentElement.dataset.theme = theme;
      localStorage.setItem('subdispatch-theme', theme);
      document.getElementById('themeToggle').textContent = theme === 'light' ? '☀' : '☾';
    }
    function toggleTheme() {
      applyTheme(document.documentElement.dataset.theme === 'light' ? 'dark' : 'light');
    }
    function eyeIcon(hidden) {
      return hidden
        ? '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 3l18 18"/><path d="M10.6 10.6a2 2 0 0 0 2.8 2.8"/><path d="M9.9 4.2A10.8 10.8 0 0 1 12 4c5.5 0 9 5 9 8a8.2 8.2 0 0 1-2.1 3.7"/><path d="M6.6 6.6C4.3 8 3 10.2 3 12c0 3 3.5 8 9 8a10 10 0 0 0 4.1-.9"/></svg>'
        : '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2.5 12s3.5-7 9.5-7 9.5 7 9.5 7-3.5 7-9.5 7-9.5-7-9.5-7Z"/><circle cx="12" cy="12" r="3"/></svg>';
    }
    function renderInactiveToggle() {
      const button = document.getElementById('inactiveToggle');
      button.innerHTML = eyeIcon(hideInactive);
      button.classList.toggle('is-on', hideInactive);
      button.setAttribute('aria-label', hideInactive ? '显示空闲终端' : '隐藏空闲终端');
      button.setAttribute('title', hideInactive ? '显示空闲终端' : '隐藏空闲终端');
    }
    function trashIcon() {
      return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="M6 6l1 15h10l1-15"/><path d="M10 11v6"/><path d="M14 11v6"/></svg>';
    }
    function renderCleanButton() {
      const button = document.getElementById('cleanToggle');
      button.innerHTML = trashIcon();
    }
    function clearTerminalDisplay() {
      for (const key of logs.keys()) hiddenTerminals.add(key);
      logs.clear();
      lastSeen.clear();
      frozen.clear();
      hideInactive = false;
      renderInactiveToggle();
      document.getElementById('eventCount').textContent = '0';
    }
    function esc(value) {
      return String(value ?? '').replace(/[&<>"']/g, ch => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
    }
    function envKey(value) {
      return String(value || '').replace(/[^a-zA-Z0-9]/g, '_').toUpperCase();
    }
    function parseEnv(text) {
      const map = new Map();
      for (const line of String(text || '').split('\n')) {
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith('#') || !trimmed.includes('=')) continue;
        const [key, ...rest] = trimmed.split('=');
        map.set(key.trim(), rest.join('=').trim());
      }
      return map;
    }
    function readProfileForm() {
      return {
        provider: document.getElementById('setupProvider').value.trim(),
        model: document.getElementById('setupModel').value.trim(),
        concurrency: document.getElementById('setupConcurrency').value.trim() || '1',
        baseUrl: document.getElementById('setupBaseUrl').value.trim(),
        token: document.getElementById('setupToken').value.trim(),
        command: document.getElementById('setupCommand').value.trim(),
        strengths: document.getElementById('setupStrengths').value.trim(),
        cost: document.getElementById('setupCost').value,
        delegationTrust: document.getElementById('setupDelegationTrust').value
      };
    }
    function ensurePromptConfig(config = {}) {
      return {
        mcp: {
          list_workers: config.mcp?.list_workers || '',
          start_task: config.mcp?.start_task || '',
          poll_tasks: config.mcp?.poll_tasks || '',
          collect_task: config.mcp?.collect_task || '',
          delete_worktree: config.mcp?.delete_worktree || ''
        },
        child: {
          template: config.child?.template || '',
          manifest_schema: config.child?.manifest_schema || '',
          safety_rules: config.child?.safety_rules || ''
        },
        review: {
          collect_guidance: config.review?.collect_guidance || '',
          worker_selection: config.review?.worker_selection || ''
        }
      };
    }
    function selectPromptSection(section) {
      activePromptSection = section;
      for (const tab of document.querySelectorAll('[data-prompt-section]')) {
        tab.classList.toggle('active', tab.dataset.promptSection === section);
      }
      renderPromptEditor();
    }
    function promptField(label, path, options = {}) {
      const id = `prompt_${path.join('_')}`;
      const value = getPath(promptConfig, path) || '';
      const cls = options.large ? ' class="large"' : '';
      return `<label class="wide">${esc(label)}<textarea id="${esc(id)}" data-prompt-path="${esc(path.join('.'))}"${cls}>${esc(value)}</textarea></label>`;
    }
    function renderPromptEditor() {
      if (!promptConfig) {
        promptEditor.innerHTML = '<p class="prompt-note">Prompt 配置加载中。</p>';
        return;
      }
      if (activePromptSection === 'mcp') {
        promptEditor.innerHTML = `
          <p class="prompt-note">这些描述会通过 MCP tools/list 暴露给客户端。MCP client 可能需要重新加载 tools，或重启 MCP session 后才会看到变化。</p>
          ${promptField('list_workers', ['mcp', 'list_workers'])}
          ${promptField('start_task', ['mcp', 'start_task'], { large: true })}
          ${promptField('poll_tasks', ['mcp', 'poll_tasks'])}
          ${promptField('collect_task', ['mcp', 'collect_task'])}
          ${promptField('delete_worktree', ['mcp', 'delete_worktree'])}
        `;
      } else if (activePromptSection === 'child') {
        promptEditor.innerHTML = `
          <p class="prompt-note">Child template 支持 {{goal}}, {{instruction}}, {{read_scope}}, {{write_scope}}, {{forbidden_paths}}, {{result_path}}, {{manifest_schema}}, {{safety_rules}} 和 {{context_block}}。</p>
          ${promptField('子代理任务模板', ['child', 'template'], { large: true })}
          ${promptField('Manifest schema 提示词', ['child', 'manifest_schema'])}
          ${promptField('安全规则', ['child', 'safety_rules'])}
        `;
      } else if (activePromptSection === 'review') {
        promptEditor.innerHTML = `
          <p class="prompt-note">Setup/.env 是 worker metadata 的唯一事实源。这里仅编辑选择策略和证据审查原则。</p>
          ${promptField('Worker 选择策略', ['review', 'worker_selection'], { large: true })}
          ${promptField('回收与审查策略', ['review', 'collect_guidance'], { large: true })}
        `;
      }
      for (const field of promptEditor.querySelectorAll('[data-prompt-path]')) {
        field.addEventListener('input', () => {
          setPath(promptConfig, field.dataset.promptPath.split('.'), field.value);
          autosizeTextarea(field);
        });
      }
      autosizeTextareas(promptEditor);
    }
    function autosizeTextarea(field) {
      field.style.height = 'auto';
      field.style.height = `${Math.max(field.scrollHeight + 2, 42)}px`;
    }
    function autosizeTextareas(scope = document) {
      for (const field of scope.querySelectorAll('textarea')) autosizeTextarea(field);
    }
    function showToast(title, message) {
      toastEl.innerHTML = `<strong>${esc(title)}</strong><span>${esc(message)}</span>`;
      toastEl.classList.add('show');
      window.clearTimeout(showToast.timer);
      showToast.timer = window.setTimeout(() => toastEl.classList.remove('show'), 6200);
    }
    function getPath(object, path) {
      return path.reduce((value, key) => value && value[key], object);
    }
    function setPath(object, path, value) {
      let target = object;
      for (const key of path.slice(0, -1)) target = target[key] ||= {};
      target[path[path.length - 1]] = value;
    }
    function copyPromptSection(source, target, section) {
      target[section] = JSON.parse(JSON.stringify(source[section] || {}));
    }
    async function savePrompts() {
      if (!promptConfig) return;
      promptStatus.textContent = '正在保存 prompts';
      const response = await fetch('/api/prompts', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ config: promptConfig })
      });
      promptStatus.textContent = response.ok ? '已保存，将应用于新的 tool listings 和新任务' : '保存失败';
      if (response.ok) {
        showToast('Prompts 已保存', 'MCP descriptions 会在 MCP client 重新加载 tools 或重启 MCP session 后生效。Child templates 和 review guidance 会应用到新启动的任务。');
        await loadPrompts();
      } else {
        showToast('保存失败', 'Prompt 配置未保存。请检查字段内容后重试。');
      }
    }
    async function loadPrompts() {
      const response = await fetch('/api/prompts');
      const data = await response.json();
      promptConfig = ensurePromptConfig(data.config || {});
      promptDefaults = ensurePromptConfig(data.defaults || {});
      promptStatus.textContent = data.exists ? `已加载 ${data.path}` : '正在使用内置默认值';
      renderPromptEditor();
    }
    function selectProfile(profile) {
      activeProfile = profile;
      for (const tab of document.querySelectorAll('[data-profile]')) {
        tab.classList.toggle('active', tab.dataset.profile === profile);
      }
      loadProfileIntoForm(profile);
      updateConfigPreview();
    }
    function loadProfileIntoForm(profile) {
      const defaults = profileDefaults[profile] || profileDefaults.glm;
      const env = parseEnv(currentEnv);
      const provider = defaults.provider;
      const key = envKey(provider);
      document.getElementById('setupProvider').value = provider;
      document.getElementById('setupModel').value = env.get(`SUBDISPATCH_WORKER_${key}_MODEL`) || defaults.model;
      document.getElementById('setupConcurrency').value = env.get(`SUBDISPATCH_WORKER_${key}_MAX_CONCURRENCY`) || defaults.concurrency;
      document.getElementById('setupBaseUrl').value = env.get(`SUBDISPATCH_WORKER_${key}_ENV_ANTHROPIC_BASE_URL`) || defaults.baseUrl;
      document.getElementById('setupToken').value = '';
      document.getElementById('setupCommand').value = env.get(`SUBDISPATCH_WORKER_${key}_COMMAND`) || 'claude -p $prompt --permission-mode $permission_mode --output-format text';
      document.getElementById('setupStrengths').value = env.get(`SUBDISPATCH_WORKER_${key}_STRENGTHS`) || defaults.strengths;
      document.getElementById('setupCost').value = env.get(`SUBDISPATCH_WORKER_${key}_COST`) || defaults.cost;
      document.getElementById('setupDelegationTrust').value = env.get(`SUBDISPATCH_WORKER_${key}_DELEGATION_TRUST`) || defaults.delegationTrust;
      envStatus.textContent = env.get(`SUBDISPATCH_WORKER_${key}_MODEL`) ? `已加载 ${provider}` : `预设 ${provider}`;
    }
    function profileLines(profile, existingEnv) {
      const key = envKey(profile.provider);
      const existing = parseEnv(existingEnv);
      const token = profile.token || existing.get(`SUBDISPATCH_WORKER_${key}_ENV_ANTHROPIC_AUTH_TOKEN`) || existing.get(`SUBDISPATCH_WORKER_${key}_ENV_ANTHROPIC_API_KEY`) || '';
      return [
        `SUBDISPATCH_WORKER_${key}_ENABLED=1`,
        `SUBDISPATCH_WORKER_${key}_MODEL=${profile.model}`,
        `SUBDISPATCH_WORKER_${key}_MAX_CONCURRENCY=${profile.concurrency}`,
        `SUBDISPATCH_WORKER_${key}_DESCRIPTION=${profile.provider} Claude Code worker.`,
        `SUBDISPATCH_WORKER_${key}_STRENGTHS=${profile.strengths}`,
        `SUBDISPATCH_WORKER_${key}_COST=${profile.cost}`,
        `SUBDISPATCH_WORKER_${key}_DELEGATION_TRUST=${profile.delegationTrust}`,
        `SUBDISPATCH_WORKER_${key}_SPEED=unknown`,
        `SUBDISPATCH_WORKER_${key}_PERMISSION_MODE=bypassPermissions`,
        `SUBDISPATCH_WORKER_${key}_COMMAND=${profile.command}`,
        `SUBDISPATCH_WORKER_${key}_ENV_ANTHROPIC_BASE_URL=${profile.baseUrl}`,
        `SUBDISPATCH_WORKER_${key}_ENV_ANTHROPIC_AUTH_TOKEN=${token}`,
        `SUBDISPATCH_WORKER_${key}_ENV_ANTHROPIC_MODEL=${profile.model}`,
        `SUBDISPATCH_WORKER_${key}_ENV_ANTHROPIC_DEFAULT_SONNET_MODEL=${profile.model}`,
        `SUBDISPATCH_WORKER_${key}_ENV_ANTHROPIC_DEFAULT_OPUS_MODEL=${profile.model}`,
        `SUBDISPATCH_WORKER_${key}_ENV_ANTHROPIC_DEFAULT_HAIKU_MODEL=${profile.model}`
      ];
    }
    function redactSecret(value) {
      if (!value) return '';
      if (String(value).length <= 8) return '********';
      return `${String(value).slice(0, 4)}...${String(value).slice(-4)}`;
    }
    function profilePreviewLines(profile, existingEnv) {
      return profileLines(profile, existingEnv).map(line => {
        if (line.startsWith(`SUBDISPATCH_WORKER_${envKey(profile.provider)}_ENV_ANTHROPIC_AUTH_TOKEN=`)) {
          const [key, value] = line.split('=');
          return `${key}=${redactSecret(value)}`;
        }
        return line;
      });
    }
    function mergeProfileIntoEnv(envText, profile) {
      const provider = profile.provider || activeProfile;
      const key = envKey(provider);
      const workers = new Set((parseEnv(envText).get('SUBDISPATCH_WORKERS') || '').split(',').map(item => item.trim()).filter(Boolean));
      workers.add(provider);
      const prefix = `SUBDISPATCH_WORKER_${key}_`;
      const kept = String(envText || '')
        .split('\n')
        .filter(line => {
          const trimmed = line.trim();
          return !(trimmed.startsWith(`${prefix}`) || trimmed.startsWith('SUBDISPATCH_WORKERS='));
        })
        .filter((line, index, arr) => !(line.trim() === '' && arr[index - 1]?.trim() === ''));
      return [
        ...kept.filter(line => line.trim() !== ''),
        `SUBDISPATCH_WORKERS=${Array.from(workers).join(',')}`,
        ...profileLines({ ...profile, provider }, envText),
        ''
      ].join('\n');
    }
    function updateConfigPreview() {
      configPreview.textContent = profilePreviewLines(readProfileForm(), currentEnv).join('\n');
    }
    function nowTime() {
      return new Date().toLocaleTimeString();
    }
    function taskKey(task) {
      return task.id;
    }
    function addLog(key, html) {
      const list = logs.get(key) || [];
      list.push(`<p class="line"><span class="ts">${nowTime()}</span> ${html}</p>`);
      logs.set(key, list.slice(-14));
    }
    function addLogBlock(key, title, text) {
      const clean = compactText(text, 760);
      if (!clean) return;
      addLog(key, `<span class="dim">${esc(title)}</span> ${esc(clean)}`);
    }
    function compactText(text, limit = 520) {
      const clean = String(text || '').replace(/\s+/g, ' ').trim();
      if (!clean) return '';
      if (clean.length <= limit) return clean;
      return `${clean.slice(0, limit - 1)}…`;
    }
    function shortCommand(command) {
      return compactText(String(command || '').replace(/\n+/g, ' && '), 180);
    }
    function eventFile(event) {
      return event?.file_path ? shortPath(event.file_path) : '';
    }
    function eventLine(event) {
      const name = event?.hook_event_name || '-';
      const tool = event?.tool_name || '-';
      const file = eventFile(event);
      const parts = [`event=${name}`];
      if (tool && tool !== '-') parts.push(`tool=${tool}`);
      if (file) parts.push(`file=${file}`);
      if (event?.command) parts.push(`cmd="${shortCommand(event.command)}"`);
      if (event?.duration_ms != null) parts.push(`duration=${event.duration_ms}ms`);
      if (event?.stdout_tail) parts.push(`stdout="${compactText(event.stdout_tail, 180)}"`);
      if (event?.stderr_tail) parts.push(`stderr="${compactText(event.stderr_tail, 180)}"`);
      if (event?.last_assistant_message_tail) parts.push(`assistant="${compactText(event.last_assistant_message_tail, 220)}"`);
      return parts.join(' ');
    }
    function latestHookLine(task) {
      const event = task.last_event_name || '-';
      const tool = task.last_tool_name || '-';
      const file = task.last_file_path ? shortPath(task.last_file_path) : '';
      const parts = [`status=${task.status || '-'}`];
      if (event && event !== '-') parts.push(`event=${event}`);
      if (tool && tool !== '-') parts.push(`tool=${tool}`);
      if (file) parts.push(`file=${file}`);
      return parts.join(' ');
    }
    function taskEvidenceLine(task) {
      const parts = [];
      parts.push(`files=${task.changed_files_count || 0}`);
      parts.push(task.manifest_exists ? 'manifest=yes' : 'manifest=no');
      parts.push(task.patch_exists ? 'patch=yes' : 'patch=no');
      if (task.runtime_seconds != null) parts.push(`runtime=${task.runtime_seconds}s`);
      if (task.status === 'running' && task.idle_seconds != null) parts.push(`idle=${task.idle_seconds}s`);
      return parts.join(' ');
    }
    function observeTask(task) {
      const key = taskKey(task);
      if (frozen.has(key)) return;
      const snapshot = JSON.stringify({
        status: task.status,
        events: task.event_count,
        tool: task.last_tool_name,
        files: task.changed_files_count,
        event: task.last_event_name,
        file: task.last_file_path,
        idle: task.idle_seconds,
        manifest: task.manifest_exists,
        patch: task.patch_exists,
        assistant: task.last_assistant_message_tail,
        recent: task.recent_events
      });
      if (!lastSeen.has(key)) {
        addLog(key, `<span class="run">start</span> ${esc(task.id)} <span class="dim">on</span> ${esc(providerLabel(task.worker))}`);
        addLogBlock(key, 'goal', task.goal || task.instruction || '');
        addLog(key, `<span class="dim">branch</span> ${esc(task.branch || '-')} <span class="dim">worktree</span> ${esc(shortPath(task.worktree || '-'))}`);
        for (const event of (task.recent_events || []).slice(-6)) {
          addLog(key, `<span class="dim">hook</span> ${esc(eventLine(event))}`);
        }
      } else if (lastSeen.get(key) !== snapshot) {
        const cls = task.status === 'running' ? 'run' : task.status === 'completed' ? 'ok' : task.status === 'failed' ? 'bad' : 'warn';
        addLog(key, `<span class="${cls}">${esc(task.status)}</span> ${esc(latestHookLine(task))} <span class="dim">${esc(taskEvidenceLine(task))}</span>`);
        if (task.status !== 'completed' && task.status !== 'failed' && task.last_assistant_message_tail) {
          addLogBlock(key, 'agent', task.last_assistant_message_tail);
        }
      }
      if ((task.status === 'completed' || task.status === 'failed') && !String(lastSeen.get(key) || '').includes(`"final":"${task.status}"`)) {
        const cls = task.status === 'completed' ? 'ok' : 'bad';
        addLog(key, `<span class="${cls}">end</span> exit=${esc(task.exit_code ?? 0)} ${esc(taskEvidenceLine(task))}`);
        addLogBlock(key, 'final', task.last_assistant_message_tail || task.error || '');
        frozen.add(key);
      }
      lastSeen.set(key, snapshot + ((task.status === 'completed' || task.status === 'failed') ? `,"final":"${task.status}"` : ''));
    }
    function sortTasks(tasks) {
      return [...tasks].sort((a, b) => (b.created_at || 0) - (a.created_at || 0));
    }
    function renderTerminals(snapshot) {
      const allTasks = sortTasks(snapshot.tasks || []);
      for (const task of allTasks) observeTask(task);

      const running = allTasks.filter(task => task.status === 'running');
      const maxTerminals = Math.max(totalWorkerSlots(snapshot.workers || []), running.length, 1);
      const latestTaskId = allTasks[0]?.id;
      const recentCompleted = allTasks
        .filter(task => task.status !== 'running')
        .slice(0, 6);
      const taskTerminals = [...running, ...recentCompleted]
        .filter((task, index, arr) => arr.findIndex(other => taskKey(other) === taskKey(task)) === index)
        .filter(task => task.status === 'running' || !hiddenTerminals.has(taskKey(task)))
        .filter(task => (logs.get(taskKey(task)) || []).length > 0)
        .filter(task => !hideInactive || task.status === 'running')
        .slice(0, maxTerminals);
      const freeSlotsNeeded = Math.max(maxTerminals - taskTerminals.length, 0);
      const freeTerminals = hideInactive ? [] : makeFreeTerminals(snapshot.workers || [], running).slice(0, freeSlotsNeeded);
      const rendered = [
        ...taskTerminals.map((task) => renderTaskTerminal(task)),
        ...freeTerminals.map((item) => renderFreeTerminal(item))
      ].join('');
      terminalWallEl.innerHTML = rendered || '<p class="empty-row">暂无活跃终端输出</p>';
      stickTerminalBodiesToBottom();

      document.getElementById('runningCount').textContent = running.length;
      document.getElementById('idleCount').textContent = (snapshot.workers || []).reduce((sum, worker) => sum + (worker.available_slots || 0), 0);
      document.getElementById('recentTask').textContent = latestTaskId || '-';
      document.getElementById('eventCount').textContent = taskTerminals.reduce((sum, task) => sum + (task.event_count || 0), 0);
      document.getElementById('terminalMeta').textContent = hideInactive ? '仅显示运行中的终端' : '显示最近终端和空闲 slots';
    }
    function totalWorkerSlots(workers) {
      return workers.reduce((sum, worker) => sum + (worker.max_concurrency || 0), 0);
    }
    function stickTerminalBodiesToBottom() {
      for (const body of document.querySelectorAll('.terminal-body')) {
        body.scrollTop = body.scrollHeight;
      }
    }
    function makeFreeTerminals(workers, runningTasks) {
      const activeByWorker = runningTasks.reduce((acc, item) => {
        acc[item.worker] = (acc[item.worker] || 0) + 1;
        return acc;
      }, {});
      const free = [];
      for (const worker of workers) {
        const slots = Math.max((worker.max_concurrency || 0) - (activeByWorker[worker.id] || 0), 0);
        for (let slot = 1; slot <= slots; slot++) free.push({ worker, slot });
      }
      return free;
    }
    function renderTaskTerminal(task) {
      const key = taskKey(task);
      const lines = logs.get(key) || [];
      const status = task.status || 'missing';
      const provider = providerLabel(task.worker);
      const shortId = shortTaskId(task.id);
      const idle = status === 'running' && task.idle_seconds != null ? ` idle=${esc(task.idle_seconds)}s` : '';
      const latest = `<p class="line"><span class="dim">now</span> ${esc(latestHookLine(task))} <span class="dim">hook=${task.event_count || 0}${idle} ${esc(taskEvidenceLine(task))}</span></p>`;
      const taskLine = `<p class="line"><span class="dim">task</span> ${esc(compactText(task.goal || task.instruction || task.id, 220))}</p>`;
      return `
        <article class="terminal ${esc(status)}">
          <div class="terminal-head">
            <div class="agent-title">
              <strong>${esc(provider)} <span class="agent-id">· ${esc(shortId)}</span></strong>
            </div>
            <span class="badge ${esc(status)}">${esc(status)}</span>
          </div>
          <div class="terminal-body">
            ${latest}
            ${taskLine}
            ${lines.join('')}
          </div>
        </article>
      `;
    }
    function renderFreeTerminal(item) {
      const provider = providerLabel(item.worker.id);
      return `
        <article class="terminal free">
          <div class="terminal-head">
            <div class="agent-title">
              <strong>${esc(provider)} <span class="agent-id">· slot ${esc(item.slot)}</span></strong>
            </div>
            <span class="badge idle">free</span>
          </div>
          <div class="terminal-body">
            <div class="claude">
              <div>
                <strong>Claude Code</strong>
                <span>已准备好接收委派任务</span>
              </div>
            </div>
          </div>
        </article>
      `;
    }
    function providerLabel(workerId) {
      const value = String(workerId || 'worker');
      if (value.toLowerCase() === 'glm') return 'GLM';
      if (value.toLowerCase() === 'minimax') return 'MiniMax';
      if (value.toLowerCase() === 'deepseek') return 'DeepSeek';
      return value.charAt(0).toUpperCase() + value.slice(1);
    }
    function shortTaskId(taskId) {
      const value = String(taskId || 'task');
      if (value.length <= 26) return value;
      return `${value.slice(0, 18)}...${value.slice(-5)}`;
    }
    function shortPath(path) {
      const value = String(path || '');
      const marker = '/.subdispatch/worktrees/tasks/';
      const index = value.indexOf(marker);
      if (index >= 0) return `tasks/${value.slice(index + marker.length)}`;
      if (value.length <= 54) return value;
      return `...${value.slice(-51)}`;
    }
    function renderSetup(data) {
      const checks = data.checks || {};
      setupEl.innerHTML = `
        <div class="setup-status">
          <div class="setup-status-row"><span>状态</span><strong>${esc(data.status)}</strong></div>
          <div class="setup-status-row"><span>.env</span><strong>${checks.env_exists ? '已存在' : '缺失'}</strong></div>
          <div class="setup-status-row"><span>Git</span><strong>${checks.git_available && checks.git_repo ? '就绪' : '缺失'}</strong></div>
          <div class="setup-status-row"><span>Claude</span><strong>${checks.claude_available ? '可用' : '缺失'}</strong></div>
          <div class="setup-status-row"><span>MCP</span><strong>${checks.project_mcp_installed || checks.global_mcp_installed ? '已安装' : '未安装'}</strong></div>
        </div>
        <code>${esc(data.next_step || '')}</code>
      `;
    }
    async function refresh(loadEnv = false) {
      const requests = [
        fetch('/api/setup').then(r => r.json()),
        fetch('/api/snapshot').then(r => r.json())
      ];
      if (loadEnv) requests.push(fetch('/api/env').then(r => r.json()));
      const [setup, snapshot, env] = await Promise.all(requests);
      renderSetup(setup);
      renderTerminals(snapshot);
      if (env) {
        currentEnv = env.text || '';
        envStatus.textContent = env.env_path || '';
        loadProfileIntoForm(activeProfile);
        updateConfigPreview();
      }
      updatedEl.textContent = `更新于 ${nowTime()}`;
    }
    applyTheme(localStorage.getItem('subdispatch-theme') || 'light');
    loadPrompts();
    refresh(true);
    setInterval(() => refresh(false), 700);
  </script>
</body>
</html>
"#;
