use crate::config;
use crate::engine::SubDispatchEngine;
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
    let mut buffer = [0_u8; 8192];
    let size = stream
        .read(&mut buffer)
        .map_err(|err| format!("failed to read HTTP request: {err}"))?;
    let request = String::from_utf8_lossy(&buffer[..size]);
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
            let setup = setup_snapshot(&workspace)?;
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
        _ => write_response(&mut stream, 404, "text/plain; charset=utf-8", "Not found"),
    }
}

fn setup_snapshot(workspace: &PathBuf) -> Result<serde_json::Value, String> {
    let env_path = workspace.join(".env");
    Ok(json!({
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "env_exists": env_path.exists(),
        "env_path": env_path.display().to_string(),
        "git_available": command_available("git"),
        "claude_available": command_available("claude"),
        "mcp_command": format!("subdispatch mcp --workspace {}", workspace.display()),
    }))
}

fn command_available(command: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|path| path.join(command))
                .find(|path| path.exists())
        })
        .is_some()
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
      --bg: #f7f8fa;
      --surface: #ffffff;
      --text: #1d2430;
      --muted: #647082;
      --border: #d8dee8;
      --accent: #0f766e;
      --warn: #b45309;
      --bad: #b91c1c;
      --good: #15803d;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      font: 14px/1.45 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      color: var(--text);
      background: var(--bg);
    }
    header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 18px 24px;
      border-bottom: 1px solid var(--border);
      background: var(--surface);
    }
    h1, h2, h3, p { margin: 0; }
    h1 { font-size: 18px; font-weight: 700; }
    h2 { font-size: 15px; margin-bottom: 12px; }
    h3 { font-size: 14px; }
    main {
      display: grid;
      grid-template-columns: minmax(280px, 360px) 1fr;
      gap: 16px;
      padding: 16px;
    }
    section, aside {
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 16px;
    }
    .stack { display: grid; gap: 12px; }
    .row {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
    }
    .muted { color: var(--muted); }
    .pill {
      display: inline-flex;
      align-items: center;
      height: 24px;
      padding: 0 8px;
      border-radius: 999px;
      border: 1px solid var(--border);
      color: var(--muted);
      background: #fbfcfe;
      font-size: 12px;
      white-space: nowrap;
    }
    .ok { color: var(--good); }
    .warn { color: var(--warn); }
    .bad { color: var(--bad); }
    button {
      border: 1px solid var(--border);
      border-radius: 6px;
      background: var(--surface);
      color: var(--text);
      height: 32px;
      padding: 0 10px;
      cursor: pointer;
    }
    button.primary {
      border-color: var(--accent);
      background: var(--accent);
      color: white;
    }
    code {
      display: block;
      overflow-wrap: anywhere;
      padding: 10px;
      border: 1px solid var(--border);
      border-radius: 6px;
      background: #f8fafc;
      color: #334155;
      font-size: 12px;
    }
    .worker, .task {
      display: grid;
      gap: 8px;
      padding: 12px 0;
      border-top: 1px solid var(--border);
    }
    .worker:first-child, .task:first-child { border-top: 0; padding-top: 0; }
    .grid {
      display: grid;
      grid-template-columns: repeat(4, minmax(0, 1fr));
      gap: 8px;
    }
    .metric {
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 8px;
      min-width: 0;
    }
    .metric strong { display: block; font-size: 16px; }
    @media (max-width: 820px) {
      main { grid-template-columns: 1fr; }
      .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    }
  </style>
</head>
<body>
  <header>
    <div>
      <h1>SubDispatch</h1>
      <p class="muted">Local child-agent activity and setup</p>
    </div>
    <span id="updated" class="pill">Loading</span>
  </header>
  <main>
    <aside class="stack">
      <section class="stack">
        <div class="row">
          <h2>Setup</h2>
          <button id="initEnv" class="primary">Init .env</button>
        </div>
        <div id="setup" class="stack"></div>
      </section>
      <section>
        <h2>Workers</h2>
        <div id="workers"></div>
      </section>
    </aside>
    <section>
      <h2>Activity</h2>
      <div id="runs" class="stack"></div>
    </section>
  </main>
  <script>
    const setupEl = document.getElementById('setup');
    const workersEl = document.getElementById('workers');
    const runsEl = document.getElementById('runs');
    const updatedEl = document.getElementById('updated');
    document.getElementById('initEnv').addEventListener('click', async () => {
      await fetch('/api/init-env', { method: 'POST' });
      await refresh();
    });
    function statusClass(value) {
      if (value === true || value === 'completed') return 'ok';
      if (value === false || value === 'failed' || value === 'missing') return 'bad';
      return 'warn';
    }
    function renderSetup(data) {
      setupEl.innerHTML = `
        <div class="row"><span>.env</span><span class="${statusClass(data.env_exists)}">${data.env_exists ? 'present' : 'missing'}</span></div>
        <div class="row"><span>git</span><span class="${statusClass(data.git_available)}">${data.git_available ? 'available' : 'missing'}</span></div>
        <div class="row"><span>claude</span><span class="${statusClass(data.claude_available)}">${data.claude_available ? 'available' : 'missing'}</span></div>
        <code>${data.mcp_command}</code>
      `;
    }
    function renderWorkers(workers) {
      workersEl.innerHTML = workers.map(worker => `
        <div class="worker">
          <div class="row"><h3>${worker.id}</h3><span class="pill">${worker.available_slots}/${worker.max_concurrency} free</span></div>
          <p class="muted">${worker.description || ''}</p>
          <div class="grid">
            <div class="metric"><span class="muted">Model</span><strong>${worker.model || '-'}</strong></div>
            <div class="metric"><span class="muted">Running</span><strong>${worker.running}</strong></div>
            <div class="metric"><span class="muted">Queued</span><strong>${worker.queued}</strong></div>
            <div class="metric"><span class="muted">Speed</span><strong>${worker.speed}</strong></div>
          </div>
          <p class="${worker.enabled ? 'ok' : 'bad'}">${worker.enabled ? 'enabled' : worker.unavailable_reason}</p>
        </div>
      `).join('') || '<p class="muted">No workers configured.</p>';
    }
    function renderRuns(runs) {
      runsEl.innerHTML = runs.map(run => `
        <div class="task">
          <div class="row"><h3>${run.id}</h3><span class="pill">${run.tasks.length} tasks</span></div>
          <p>${run.goal}</p>
          ${run.tasks.map(task => `
            <div class="task">
              <div class="row"><strong>${task.id}</strong><span class="${statusClass(task.status)}">${task.status}</span></div>
              <div class="grid">
                <div class="metric"><span class="muted">Worker</span><strong>${task.worker}</strong></div>
                <div class="metric"><span class="muted">Files</span><strong>${task.changed_files_count}</strong></div>
                <div class="metric"><span class="muted">Events</span><strong>${task.event_count}</strong></div>
                <div class="metric"><span class="muted">Tool</span><strong>${task.last_tool_name || '-'}</strong></div>
              </div>
              <p class="muted">${task.last_assistant_message_tail || ''}</p>
            </div>
          `).join('')}
        </div>
      `).join('') || '<p class="muted">No delegated runs yet.</p>';
    }
    async function refresh() {
      const [setup, snapshot] = await Promise.all([
        fetch('/api/setup').then(r => r.json()),
        fetch('/api/snapshot').then(r => r.json())
      ]);
      renderSetup(setup);
      renderWorkers(snapshot.workers || []);
      renderRuns(snapshot.runs || []);
      updatedEl.textContent = new Date().toLocaleTimeString();
    }
    refresh();
    setInterval(refresh, 1000);
  </script>
</body>
</html>
"#;
