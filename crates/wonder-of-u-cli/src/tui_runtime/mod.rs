//! Provides tui runtime support
//!
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::mpsc,
    time::Duration,
};

use crossterm::{
    cursor::{Hide, Show},
    event::{
        EnableBracketedPaste, EnableMouseCapture, KeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::executor::block_on;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    prelude::{Color as RatatuiColor, Modifier, Style as RatatuiStyle},
};
use wonder_of_u_agent::{
    CompletionRequest, CredentialStore, ProviderRegistry, ProviderResolver, ProviderRuntime,
    ProviderSelection, ProviderToolCall, ProviderToolResultMessage, ProviderToolSpec,
    SettingsStore, ToolConversationRound, ToolUseRequest, ToolUseResponse,
    poll_copilot_access_token, request_copilot_device_code,
};
use wonder_of_u_core::{
    AdditionalWorkingDirectory, AppState, AuthMaterialKind, AuthState, CommandContext,
    CommandOutput, CommandQuery, CommandRegistry, FeatureSet, InputMode, MessageEnvelope,
    MessagePayload, PendingLocalToolCall, PendingProviderToolCall, PendingProviderToolResult,
    PendingToolApprovalState, PendingToolConversationRound, PermissionDecision, PermissionMode,
    PermissionRequest, PermissionRuleSource, ProviderReadiness, QueuePlacement, Result, SessionId,
    TaskState, TaskStatus, TodoTaskStatus, ToolContext, ToolKind, ToolQuery, ToolResult,
    ToolSource, ToolUseId, WonderError, parse_slash_command, payload_from_task_state,
    token_budget::{
        AUTOCOMPACT_BUFFER_TOKENS, MANUAL_COMPACT_BUFFER_TOKENS, effective_context_window,
    },
};
use wonder_of_u_mcp::McpConfigStore;
use wonder_of_u_storage::{TaskStore, TodoTaskStore, TranscriptStore};
use wonder_of_u_tools::provider_tool_specs;
use wonder_of_u_tui::{
    CrosstermEventSource, DialogActionView, DialogView, EditAction, EventLoop,
    GlobalSearchOverlayView, HistorySearchView, KeyBindingContext, KeyBindingResolver, KeyCode,
    KeyEvent, MouseEventKind, NotificationInput, NotificationLifetime, NotificationQueue,
    NotificationSeverity, PermissionSummaryView, PickerListEntry, PickerListView, PromptSuggestion,
    PromptSuggestionState, Rect, ResolvedKey, ShellLayout, ShellView, SlashSuggestionEntry,
    SlashSuggestionsOverlay, TextBuffer, Theme, TranscriptScrollView, TurnState, UiEvent, VimMode,
    VimState, message::SearchMatch, message_lines_for_width, shell_main_area_width,
};

use crate::commands;
use crate::commands::prompt::{
    SessionPersistenceState, append_contextual_message, build_fork_context_snapshot,
    load_or_create_state, persist_messages_and_state, persist_prompt_state, process_tool_effects,
    truncate_chars,
};

pub(crate) struct TuiLaunchOptions {
    pub session_id: Option<String>,
}

const MAX_TOOL_LOOP_ITERATIONS: usize = 100;
const PICKER_CONTROLS_NOTE: &str =
    "type to filter, use Up/Down to choose, Tab/Enter to select, Esc to cancel";
const HISTORY_SEARCH_CONTROLS_NOTE: &str =
    "type to filter, Ctrl+R/Up/Down to cycle, Enter to accept, Esc to cancel";
/// Number of ticks before an auto-dismissed task notification dialog disappears.
const TASK_NOTICE_TTL: u8 = 30;
/// Default lifetime for non-blocking shell overlay notifications.
const SHELL_NOTIFICATION_TTL: u32 = 30;

fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
}

mod controller;
mod extract_memories;
mod helpers;
mod screen;
mod scroll;
mod setup;
mod status_line;

#[cfg(test)]
mod tests;

use controller::*;
use helpers::*;
use screen::*;
use scroll::*;
use setup::*;

pub(crate) fn run_tui<W: Write>(
    writer: W,
    registry: &CommandRegistry,
    storage_dir: Option<&Path>,
    options: TuiLaunchOptions,
) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(WonderError::validation(
            "`wonder-of-u tui` requires an interactive terminal",
        ));
    }
    let authenticated = ProviderResolver::builtin()
        .load_report(storage_dir)?
        .readiness
        == ProviderReadiness::Ready;
    let context = CommandContext {
        session_id: SessionId::new(),
        cwd: std::env::current_dir()?,
        features: FeatureSet::first_release(),
        authenticated,
        interactive: true,
        permission_mode: PermissionMode::Default,
        theme: None,
        session_color: None,
        effort_level: None,
        brief_mode: false,
        fast_mode: false,
        optimize_token_mode: false,
        session_tags: Vec::new(),
        additional_working_directories: Vec::new(),
    };
    let mut controller = TuiController::new(context, registry, storage_dir, options)?;
    let mut term = setup_ratatui_terminal(writer)?;
    let mut events = EventLoop::new(CrosstermEventSource, Duration::from_millis(500));
    let initial_size = term.size()?;
    controller.on_terminal_resize(initial_size.width, initial_size.height);

    render_tui(&mut term, &controller)?;
    controller.mark_rendered();

    let run_result = (|| -> Result<()> {
        while !controller.exit_requested() {
            let event = events.next_event()?;
            controller.handle_event(event, |c| render_tui(&mut term, c))?;
            if let Some(request) = controller.take_external_editor_request() {
                // Temporarily leave the TUI while the external editor runs.
                restore_ratatui_terminal(&mut term);
                let result = launch_external_editor(&request);
                terminal::enable_raw_mode()?;
                execute!(
                    term.backend_mut(),
                    EnterAlternateScreen,
                    Hide,
                    EnableBracketedPaste,
                    EnableMouseCapture,
                    PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
                )?;
                term.clear()?;
                controller.finish_external_editor_request(&request, result);
            }
            if controller.needs_render() {
                render_tui(&mut term, &controller)?;
                controller.mark_rendered();
            }
        }
        Ok(())
    })();

    restore_ratatui_terminal(&mut term);
    run_result
}
