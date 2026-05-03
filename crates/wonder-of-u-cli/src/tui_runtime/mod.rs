//! Provides tui runtime support
//!
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::Duration,
};

use crossterm::{
    cursor::{Hide, Show},
    event::{DisableBracketedPaste, EnableBracketedPaste},
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
    CompletionRequest, CredentialStore, ProviderResolver, ProviderRuntime, ProviderSelection,
    ProviderToolCall, ProviderToolResultMessage, ProviderToolSpec, SettingsStore,
    ToolConversationRound, ToolUseRequest, ToolUseResponse, builtin_tool_registry,
};
use wonder_of_u_core::{
    AdditionalWorkingDirectory, AppState, AuthMaterialKind, AuthState, CommandContext,
    CommandOutput, CommandQuery, CommandRegistry, FeatureSet, InputMode, MessageEnvelope,
    MessagePayload, PendingLocalToolCall, PendingProviderToolCall, PendingProviderToolResult,
    PendingToolApprovalState, PendingToolConversationRound, PermissionDecision, PermissionMode,
    PermissionRequest, PermissionRuleSource, ProviderReadiness, QueuePlacement, Result, SessionId,
    TaskState, TaskStatus, ToolContext, ToolQuery, ToolResult, ToolUseId, WonderError,
    parse_slash_command, session_footer_text, session_status_text,
};
use wonder_of_u_storage::{TaskStore, TranscriptStore};
use wonder_of_u_tools::provider_tool_specs;
use wonder_of_u_tui::{
    CrosstermEventSource, DialogView, EditAction, EventLoop, HistorySearchView, KeyBindingContext,
    KeyBindingResolver, KeyCode, KeyEvent, MouseEventKind, NotificationInput, NotificationLifetime,
    NotificationQueue, NotificationSeverity, PermissionSummaryView, PickerListEntry,
    PickerListView, PromptSuggestion, PromptSuggestionState, Rect, ResolvedKey, ShellLayout,
    ShellView, SlashSuggestionEntry, SlashSuggestionsOverlay, TextBuffer, Theme,
    TranscriptScrollView, TurnState, UiEvent, VimMode, VimState, message_lines,
};

use crate::commands;
use crate::commands::prompt::{
    SessionPersistenceState, append_contextual_message, load_or_create_state,
    persist_messages_and_state, persist_prompt_state, truncate_chars,
};

pub(crate) struct TuiLaunchOptions {
    pub session_id: Option<String>,
}

const MAX_TOOL_LOOP_ITERATIONS: usize = 6;
const PICKER_CONTROLS_NOTE: &str =
    "type to filter, use Up/Down to choose, Tab/Enter to select, Esc to cancel";
const HISTORY_SEARCH_CONTROLS_NOTE: &str =
    "type to filter, Ctrl+R/Up/Down to cycle, Enter to accept, Esc to cancel";
/// Number of ticks before an auto-dismissed task notification dialog disappears.
const TASK_NOTICE_TTL: u8 = 30;
/// Default lifetime for non-blocking shell overlay notifications.
const SHELL_NOTIFICATION_TTL: u32 = 30;

mod controller;
mod helpers;
mod screen;
mod scroll;
mod setup;

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
        session_tags: Vec::new(),
        additional_working_directories: Vec::new(),
    };
    let mut controller = TuiController::new(context, registry, storage_dir, options)?;
    let mut term = setup_ratatui_terminal(writer)?;
    let mut events = EventLoop::new(CrosstermEventSource, Duration::from_millis(500));

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
                    EnableBracketedPaste
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
