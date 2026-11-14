use std::{
    io::Write,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// Program names of coding agents, as Rex reports a pane's foreground process.
const AGENTS: &[&str] = &[
    "claude", "codex", "gemini", "opencode", "aider", "amp", "goose", "cursor-agent", "crush", "droid", "copilot", "qwen",
    "kiro-cli", "grok", "pi",
];

pub struct Agent {
    pub name: String,
    /// Where it is relative to tg: "this tab", "tab 2", or "session · tab 2".
    pub place: String,
    pub cwd: String,
    session: String,
    block: String,
}

fn rex(args: &[&str], input: Option<&str>) -> Result<Value> {
    let mut child = Command::new("rex")
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Rex isn't available")?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(input.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("rex {}: {}", args[0], String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(serde_json::from_slice(&out.stdout).unwrap_or(Value::Null))
}

/// Agents running in any Rex session: ones in tg's own tab first, then its session, then the rest.
pub fn discover() -> Result<Vec<Agent>> {
    let me = std::env::var("REX_BLOCK").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let sessions = rex(&["ls", "--json"], None)?;
    let mut agents = Vec::new();
    for session in sessions["sessions"].as_array().into_iter().flatten() {
        let id = session["session_id"].as_str().unwrap_or_default();
        let windows = rex(&["window", "list", "-s", id, "--json"], None)?;
        let windows = windows["windows"].as_array().cloned().unwrap_or_default();
        let blocks = rex(&["block", "list", "-s", id, "--json"], None)?;
        let blocks: Vec<&Value> = blocks["blocks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["flavor"].as_str().is_some_and(|f| f.contains("terminal")))
            .collect();
        let my_window = blocks.iter().find(|b| b["block_id"] == me.as_str()).map(|b| b["window_id"].clone());

        // One `rex` call per pane; run them side by side.
        let processes: Vec<Option<Value>> = thread::scope(|scope| {
            let handles: Vec<_> = blocks
                .iter()
                .map(|b| {
                    let block = b["block_id"].as_str().unwrap_or_default();
                    scope.spawn(move || rex(&["block", "call", "-s", id, "-b", block, "process"], None).ok())
                })
                .collect();
            handles.into_iter().map(|h| h.join().ok().flatten()).collect()
        });

        for (block, process) in blocks.iter().zip(processes) {
            let fg = &process.unwrap_or_default()["foreground"];
            let program = fg["argv0"].as_str().or(fg["name"].as_str()).unwrap_or_default();
            let name = program.rsplit('/').next().unwrap_or_default().trim_end_matches(".exe");
            if !AGENTS.contains(&name) {
                continue;
            }
            let here = my_window.as_ref() == Some(&block["window_id"]);
            let tab = windows.iter().position(|w| w["window_id"] == block["window_id"]).map_or(0, |i| i + 1);
            let label = windows.get(tab.wrapping_sub(1)).and_then(|w| w["label"].as_str()).unwrap_or_default();
            // Unnamed tabs are labeled "Window <id>"; a tab number reads better.
            let tab = if label.is_empty() || label.starts_with("Window ") { format!("tab {tab}") } else { label.to_string() };
            let place = match (here, my_window.is_some()) {
                (true, _) => "this tab".to_string(),
                (false, true) => tab,
                (false, false) => format!("{} · {tab}", session["label"].as_str().unwrap_or_default()),
            };
            let cwd = fg["cwd"].as_str().unwrap_or_default();
            agents.push((
                (!here, my_window.is_none()),
                Agent {
                    name: name.to_string(),
                    place,
                    cwd: cwd.strip_prefix(&home).map_or(cwd.to_string(), |rest| format!("~{rest}")),
                    session: id.to_string(),
                    block: block["block_id"].as_str().unwrap_or_default().to_string(),
                },
            ));
        }
    }
    agents.sort_by_key(|(rank, _)| *rank);
    Ok(agents.into_iter().map(|(_, agent)| agent).collect())
}

/// Pastes `text` into the agent's prompt and submits it.
pub fn send(agent: &Agent, text: &str) -> Result<()> {
    let write = |data: &str| {
        let args = json!({ "data": data }).to_string();
        rex(&["block", "call", "-s", &agent.session, "-b", &agent.block, "write", "-"], Some(&args))
    };
    // Bracketed paste keeps the newlines from submitting early; only wrap when the agent has it on.
    let modes = rex(&["block", "call", "-s", &agent.session, "-b", &agent.block, "modes"], None)?;
    let bracketed = modes["modes"].as_array().into_iter().flatten().any(|m| m["name"] == "bracketed_paste" && m["set"] == true);
    write(&if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text.to_string() })?;
    // Give the agent a moment to take the paste before pressing Enter.
    thread::sleep(Duration::from_millis(150));
    write("\r")?;
    Ok(())
}
