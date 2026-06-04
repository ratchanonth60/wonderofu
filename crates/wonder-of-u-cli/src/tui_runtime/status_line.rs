//! Status line hook: runs a user-configurable shell command after each AI
//! response and displays its stdout as the footer bar text.
//!
//! The command receives a JSON object on stdin that mirrors the fields exposed
//! by claude-code's `StatusLineCommandInput` so existing status-line scripts
//! work without changes.

use std::{
    io::Write,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::Serialize;
use wonder_of_u_core::{AppState, permission_mode_label};

// ─── Public handle ────────────────────────────────────────────────────────────

/// Tracks the running state and latest output of the background status-line thread.
#[derive(Debug, Default)]
pub(super) struct StatusLineHandle {
    active: Arc<AtomicBool>,
    /// Last text returned by the command (`None` = no run yet / command absent).
    pub(super) output: Arc<Mutex<Option<String>>>,
}

impl StatusLineHandle {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Returns the last cached output, if any.
    pub(super) fn current_text(&self) -> Option<String> {
        self.output.lock().ok()?.clone()
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

/// Spawn a background thread to run the status line command if not already running.
pub(super) fn maybe_run_status_line(handle: &StatusLineHandle, command: &str, state: &AppState) {
    if handle.active.load(Ordering::Relaxed) {
        return;
    }
    handle.active.store(true, Ordering::Relaxed);

    let input = build_status_line_input(state);
    let cmd = command.to_owned();
    let active = Arc::clone(&handle.active);
    let output = Arc::clone(&handle.output);

    std::thread::spawn(move || {
        let text = run_command(&cmd, &input).unwrap_or_default();
        let text = text.trim().to_string();
        if let Ok(mut guard) = output.lock() {
            if !text.is_empty() {
                *guard = Some(text);
            }
        }
        active.store(false, Ordering::Relaxed);
    });
}

// ─── JSON input schema ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct StatusLineInput {
    model: ModelInfo,
    permission_mode: String,
    context_window: ContextWindowInfo,
    cost: CostInfo,
}

#[derive(Serialize)]
struct ModelInfo {
    id: String,
}

#[derive(Serialize)]
struct ContextWindowInfo {
    context_window_size: u64,
    current_usage: CurrentUsage,
    used_percentage: u64,
    remaining_percentage: u64,
    total_input_tokens: u64,
    total_output_tokens: u64,
}

#[derive(Serialize)]
struct CurrentUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
}

#[derive(Serialize)]
struct CostInfo {
    total_cost_usd: f64,
}

fn build_status_line_input(state: &AppState) -> StatusLineInput {
    let usage = &state.costs.usage;
    let window_size = state.context_window_size.unwrap_or(200_000);
    let total_used = usage.total_tokens();
    let used_pct = total_used
        .saturating_mul(100)
        .checked_div(window_size)
        .unwrap_or(0);

    StatusLineInput {
        model: ModelInfo {
            id: state.model.clone().unwrap_or_default(),
        },
        permission_mode: permission_mode_label(state.permission_mode).to_string(),
        context_window: ContextWindowInfo {
            context_window_size: window_size,
            current_usage: CurrentUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_creation_input_tokens: usage.cache_creation_tokens,
                cache_read_input_tokens: usage.cache_read_tokens,
            },
            used_percentage: used_pct,
            remaining_percentage: 100u64.saturating_sub(used_pct),
            total_input_tokens: usage.input_tokens,
            total_output_tokens: usage.output_tokens,
        },
        cost: CostInfo {
            total_cost_usd: state.costs.estimated_cost_usd.unwrap_or(0.0),
        },
    }
}

// ─── Shell execution ──────────────────────────────────────────────────────────

fn run_command(command: &str, input: &StatusLineInput) -> Option<String> {
    let json = serde_json::to_string(input).ok()?;

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    if let Some(stdin) = child.stdin.take() {
        let mut stdin = stdin;
        let _ = stdin.write_all(json.as_bytes());
    }

    let out = child.wait_with_output().ok()?;
    if out.status.success() {
        String::from_utf8(out.stdout).ok()
    } else {
        None
    }
}
