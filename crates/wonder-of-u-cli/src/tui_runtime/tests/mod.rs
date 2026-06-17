use super::*;

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    thread,
};

use serde_json::{Value, json};
use wonder_of_u_agent::{
    AgentSettings, AuthMaterial, CredentialStore, SettingsStore, StoredCredentials,
};
use wonder_of_u_core::{
    AuthState, FleetId, FleetRunState, FleetRunStatus, InputMode, MessageEnvelope, MessagePayload,
    PendingLocalToolCall, PendingProviderToolCall, PendingToolApprovalState,
    PendingToolConversationRound, PermissionMode, TodoTaskEntry, TodoTaskList, TodoTaskStatus,
    TokenUsage,
};
use wonder_of_u_storage::{FleetStore, TodoTaskStore};
use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};
use wonder_of_u_tui::KeyModifiers;

use crate::commands;

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(f)
}

/// Pumps the controller's non-blocking `ActiveTurn` until it settles: the
/// turn finished (`Completed`/`Interrupted`), paused for a permission or
/// interaction decision, or the poll budget is exhausted (panics so the test
/// fails loudly instead of hanging on a provider-server `join`).
///
/// Each `poll_active_turn` runs inside its own tokio runtime via `block_on`;
/// dropping that runtime waits for the `spawn_blocking` HTTP worker, so the
/// next poll observes the provider response deterministically.
fn pump_active_turn(controller: &mut TuiController<'_>) {
    for _ in 0..10_000 {
        if !controller.has_active_turn()
            || matches!(controller.turn_state, TurnState::ToolPermissionPending)
        {
            return;
        }
        block_on(controller.poll_active_turn()).expect("poll active turn");
        thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("active turn did not settle after 10000 polls");
}

/// Drains queued commands and pumps each resulting turn until the queue is
/// empty, mirroring what the real event loop does across ticks.
fn drain_and_pump(controller: &mut TuiController<'_>) {
    for _ in 0..100 {
        block_on(controller.drain_queued_commands()).expect("drain queued commands");
        pump_active_turn(controller);
        if matches!(controller.turn_state, TurnState::ToolPermissionPending) {
            return;
        }
        if controller.state.queued_commands.is_empty() && !controller.has_active_turn() {
            return;
        }
    }
    panic!("queued commands did not drain");
}

include!("chunk_0.rs");
include!("chunk_1.rs");
include!("chunk_2.rs");
include!("chunk_3.rs");
include!("chunk_4.rs");
include!("chunk_5.rs");
