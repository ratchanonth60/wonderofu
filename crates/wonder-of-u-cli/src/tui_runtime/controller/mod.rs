use super::extract_memories::{ExtractionHandle, maybe_spawn_extract_memories};
use super::status_line::{StatusLineHandle, maybe_run_status_line};
use super::*;
use std::collections::VecDeque;
use std::time::Instant;

use crate::commands::task_runtime::TaskManager;

/// Lines scrolled per single mouse-wheel notch in the transcript area.
const MOUSE_SCROLL_LINES: i32 = 3;
/// Lines scrolled per Alt+Up/Alt+Down keypress.
const KEYBOARD_SCROLL_LINES: i32 = 3;
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) enum ActiveOverlay {
    HistorySearch,
    GlobalSearch,
    Picker,
    ConfirmDialog,
    NoticeDialog,
    FleetPanel,
    None,
}
enum StreamingCompletionEvent {
    Delta(String),
    Done(Result<wonder_of_u_agent::CompletionResponse>),
    #[allow(dead_code)]
    Progress,
}
pub(super) struct TuiController<'a> {
    pub(super) registry: &'a CommandRegistry,
    pub(super) storage_dir: Option<PathBuf>,
    pub(super) state: AppState,
    pub(super) persistence: SessionPersistenceState,
    pub(super) prompt: TextBuffer,
    pub(super) keymap: KeyBindingResolver,
    pub(super) vim: VimState,
    pub(super) vim_enabled: bool,
    pub(super) history_search: Option<HistorySearchState>,
    pub(super) global_search_open: bool,
    pub(super) global_search_query: String,
    pub(super) global_search_results: Vec<SearchMatch>,
    pub(super) global_search_selected: usize,
    pub(super) global_search_cursor: usize,
    pub(super) global_search_dirty_since: Option<Instant>,
    pub(super) turn_state: TurnState,
    pub(super) loading_frame: u64,
    tick_counter: u64,
    pub(super) needs_render: bool,
    pub(super) exit_requested: bool,
    pub(super) status_note: Option<String>,
    pub(super) dialog: Option<DialogView>,
    pub(super) pending_tag_removal: Option<TagRemovalState>,
    pub(super) pending_theme_picker: Option<ThemePickerState>,
    pub(super) pending_model_picker: Option<ModelPickerState>,
    pub(super) pending_permission_picker: Option<PermissionPickerState>,
    pub(super) pending_memory_picker: Option<MemoryPickerState>,
    pub(super) pending_external_editor: Option<ExternalEditorRequest>,
    /// Ephemeral setup hub overlay opened by `/setup`.  Never persisted.
    pub(super) pending_setup_overlay: Option<SetupOverlayState>,
    /// Ephemeral provider-login / API-base form opened from the setup hub.  Never persisted.
    pub(super) pending_provider_form: Option<ProviderFormState>,
    /// Ephemeral Copilot device-code OAuth flow.  Never persisted.
    pub(super) pending_copilot_oauth: Option<CopilotOAuthFlowState>,
    /// Countdown ticks until the task notification dialog is auto-dismissed.
    pub(super) task_notice_ttl: Option<u16>,
    /// Whether the cost threshold dialog has been shown this session.
    cost_threshold_dialog_shown_session: bool,
    pub(super) notifications: NotificationQueue,
    /// All slash-command suggestions, built once at startup.
    pub(super) slash_suggestions: Vec<PromptSuggestion>,
    /// Live filtered state when the user is typing a `/` command.
    pub(super) active_suggestions: Option<PromptSuggestionState>,
    /// Cached file-mention suggestions (project files). Lazily built.
    pub(super) file_mention_cache: Option<Vec<PromptSuggestion>>,
    /// Live filtered state when the user is typing an `@` file mention.
    pub(super) file_mentions: Option<PromptSuggestionState>,
    /// Ephemeral transcript scroll position; never persisted to `AppState`.
    pub(super) scroll_state: TranscriptScrollState,
    /// Last known terminal dimensions `(width, height)` in columns × rows.
    ///
    /// Updated on every `UiEvent::Resize` so that mouse hit-testing can
    /// recompute the transcript area rect without touching ratatui's backend.
    pub(super) last_terminal_size: (u16, u16),
    /// Set to `true` when the user explicitly cancels the setup overlay during
    /// this session, suppressing the autostart re-open logic.
    pub(super) setup_cancelled_this_session: bool,
    /// Whether the right-side sidebar companion panel is currently visible.
    /// Ephemeral per TUI session; never persisted.
    pub(super) sidebar_visible: bool,
    /// Whether collapsed tool output previews should render fully expanded inline.
    pub(super) expand_tool_output: bool,
    /// Cached live integration summaries shown in the sidebar.
    ///
    /// Some sections read small config files or scan PATH; cache them outside
    /// `view()` so typing/rendering never performs blocking discovery work.
    pub(super) sidebar_cache: SidebarPanelCache,
    /// The permission mode that was active immediately before the session
    /// entered [`PermissionMode::Plan`].  Set when plan mode is entered so that
    /// [`PlanAction::Exit`] can restore the original mode (e.g. `AcceptEdits`
    /// or `BypassPermissions`) rather than always falling back to `Default`.
    /// Cleared whenever the session leaves plan mode.
    pub(super) pre_plan_permission_mode: Option<PermissionMode>,
    /// Lazily-initialised task manager used for agent-summary generation.
    ///
    /// Kept on the controller so that the internal `Arc<Mutex<...>>` timing
    /// state survives across TUI ticks.  `None` when no `storage_dir` is set.
    pub(super) task_manager: Option<TaskManager>,
    /// Live stdout lines received from a running shell tool (bash/shell).
    /// Cleared after each tool call completes. Shown in the loading area.
    pub(super) tool_progress_lines: VecDeque<String>,
    /// Receiving end of the shell-tool progress channel.
    /// Drained on every tick while a tool is executing.
    pub(super) tool_progress_rx: Option<std::sync::mpsc::Receiver<String>>,
    /// Total context tokens reported by the latest provider response
    /// (input + cache creation + cache read + output).  `None` until the first
    /// response of the session arrives (or after `/clear` / `/compact`).
    pub(super) last_context_usage: Option<u64>,
    /// Number of messages in `state.messages` already covered by
    /// `last_context_usage`.  Messages at indices `>= anchor` are estimated
    /// with the character heuristic instead.
    pub(super) context_usage_anchor: usize,
    /// Consecutive auto-compact failures.  When this reaches the circuit-breaker
    /// limit we stop attempting automatic compaction until the session resets.
    pub(super) autocompact_failures: u8,
    /// Set when `maybe_autocompact` queues a `/compact`.  Cleared on success
    /// (ViewActionHint::Compact) or failure (TurnState::Interrupted).
    pub(super) autocompact_pending: bool,
    /// When `true`, mouse capture is disabled so the terminal can handle native
    /// text selection.  Press any key to exit this mode.
    pub(super) selection_mode: bool,
    /// Sidebar visibility saved when entering selection mode, restored on exit.
    pub(super) selection_mode_sidebar_was_visible: bool,
    /// Index into `prompt_history_entries` currently shown via Up/Down recall.
    /// `None` = not in recall mode.  0 = most recent entry.
    pub(super) history_recall_index: Option<usize>,
    /// Prompt text saved when recall started, restored when user presses Down
    /// past the most recent entry.
    pub(super) history_recall_saved: String,
    /// When an interaction tool (ask_user) is executing, this sends the user's
    /// typed answer to the waiting tool thread.
    /// Question text displayed to the user while an interaction tool is waiting.
    pub(super) interaction_question: Option<String>,
    /// Pre-defined options for the current interaction question.
    /// Empty when the question has no options (free-text only).
    pub(super) interaction_options: Vec<String>,
    /// Set when user selects "Other" from the options list — enables free-text
    /// prompt input instead of navigating the option picker.
    pub(super) interaction_other_mode: bool,
    /// Answer typed by the user, pending delivery to a paused interaction tool.
    pub(super) interaction_pending_answer: Option<String>,
    /// Background memory extraction state.
    pub(super) extraction: ExtractionHandle,
    /// Tracks whether extraction was active on the previous tick, for detecting
    /// completion transitions and pushing a memory-updated notification.
    extraction_was_active: bool,
    /// Shell command from `settings.statusLine` to run after each AI response.
    pub(super) status_line_command: Option<String>,
    /// Handle for the background status-line command thread.
    pub(super) status_line_handle: StatusLineHandle,
    pub(super) active_turn: ActiveTurn,
    /// Images queued to be sent with the next prompt submission.
    /// Each entry is `(attachment, placeholder_label)` where the label is
    /// what was inserted into the prompt buffer (e.g. `"[Image: foo.png]"`).
    pub(super) pending_images: Vec<(wonder_of_u_agent::ImageAttachment, String)>,
    /// Index of the currently selected message when in message cursor mode.
    pub(super) message_cursor_index: Option<usize>,
    /// Whether the selected tool group is expanded.
    pub(super) message_cursor_expanded: bool,
    /// Ephemeral message selector (rewind) picker state.
    pub(super) pending_message_selector: Option<MessageSelectorState>,
    /// The message indices (in `self.state.messages`) of user text messages,
    /// collected when the message selector was opened. Used to compute the
    /// rewind cutoff when the user confirms a selection.
    pub(super) message_selector_user_indices: Vec<usize>,
    /// Ephemeral git diff dialog state.  Never persisted.
    pub(super) diff_dialog: Option<DiffDialogState>,
    /// Ephemeral log selector (session picker) state.  Never persisted.
    pub(super) pending_log_selector: Option<LogSelectorState>,
    /// Ephemeral export dialog state.  Never persisted.
    pub(super) pending_export_dialog: Option<ExportDialogState>,
    /// Ephemeral output-style picker state.  Never persisted.
    pub(super) pending_output_style_picker: Option<OutputStylePickerState>,
    /// Ephemeral memory file selector state.  Never persisted.
    pub(super) pending_memory_file_selector: Option<MemoryFileSelectorState>,
    /// Ephemeral hooks config browser state.  Never persisted.
    pub(super) pending_hooks_menu: Option<HooksMenuState>,
    /// Ephemeral fleet panel state.  Never persisted.
    pub(super) fleet_panel: Option<FleetPanelState>,
    /// Idempotence keys for fleet completion notifications fired this session.
    pub(super) fleet_completion_keys: std::collections::BTreeSet<String>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct DiffFileEntry {
    pub path: String,
    pub lines_added: u64,
    pub lines_removed: u64,
    pub is_binary: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct DiffDialogState {
    pub files: Vec<DiffFileEntry>,
    pub selected_index: usize,
    pub detail_mode: bool,
    pub detail_title: String,
    pub detail_lines: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct LogSelectorEntry {
    pub session_id: SessionId,
    pub title: String,
    pub message_count: usize,
    pub updated_at: time::OffsetDateTime,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct LogSelectorState {
    pub entries: Vec<LogSelectorEntry>,
    pub selected_index: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum ExportDialogMode {
    #[default]
    PickOption,
    EnterFilename,
}

#[derive(Clone, Debug)]
pub(super) struct ExportDialogState {
    pub selected_index: usize,
    pub mode: ExportDialogMode,
    pub filename: TextBuffer,
}

impl Default for ExportDialogState {
    fn default() -> Self {
        Self {
            selected_index: 0,
            mode: ExportDialogMode::default(),
            filename: TextBuffer::from_text("transcript.md", false),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OutputStyleOption {
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Debug)]
pub(super) struct OutputStylePickerState {
    pub selected_index: usize,
    pub current: String,
    pub options: Vec<OutputStyleOption>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MemoryFileEntry {
    pub kind: String,
    pub label: String,
    pub path: PathBuf,
    pub exists: bool,
    pub depth: usize,
}

#[derive(Clone, Debug)]
pub(super) struct MemoryFileSelectorState {
    pub selected_index: usize,
    pub entries: Vec<MemoryFileEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HooksMenuEntry {
    pub id: String,
    pub event: String,
    pub matcher: String,
    pub kind: String,
    pub target: String,
    pub condition: Option<String>,
    pub status: String,
    pub managed: bool,
    pub supported: bool,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum HooksMenuScreen {
    #[default]
    Browse,
    ViewDetail(usize),
}

#[derive(Clone, Debug)]
pub(super) struct HooksMenuState {
    pub entries: Vec<HooksMenuEntry>,
    pub selected_index: usize,
    pub screen: HooksMenuScreen,
    pub disabled: bool,
    pub config_path: String,
}

#[derive(Clone, Debug)]
pub(super) struct FleetPanelState {
    pub selected_index: usize,
    pub fleet_views: Vec<wonder_of_u_tui::fleet_view::FleetRunView>,
    pub selected_run_index: Option<usize>,
    pub scroll_offset: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct SidebarPanelCache {
    pub(super) tool_lines: Vec<String>,
    pub(super) mcp_lines: Vec<String>,
    pub(super) lsp_lines: Vec<String>,
    pub(super) todo_lines: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModelPickerOption {
    pub(super) provider: String,
    pub(super) provider_display: String,
    pub(super) model: String,
    pub(super) model_display: String,
    pub(super) default: bool,
    pub(super) selected: bool,
    pub(super) auth: String,
}
impl ModelPickerOption {
    fn search_key(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.provider, self.provider_display, self.model, self.model_display, self.auth
        )
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModelPickerState {
    pub(super) original_input: String,
    pub(super) options: Vec<ModelPickerOption>,
    pub(super) selected_index: usize,
    pub(super) query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ThemePickerOption {
    pub(super) theme: String,
    pub(super) label: String,
    pub(super) description: String,
    pub(super) selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ThemePickerState {
    pub(super) original_input: String,
    pub(super) options: Vec<ThemePickerOption>,
    pub(super) selected_index: usize,
    pub(super) query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TagRemovalState {
    pub(super) original_input: String,
    pub(super) tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PermissionPickerOption {
    pub(super) mode: PermissionMode,
    pub(super) label: String,
    pub(super) description: String,
    pub(super) selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PermissionPickerState {
    pub(super) original_input: String,
    pub(super) options: Vec<PermissionPickerOption>,
    pub(super) selected_index: usize,
    pub(super) query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MemoryPickerOption {
    pub(super) target: crate::commands::project::MemoryTarget,
    pub(super) label: String,
    pub(super) description: String,
    pub(super) path: PathBuf,
    pub(super) selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MemoryPickerState {
    pub(super) original_input: String,
    pub(super) options: Vec<MemoryPickerOption>,
    pub(super) selected_index: usize,
    pub(super) query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HistorySearchState {
    pub(super) query: TextBuffer,
    pub(super) matches: Vec<usize>,
    pub(super) cursor: usize,
    pub(super) saved_buffer: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MessageSelectorEntry {
    pub(super) preview: String,
    pub(super) message_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MessageSelectorState {
    pub(super) selected_index: usize,
    pub(super) entries: Vec<MessageSelectorEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExternalEditorRequest {
    pub(super) cwd: PathBuf,
    pub(super) path: PathBuf,
}

#[derive(Clone)]
pub(super) struct LocalToolCall {
    pub(super) provider_call: ProviderToolCall,
    pub(super) use_id: ToolUseId,
}

pub(super) enum ToolExecutionOutcome {
    Completed(ProviderToolResultMessage),
    Paused { reason: String },
}

pub(super) enum RestoredPromptUiState {
    PermissionPicker(PermissionPickerState),
    MemoryPicker(MemoryPickerState),
    TagRemoval(TagRemovalState),
    ThemePicker(ThemePickerState),
    ModelPicker(ModelPickerState),
    Notice {
        dialog: DialogView,
        status_note: String,
    },
    Status(String),
}

/// Represents a streaming completion currently in progress.
#[allow(dead_code)]
pub(super) struct ActiveStreaming {
    assistant_index: usize,
    provider_id: String,
    event_rx: std::sync::mpsc::Receiver<StreamingCompletionEvent>,
    user_message: MessageEnvelope,
    resolved: wonder_of_u_agent::ResolvedProviderExecution,
    input: String,
}

/// Phases of the tool loop state machine.
enum ToolLoopPhase {
    SendRequest {
        iteration: usize,
    },
    AwaitingResponse {
        iteration: usize,
        response_rx: std::sync::mpsc::Receiver<Result<wonder_of_u_agent::ToolUseResponse>>,
    },
    ExecutingTools {
        iteration: usize,
        tool_index: usize,
        local_calls: Vec<LocalToolCall>,
        round: ToolConversationRound,
    },
    PausedForApproval,
}

pub(super) struct ActiveToolLoop {
    phase: ToolLoopPhase,
    request_prompt: String,
    system_prompt: Option<String>,
    resolved: wonder_of_u_agent::ResolvedProviderExecution,
    rounds: Vec<ToolConversationRound>,
    #[allow(dead_code)]
    user_message: MessageEnvelope,
    staged_messages: Vec<MessageEnvelope>,
    /// Images attached to the initial user turn; included only on round 0.
    images: Vec<wonder_of_u_agent::ImageAttachment>,
}

pub(super) enum ActiveTurn {
    None,
    Streaming(ActiveStreaming),
    ToolLoop(ActiveToolLoop),
}

pub(super) mod autocompact;
pub(super) mod dialogs;
pub(super) mod events;
pub(super) mod export_dialog;
pub(super) mod fleet_panel;
pub(super) mod history;
pub(super) mod hooks_menu;
pub(super) mod log_selector;
pub(super) mod memory_file_selector;
pub(super) mod message_cursor;
pub(super) mod output_style_picker;
pub(super) mod search;
pub(super) mod selection;
pub(super) mod state;
pub(super) mod turn;
pub(super) mod view;

#[cfg(test)]
mod tests;

// ── Free helper functions ──

pub(super) fn estimate_payload_chars(payload: &MessagePayload) -> u64 {
    match payload {
        MessagePayload::UserText { content } => content.len() as u64,
        MessagePayload::AssistantText { content } => content.len() as u64,
        MessagePayload::AssistantToolUse { tool, input, .. } => {
            (tool.len() + input.to_string().len()) as u64
        }
        MessagePayload::ToolResult { tool, content, .. } => (tool.len() + content.len()) as u64,
        MessagePayload::CompactBoundary { summary } => summary.len() as u64,
        MessagePayload::System { content } => content.len() as u64,
        MessagePayload::Command { input, output } => {
            input.len() as u64 + output.as_ref().map_or(0, |o| o.len()) as u64
        }
        MessagePayload::ProviderError { message, .. } => message.len() as u64,
        MessagePayload::Permission { tool, reason, .. } => (tool.len() + reason.len()) as u64,
        _ => 0,
    }
}
pub(super) fn is_loading_turn_state(state: TurnState) -> bool {
    matches!(
        state,
        TurnState::ModelRequestActive
            | TurnState::CommandQueued
            | TurnState::ToolPermissionPending
            | TurnState::StreamingResponse
            | TurnState::ToolExecuting
    )
}
pub(super) fn loading_verb_label(state: TurnState) -> Option<&'static str> {
    match state {
        TurnState::ModelRequestActive => Some("thinking"),
        TurnState::CommandQueued => Some("running"),
        TurnState::ToolPermissionPending => Some("waiting"),
        TurnState::StreamingResponse => Some("streaming"),
        TurnState::ToolExecuting => Some("executing"),
        _ => None,
    }
}
pub(super) fn model_status_label(provider: Option<&str>, model: Option<&str>) -> String {
    match (provider, model) {
        (Some(provider), Some(model)) => format!("{provider}:{model}"),
        (None, Some(model)) => model.to_string(),
        (Some(provider), None) => provider.to_string(),
        (None, None) => "model:auto".into(),
    }
}
pub(super) fn estimated_cost_label(cost: Option<f64>) -> String {
    cost.map_or_else(|| "cost:--".into(), |cost| format!("${cost:.2}"))
}
pub(super) fn context_sidebar_lines(used_tokens: u64, max_tokens: Option<u64>) -> Vec<String> {
    let Some(max_tokens) = max_tokens.filter(|max_tokens| *max_tokens > 0) else {
        return vec!["Context: unknown".into()];
    };
    let percentage = used_tokens.saturating_mul(100) / max_tokens;
    let filled = ((used_tokens.saturating_mul(24)) / max_tokens).min(24) as usize;
    vec![
        format!(
            "{} / {} tokens",
            format_token_count(used_tokens),
            format_token_count(max_tokens)
        ),
        format!(
            "[{}{}] {}%",
            "#".repeat(filled),
            "-".repeat(24usize.saturating_sub(filled)),
            percentage.min(100)
        ),
    ]
}
/// Mirrors `ShellView::prompt_height()` for a raw prompt string.
///
/// Returns the number of rows the prompt box will occupy: one row per logical
/// line (using `split('\n')` to match the renderer so trailing newlines from
/// Shift+Enter count as a visible blank row) plus the top border, integrated
/// footer row, and bottom border (`+ 3`).  Minimum is 4.
///
/// Using this helper in `on_terminal_resize` and `transcript_messages_rect`
/// keeps the controller's prompt-height estimate consistent with the renderer
/// so that `scroll_state.last_visible_lines` and the mouse-hit-test rect are
/// always accurate.
pub(super) fn controller_prompt_height(prompt_text: &str) -> u16 {
    let line_count = prompt_text.split('\n').count().max(1);
    u16::try_from(line_count)
        .unwrap_or(u16::MAX)
        .saturating_add(3) // top border + integrated footer row + bottom border
        .max(4) // minimum boxed height: border + 1 content row + footer + border
}
pub(super) fn format_token_count(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(digit);
    }
    formatted
}
pub(super) fn compact_cwd_label(path: &Path) -> String {
    let display = path.display().to_string();
    if display.chars().count() <= 24 {
        return display;
    }

    let tail = display
        .chars()
        .rev()
        .take(23)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("…{tail}")
}
pub(super) fn chrome_status_text(state: &AppState) -> String {
    let mut parts = vec![
        model_status_label(state.provider.as_deref(), state.model.as_deref()),
        compact_cwd_label(&state.session.cwd),
        format!("{} tok", state.costs.usage.total_tokens()),
        estimated_cost_label(state.costs.estimated_cost_usd),
    ];

    if state.messages.is_empty() {
        parts.push("ready".into());
    }

    parts.join(" | ")
}
/// Build the full list of slash-command suggestions from the command registry.
///
/// Called once at TUI startup; the result is stored on `TuiController` and
/// re-used (with live filtering) on every keystroke.
///
/// When a [`CommandSpec`] carries an `argument_hint` (e.g. `[on|off]`), the
/// hint is appended to the display text so the autocomplete overlay reads e.g.
/// `/fast [on|off]`.  The *replacement* text stays as just `/commandname` so
/// the cursor lands right after the command name, ready for the user to type
/// their argument.
pub(super) fn build_slash_suggestions(registry: &CommandRegistry) -> Vec<PromptSuggestion> {
    registry
        .all_specs()
        .into_iter()
        .filter(|spec| !spec.hidden)
        .map(|spec| {
            let slash = format!("/{}", spec.name);
            let display = match &spec.argument_hint {
                Some(hint) => format!("{slash} {hint}"),
                None => slash.clone(),
            };
            PromptSuggestion::new(spec.name.clone(), display, slash)
                .with_description(spec.description.clone())
                .with_keywords(spec.aliases.iter().map(|a| format!("/{a}")))
        })
        .collect()
}
pub(super) const LSP_SERVERS: &[(&str, &str)] = &[
    ("rust-analyzer", "Rust"),
    ("typescript-language-server", "TypeScript"),
    ("pyright-langserver", "Python"),
    ("gopls", "Go"),
    ("clangd", "C/C++"),
];
/// Compile a compact tools summary for the sidebar.
///
/// Shows `N enabled / M registered` on the first line, then a breakdown of
/// enabled tools by [`ToolKind`].  Falls back to an error line if the registry
/// cannot be built.
///
/// When `storage_dir` is `Some`, MCP catalog tools from enabled servers are
/// included in the count (soft-fails per server).
pub(super) fn tool_sidebar_lines(context: &ToolContext, storage_dir: Option<&Path>) -> Vec<String> {
    let registry = {
        let result = match storage_dir {
            Some(root) => wonder_of_u_tools::builtin_registry_with_mcp_catalog(root),
            None => wonder_of_u_tools::builtin_registry(),
        };
        match result {
            Ok(r) => r,
            Err(e) => return vec![format!("⚠ tools unavailable: {e}")],
        }
    };

    let all_specs = registry.all_specs();
    let registered = all_specs.len();

    // Match the provider tool loop so this count reflects tools actually
    // offered to the model after feature and static permission filtering.
    let enabled_specs = provider_tool_specs(&registry, context, None);
    let enabled = enabled_specs.len();

    let mut lines = vec![format!("  {enabled} enabled / {registered} registered")];

    // Breakdown by source – compact one-liner per non-zero category.
    let native = enabled_specs
        .iter()
        .filter(|s| s.source == ToolSource::Native)
        .count();
    let mcp = enabled_specs
        .iter()
        .filter(|s| s.source == ToolSource::Mcp)
        .count();
    let skill = enabled_specs
        .iter()
        .filter(|s| s.source == ToolSource::Skill)
        .count();
    let plugin = enabled_specs
        .iter()
        .filter(|s| s.source == ToolSource::Plugin)
        .count();

    // Breakdown by kind for native tools (most informative for users).
    let shell = enabled_specs
        .iter()
        .filter(|s| s.kind == ToolKind::Shell)
        .count();
    let file = enabled_specs
        .iter()
        .filter(|s| s.kind == ToolKind::FileRead || s.kind == ToolKind::FileWrite)
        .count();
    let web = enabled_specs
        .iter()
        .filter(|s| s.kind == ToolKind::Web)
        .count();

    if native > 0 {
        let mut parts: Vec<String> = Vec::new();
        if shell > 0 {
            parts.push(format!("{shell}sh"));
        }
        if file > 0 {
            parts.push(format!("{file}fs"));
        }
        if web > 0 {
            parts.push(format!("{web}web"));
        }
        let rest = native.saturating_sub(shell + file + web);
        if rest > 0 {
            parts.push(format!("{rest}other"));
        }
        lines.push(format!("  native: {}", parts.join(" ")));
    }
    if mcp > 0 {
        lines.push(format!("  mcp: {mcp}"));
    }
    if skill > 0 {
        lines.push(format!("  skill: {skill}"));
    }
    if plugin > 0 {
        lines.push(format!("  plugin: {plugin}"));
    }

    lines
}
/// Build the MCP sidebar section from the stored config (no server spawning).
///
/// Reads `McpConfigStore` only during sidebar cache refreshes. Shows a concise
/// enabled/total count plus one line per server name.
pub(super) fn mcp_sidebar_lines(
    storage_dir: Option<&std::path::Path>,
    cwd: &std::path::Path,
) -> Vec<String> {
    let Some(dir) = storage_dir else {
        return vec!["  mcp: no storage dir".into()];
    };

    let store = McpConfigStore::new(dir);
    let config = match store.read_with_project(cwd, None) {
        Ok((config, _)) => config,
        Err(e) => return vec![format!("⚠ mcp config unavailable: {e}")],
    };

    let total = config.servers.len();
    let enabled = config.servers.iter().filter(|s| s.enabled).count();

    if total == 0 {
        return vec!["  no servers configured".into()];
    }

    let mut lines = vec![format!("  {enabled}/{total} servers enabled")];
    for server in &config.servers {
        let icon = if server.enabled { "✓" } else { "  " };
        // Truncate long names so they fit the sidebar column.
        let name: String = server.name.chars().take(20).collect();
        lines.push(format!("{icon} {name}"));
    }
    lines
}
/// Check PATH for common LSP binaries and return one line per entry.
///
/// Each line is `✓ Label` when the binary is found or `  Label (not found)`
/// otherwise.  Never probes network or spawns processes.
pub(super) fn lsp_sidebar_lines(cwd: &std::path::Path) -> Vec<String> {
    // Annotate with project-type hints so users know which servers matter.
    let project_hints: &[(&str, &str)] = &[
        ("Cargo.toml", "rust-analyzer"),
        ("package.json", "typescript-language-server"),
        ("pyproject.toml", "pyright-langserver"),
        ("go.mod", "gopls"),
    ];

    let mut lines = Vec::new();
    for (binary, label) in LSP_SERVERS {
        let found = binary_on_path(binary);
        // Show a hint when the binary is relevant to this project.
        let is_relevant = project_hints
            .iter()
            .any(|(marker, bin)| *bin == *binary && cwd.join(marker).exists());
        if found {
            lines.push(format!("✓ {label}"));
        } else if is_relevant {
            // Missing but relevant – highlight so users notice.
            lines.push(format!("⚠ {label} (not found)"));
        } else {
            lines.push(format!("  {label} (not found)"));
        }
    }
    lines
}
pub(super) fn previous_prompt_permission_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Default => PermissionMode::Plan,
        PermissionMode::AcceptEdits => PermissionMode::Default,
        PermissionMode::BypassPermissions => PermissionMode::AcceptEdits,
        PermissionMode::DontAsk => PermissionMode::BypassPermissions,
        PermissionMode::Plan => PermissionMode::DontAsk,
    }
}
pub(super) fn permission_mode_status_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept edits",
        PermissionMode::BypassPermissions => "bypass permissions",
        PermissionMode::DontAsk => "don't ask",
        PermissionMode::Plan => "plan mode",
    }
}
/// Parse `todos.md` in `cwd` and return compact `[x]`/`[ ]` lines.
///
/// At most [`TODO_SIDEBAR_CAP`] items are shown; the rest are summarised as
/// `+N more`.  Returns an empty `Vec` when the file does not exist (not an
/// error – the Todo section is simply hidden).
pub(super) fn todo_sidebar_lines(cwd: &std::path::Path) -> Vec<String> {
    let path = cwd.join("todos.md");
    if !path.exists() {
        return Vec::new();
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return vec![format!("⚠ todos unreadable: {e}")],
    };

    parse_todo_lines(&content)
}
pub(super) fn todo_task_store_sidebar_lines(
    storage_dir: &std::path::Path,
    session_id: SessionId,
) -> Option<Vec<String>> {
    let store = TodoTaskStore::new(storage_dir);
    let list = store.read_or_default(session_id).ok()?;
    let mut visible: Vec<_> = list
        .tasks
        .values()
        .filter(|entry| entry.status != TodoTaskStatus::Deleted)
        .collect();
    visible.sort_by_key(|entry| entry.created_at);

    if visible.is_empty() {
        return None;
    }

    let total = visible.len();
    let shown = total.min(TODO_SIDEBAR_CAP);
    let mut lines: Vec<String> = visible[..shown]
        .iter()
        .map(|entry| {
            let label: String = entry.subject.chars().take(22).collect();
            match entry.status {
                TodoTaskStatus::Pending => format!("  {label}"),
                TodoTaskStatus::InProgress => format!("▷ {label}"),
                TodoTaskStatus::Completed => format!("✓ {label}"),
                TodoTaskStatus::Deleted => unreachable!("deleted entries are filtered above"),
            }
        })
        .collect();

    let remaining = total.saturating_sub(shown);
    if remaining > 0 {
        lines.push(format!("  +{remaining} more"));
    }
    Some(lines)
}
pub(super) fn todo_merged_sidebar_lines(
    cwd: &std::path::Path,
    storage_dir: Option<&std::path::Path>,
    session_id: SessionId,
) -> Vec<String> {
    if let Some(storage_dir) = storage_dir
        && let Some(lines) = todo_task_store_sidebar_lines(storage_dir, session_id)
    {
        return lines;
    }
    todo_sidebar_lines(cwd)
}
/// Maximum number of todo items shown in the sidebar before `+N more` cap.
pub(super) const TODO_SIDEBAR_CAP: usize = 6;
/// Extract and format todo checkbox lines from markdown content.
///
/// Recognises `- [ ] …` and `- [x] …` (case-insensitive `x`).  Leading
/// whitespace before the `-` is ignored so nested items are included.
pub(super) fn parse_todo_lines(content: &str) -> Vec<String> {
    let items: Vec<(bool, &str)> = content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            trimmed
                .strip_prefix("- [x] ")
                .or_else(|| trimmed.strip_prefix("- [X] "))
                .map(|t| (true, t))
                .or_else(|| trimmed.strip_prefix("- [ ] ").map(|t| (false, t)))
        })
        .collect();

    let total = items.len();
    let shown = items.len().min(TODO_SIDEBAR_CAP);
    let mut lines: Vec<String> = items[..shown]
        .iter()
        .map(|(done, text)| {
            // Truncate long task descriptions so they fit the sidebar column.
            let label: String = text.chars().take(22).collect();
            if *done {
                format!("✓ {label}")
            } else {
                format!("  {label}")
            }
        })
        .collect();

    let remaining = total.saturating_sub(shown);
    if remaining > 0 {
        lines.push(format!("  +{remaining} more"));
    }
    lines
}
/// Return `true` when `name` resolves to an executable file on `PATH`.
///
/// Mirrors the same logic used in `commands/advanced.rs` so LSP availability
/// checks are consistent across the CLI surface.
pub(super) fn binary_on_path(name: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(name).is_file())
}

/// Format an `OffsetDateTime` as a human-friendly relative label (e.g. "2h ago", "3d ago").
pub(super) fn relative_time_label(when: time::OffsetDateTime) -> String {
    let now = time::OffsetDateTime::now_utc();
    let delta = now - when;
    let total_secs = delta.whole_seconds().max(0) as u64;

    if total_secs < 60 {
        "just now".into()
    } else if total_secs < 3600 {
        format!("{}m ago", total_secs / 60)
    } else if total_secs < 86400 {
        format!("{}h ago", total_secs / 3600)
    } else if total_secs < 604800 {
        format!("{}d ago", total_secs / 86400)
    } else {
        format!("{}w ago", total_secs / 604800)
    }
}

impl<'a> TuiController<'a> {
    pub(super) fn new(
        context: CommandContext,
        registry: &'a CommandRegistry,
        storage_dir: Option<&Path>,
        options: TuiLaunchOptions,
    ) -> Result<Self> {
        let (state, persistence) = load_or_create_state(
            &context,
            storage_dir,
            options.session_id.as_deref(),
            &default_session_title(&context.cwd),
            "tui",
        )?;
        let mut controller = Self {
            registry,
            storage_dir: storage_dir.map(Path::to_path_buf),
            state,
            persistence,
            prompt: TextBuffer::new(true),
            keymap: crate::commands::keybinding_commands::load_keybinding_resolver(storage_dir)?,
            vim: VimState::default(),
            vim_enabled: true,
            history_search: None,
            global_search_open: false,
            global_search_query: String::new(),
            global_search_results: Vec::new(),
            global_search_selected: 0,
            global_search_cursor: 0,
            global_search_dirty_since: None,
            turn_state: TurnState::Idle,
            loading_frame: 0,
            tick_counter: 0,
            needs_render: true,
            exit_requested: false,
            status_note: None,
            dialog: None,
            pending_tag_removal: None,
            pending_theme_picker: None,
            pending_model_picker: None,
            pending_permission_picker: None,
            pending_memory_picker: None,
            pending_external_editor: None,
            pending_setup_overlay: None,
            pending_provider_form: None,
            pending_copilot_oauth: None,
            task_notice_ttl: None,
            cost_threshold_dialog_shown_session: false,
            notifications: NotificationQueue::new(),
            slash_suggestions: build_slash_suggestions(registry),
            active_suggestions: None,
            file_mention_cache: None,
            file_mentions: None,
            scroll_state: TranscriptScrollState::new(),
            last_terminal_size: (0, 0),
            setup_cancelled_this_session: false,
            sidebar_visible: true,
            expand_tool_output: false,
            sidebar_cache: SidebarPanelCache::default(),
            pre_plan_permission_mode: None,
            task_manager: storage_dir.map(TaskManager::new),
            tool_progress_lines: VecDeque::new(),
            tool_progress_rx: None,
            last_context_usage: None,
            context_usage_anchor: 0,
            autocompact_failures: 0,
            autocompact_pending: false,
            selection_mode: false,
            selection_mode_sidebar_was_visible: false,
            history_recall_index: None,
            history_recall_saved: String::new(),
            interaction_question: None,
            interaction_options: Vec::new(),
            interaction_other_mode: false,
            interaction_pending_answer: None,
            extraction: ExtractionHandle::new(),
            extraction_was_active: false,
            status_line_command: None,
            status_line_handle: StatusLineHandle::new(),
            active_turn: ActiveTurn::None,
            pending_images: Vec::new(),
            message_cursor_index: None,
            message_cursor_expanded: false,
            pending_message_selector: None,
            message_selector_user_indices: Vec::new(),
            diff_dialog: None,
            pending_log_selector: None,
            pending_export_dialog: None,
            pending_output_style_picker: None,
            pending_memory_file_selector: None,
            pending_hooks_menu: None,
            fleet_panel: None,
            fleet_completion_keys: std::collections::BTreeSet::new(),
        };
        controller.hydrate_initial_settings()?;
        controller.refresh_runtime_state()?;
        controller.rebuild_ephemeral_state();
        controller.refresh_sidebar_panel_cache();
        controller.persist_state_snapshot()?;
        futures::executor::block_on(controller.maybe_auto_open_setup())?;
        // Ensure auto-memory dir exists so the model can write without checking.
        let mem_dir =
            wonder_of_u_storage::memdir::auto_mem_dir(&controller.state.session.cwd, storage_dir);
        let _ = std::fs::create_dir_all(&mem_dir);
        Ok(controller)
    }
    pub(super) async fn handle_event(&mut self, event: UiEvent) -> Result<()> {
        match event {
            UiEvent::Key(key) => self.handle_key_event(key).await,
            UiEvent::Paste(text) => {
                // Empty bracketed-paste → terminal couldn't convert clipboard content
                // to text (common for Ctrl+V on an image). Try wl-paste.
                if text.is_empty()
                    && !self.has_picker_overlay()
                    && !self.global_search_open
                    && self.history_search.is_none()
                    && self.dialog.is_none()
                {
                    if let Some(img) = read_clipboard_image() {
                        let label = img
                            .filename
                            .as_deref()
                            .map(|f| format!("[Image: {f}]"))
                            .unwrap_or_else(|| "[Image: clipboard]".to_string());
                        self.prompt.insert_text(&label);
                        self.pending_images.push((img, label));
                        self.turn_state = TurnState::EditingInput;
                        self.state.input_mode = InputMode::Prompt;
                        self.status_note = Some("image attached from clipboard".into());
                        self.needs_render = true;
                    }
                    return Ok(());
                }
                if !text.is_empty() {
                    let text = if text.len() > 10_000 {
                        let head = &text[..500.min(text.len())];
                        let tail = &text[text.len().saturating_sub(500)..];
                        self.status_note =
                            Some("pasted text truncated to 10,000 characters".into());
                        format!("{head}\n[...truncated...]\n{tail}")
                    } else {
                        text
                    };
                    if let Some(form) = &mut self.pending_provider_form {
                        form.input.insert_text(&text);
                        self.needs_render = true;
                    } else if let Some(picker) = &mut self.pending_model_picker {
                        picker.query.insert_text(&text);
                        self.refresh_model_picker_dialog();
                    } else if let Some(picker) = &mut self.pending_theme_picker {
                        picker.query.insert_text(&text);
                        self.needs_render = true;
                    } else if let Some(picker) = &mut self.pending_permission_picker {
                        picker.query.insert_text(&text);
                        self.needs_render = true;
                    } else if let Some(picker) = &mut self.pending_memory_picker {
                        picker.query.insert_text(&text);
                        self.needs_render = true;
                    } else if self.global_search_open {
                        self.edit_global_search_query_text(&text);
                    } else if self.history_search.is_some() {
                        self.edit_history_search_query_text(&text);
                    } else {
                        // Split pasted text into image file paths and regular text.
                        // Image paths end with .png/.jpg/.jpeg/.gif/.webp.
                        let lines: Vec<&str> = text
                            .split(['\n', '\r'])
                            .flat_map(|part| {
                                // Also split on spaces preceding absolute paths.
                                part.split_inclusive(' ').collect::<Vec<_>>()
                            })
                            .collect();
                        let (img_lines, text_lines): (Vec<&str>, Vec<&str>) =
                            lines.iter().partition(|&&l| is_image_file_path(l));
                        let mut any_image = false;
                        for &path in &img_lines {
                            if let Some(img) = read_image_file(path.trim()) {
                                let label = img
                                    .filename
                                    .as_deref()
                                    .map(|f| format!("[Image: {f}]"))
                                    .unwrap_or_else(|| "[Image]".to_string());
                                self.prompt.insert_text(&label);
                                self.pending_images.push((img, label));
                                any_image = true;
                            }
                        }
                        let remaining = text_lines.join(" ");
                        let remaining = remaining.trim();
                        if !remaining.is_empty() {
                            self.prompt.insert_text(remaining);
                        }
                        if any_image || !remaining.is_empty() {
                            self.turn_state = TurnState::EditingInput;
                            self.state.input_mode = InputMode::Prompt;
                            self.reset_history_recall();
                            let attached_count = self.pending_images.len();
                            self.status_note = if any_image {
                                Some(format!("{} image(s) attached", attached_count))
                            } else {
                                None
                            };
                            self.needs_render = true;
                        } else {
                            self.prompt.insert_text(&text);
                            self.turn_state = TurnState::EditingInput;
                            self.state.input_mode = InputMode::Prompt;
                            self.reset_history_recall();
                            self.status_note = None;
                            self.needs_render = true;
                        }
                    }
                }
                Ok(())
            }
            UiEvent::Tick => {
                if self.has_active_turn() || is_loading_turn_state(self.turn_state) {
                    self.loading_frame = self.loading_frame.wrapping_add(1);
                    self.needs_render = true;
                } else {
                    self.loading_frame = 0;
                }
                // Drain any new lines from the shell progress channel.
                self.drain_progress_lines();
                match self.task_notice_ttl {
                    Some(0) => {
                        self.dismiss_task_notice();
                    }
                    Some(n) => {
                        self.task_notice_ttl = Some(n - 1);
                    }
                    None => {}
                }
                self.needs_render |= self.notifications.tick();
                // Push a notification when background memory extraction finishes.
                if !self.extraction.is_active() && self.extraction_was_active {
                    self.push_notification(
                        "memory-updated",
                        NotificationSeverity::Info,
                        "Memories updated",
                        std::iter::empty::<&str>(),
                        Some(2),
                        false,
                    );
                }
                self.extraction_was_active = self.extraction.is_active();
                if !self.cost_threshold_dialog_shown_session
                    && self.dialog.is_none()
                    && self.state.costs.estimated_cost_usd.unwrap_or(0.0) >= 5.0
                {
                    self.show_cost_threshold_dialog();
                }
                self.tick_copilot_oauth_poll()?;
                self.needs_render |= self.refresh_global_search_if_ready()?;
                self.tick_counter = self.tick_counter.wrapping_add(1);
                if self.tick_counter % 10 == 0 && self.refresh_runtime_state()? {
                    self.needs_render = true;
                }
                // Fire background agent-summary generation for running agent tasks.
                // Errors are silently ignored — summaries are best-effort display hints.
                if let Some(tm) = &self.task_manager {
                    tm.tick_agent_summaries(self.state.provider.clone(), self.state.model.clone());
                }
                if self.refresh_sidebar_panel_cache() {
                    self.needs_render = true;
                }
                Ok(())
            }
            UiEvent::FocusGained => {
                self.needs_render |= self.notifications.set_window_focused(true);
                self.needs_render = true;
                Ok(())
            }
            UiEvent::FocusLost => {
                self.needs_render |= self.notifications.set_window_focused(false);
                self.needs_render = true;
                Ok(())
            }
            UiEvent::Resize { width, height } => {
                self.needs_render = true;
                self.on_terminal_resize(width, height);
                Ok(())
            }
            UiEvent::Mouse(raw) => {
                self.handle_mouse_event(UiEvent::Mouse(raw));
                self.needs_render = true;
                Ok(())
            }
        }
    }
    pub(super) async fn poll_active_turn(&mut self) -> Result<bool> {
        if matches!(self.active_turn, ActiveTurn::None) {
            return Ok(false);
        }

        let mut turn = std::mem::replace(&mut self.active_turn, ActiveTurn::None);
        let result = match &mut turn {
            ActiveTurn::None => return Ok(false),
            ActiveTurn::Streaming(stream) => self.poll_streaming_step(stream).await,
            ActiveTurn::ToolLoop(tl) => self.poll_tool_loop_step(tl).await,
        };
        // Inner pollers signal completion by setting turn_state to Completed or
        // Interrupted. Streaming's Empty branch sets self.active_turn directly.
        // For all other cases where active_turn is still None (pending response,
        // awaiting approval), restore `turn` so subsequent ticks keep polling.
        if matches!(self.active_turn, ActiveTurn::None)
            && !matches!(
                self.turn_state,
                TurnState::Completed | TurnState::Interrupted
            )
        {
            self.active_turn = turn;
        }
        result
    }
    pub(super) async fn drain_queued_commands(&mut self) -> Result<()> {
        let mut drained = 0usize;
        while let Some(queued) = self.state.queued_commands.pop_front() {
            drained += 1;
            if drained > 8 {
                self.status_note = Some("queued command limit reached".into());
                self.needs_render = true;
                break;
            }
            self.persist_state_snapshot()?;
            let command = queued.command.trim().to_string();
            if command.is_empty() {
                continue;
            }
            if command.starts_with('/') {
                Box::pin(self.execute_slash_command_with(&command)).await?;
            } else {
                self.turn_state = TurnState::ModelRequestActive;
                self.status_note = Some("processing queued prompt".into());
                self.needs_render = true;
                self.execute_prompt_submission(&command).await?;
                if self.has_active_turn() {
                    // The prompt started a non-blocking ActiveTurn. Stop
                    // draining so the next queued prompt cannot clobber it;
                    // the event loop re-enters this drain once the turn
                    // finishes.
                    break;
                }
                if !matches!(self.turn_state, TurnState::ToolPermissionPending) {
                    self.turn_state = TurnState::Completed;
                }
            }
        }
        Ok(())
    }
    pub(super) fn tool_context(&self) -> ToolContext {
        let system_prompt = self.state.effective_system_prompt(None);
        ToolContext {
            session_id: self.state.session.id,
            cwd: self.state.session.cwd.clone(),
            session_worktree: self.state.session.worktree.clone(),
            permission_mode: self.state.permission_mode,
            additional_working_directories: self.state.additional_working_directories.clone(),
            provider: self.state.provider.clone(),
            model: self.state.model.clone(),
            permission_rules: Vec::new(),
            features: self.state.features.clone(),
            bash_session_store: None,
            progress_tx: None,
            interaction_rx: None,
            fork_context: build_fork_context_snapshot(&self.state, system_prompt.as_deref()),
            file_checkpointer: self.storage_dir.as_deref().map(|dir| {
                std::sync::Arc::new(wonder_of_u_storage::SessionFileCheckpointer::new(
                    dir,
                    self.state.session.id,
                )) as std::sync::Arc<dyn wonder_of_u_core::FileCheckpointer>
            }),
            network_policy: None,
        }
    }
    /// Returns the effective system prompt with the auto-memory section appended.
    ///
    /// Reads `MEMORY.md` from `~/.claude/projects/<git-root>/memory/MEMORY.md`
    /// and injects the memory instructions + index content into the system prompt
    /// so the model can read and write memories across sessions.
    pub(super) fn system_prompt_with_memory(&self) -> Option<String> {
        let base = self.state.effective_system_prompt(None);
        let memory_section = wonder_of_u_storage::memdir::build_auto_memory_section(
            &self.state.session.cwd,
            self.storage_dir.as_deref(),
        );
        Some(match base {
            Some(existing) => format!("{existing}\n\n{memory_section}"),
            None => memory_section,
        })
    }
    pub(super) fn command_context(&self) -> CommandContext {
        CommandContext {
            session_id: self.state.session.id,
            cwd: self.state.session.cwd.clone(),
            features: self.state.features.clone(),
            authenticated: self.state.provider_readiness() == ProviderReadiness::Ready,
            interactive: true,
            permission_mode: self.state.permission_mode,
            theme: self.state.theme.clone(),
            session_color: self.state.session_color.clone(),
            effort_level: self.state.effort_level.clone(),
            brief_mode: self.state.brief_mode,
            fast_mode: self.state.fast_mode,
            optimize_token_mode: self.state.optimize_token_mode,
            session_tags: self.state.session.tags.clone(),
            additional_working_directories: self.state.additional_working_directories.clone(),
        }
    }
    pub(super) fn push_notification(
        &mut self,
        key: impl Into<String>,
        severity: NotificationSeverity,
        title: impl Into<String>,
        body: impl IntoIterator<Item = impl Into<String>>,
        ttl: Option<u32>,
        focus: bool,
    ) {
        let lifetime = ttl.map_or(
            NotificationLifetime::Persistent,
            NotificationLifetime::Ticks,
        );
        self.notifications.push(
            NotificationInput::new(key, title, body)
                .severity(severity)
                .lifetime(lifetime)
                .focused(focus),
        );
        self.needs_render = true;
    }
    fn clear_picker_overlays(&mut self) {
        self.pending_permission_picker = None;
        self.pending_memory_picker = None;
        self.pending_tag_removal = None;
        self.pending_theme_picker = None;
        self.pending_model_picker = None;
        self.pending_setup_overlay = None;
        self.pending_output_style_picker = None;
        self.pending_memory_file_selector = None;
        self.pending_hooks_menu = None;
        self.fleet_panel = None;
    }
    pub(super) fn has_modal_overlay(&self) -> bool {
        self.dialog.is_some()
            || self.has_picker_overlay()
            || self.pending_copilot_oauth.is_some()
            || self.diff_dialog.is_some()
            || self.pending_export_dialog.is_some()
            || self.fleet_panel.is_some()
    }
    pub(super) fn is_interaction_free_text(&self) -> bool {
        self.interaction_question.is_some()
            && (self.interaction_options.is_empty() || self.interaction_other_mode)
    }
    pub(super) fn turn_state_for_prompt(&self) -> TurnState {
        if self.prompt.text().trim().is_empty() {
            if self.state.messages.is_empty() && self.state.background_tasks.is_empty() {
                TurnState::Idle
            } else {
                TurnState::Completed
            }
        } else {
            TurnState::EditingInput
        }
    }
    fn trigger_extract_memories(
        &mut self,
        resolved: &wonder_of_u_agent::ResolvedProviderExecution,
    ) {
        maybe_spawn_extract_memories(
            &mut self.extraction,
            &self.state.messages,
            resolved.clone(),
            &self.state.session.cwd,
            self.storage_dir.as_deref(),
        );
    }
    fn trigger_status_line(&self) {
        if let Some(cmd) = &self.status_line_command {
            maybe_run_status_line(&self.status_line_handle, cmd, &self.state);
        }
    }
    fn build_tool_registry_impl(
        storage_dir: Option<&Path>,
    ) -> Result<wonder_of_u_core::tool::ToolRegistry> {
        match storage_dir {
            Some(root) => wonder_of_u_tools::builtin_registry_with_mcp_catalog(root),
            None => wonder_of_u_tools::builtin_registry(),
        }
    }
    fn build_tool_registry(&self) -> Result<wonder_of_u_core::tool::ToolRegistry> {
        Self::build_tool_registry_impl(self.storage_dir.as_deref())
    }
    pub(super) fn has_active_turn(&self) -> bool {
        !matches!(self.active_turn, ActiveTurn::None)
    }
    #[allow(dead_code)]
    pub(super) fn active_overlay(&self) -> ActiveOverlay {
        if self.history_search.is_some() {
            return ActiveOverlay::HistorySearch;
        }
        if self.global_search_open {
            return ActiveOverlay::GlobalSearch;
        }
        if self.fleet_panel.is_some() {
            return ActiveOverlay::FleetPanel;
        }
        if self.has_picker_overlay() {
            return ActiveOverlay::Picker;
        }
        if let Some(dialog) = &self.dialog {
            return if dialog.actions.is_empty() {
                ActiveOverlay::NoticeDialog
            } else {
                ActiveOverlay::ConfirmDialog
            };
        }
        ActiveOverlay::None
    }
    pub(super) fn has_picker_overlay(&self) -> bool {
        self.pending_permission_picker.is_some()
            || self.pending_memory_picker.is_some()
            || self.pending_tag_removal.is_some()
            || self.pending_theme_picker.is_some()
            || self.pending_model_picker.is_some()
            || self.pending_setup_overlay.is_some()
            || self.pending_provider_form.is_some()
            || self.pending_message_selector.is_some()
            || self.pending_log_selector.is_some()
            || self.pending_output_style_picker.is_some()
            || self.pending_memory_file_selector.is_some()
            || self.pending_hooks_menu.is_some()
    }
    pub(super) fn current_picker_list_view(&self) -> Option<PickerListView> {
        const PICKER_HINT: &str = "↑↓ navigate  Tab/Enter select  Esc cancel";

        if let Some(picker) = &self.pending_model_picker {
            let filtered =
                filtered_picker_indices(&picker.query, &picker.options, |opt| opt.search_key());
            let mut last_provider: Option<&str> = None;
            return Some(PickerListView {
                title: "Select Model".into(),
                query: picker.query.text(),
                entries: filtered
                    .into_iter()
                    .filter_map(|index| picker.options.get(index))
                    .map(|opt| {
                        let group_header = if last_provider != Some(opt.provider.as_str()) {
                            last_provider = Some(opt.provider.as_str());
                            Some(format!("{} ({})", opt.provider_display, opt.auth))
                        } else {
                            None
                        };
                        let mut tag = String::new();
                        if opt.default {
                            tag.push_str("default");
                        }
                        if opt.selected {
                            if !tag.is_empty() {
                                tag.push_str(", ");
                            }
                            tag.push_str("current");
                        }
                        PickerListEntry {
                            label: opt.model_display.clone(),
                            description: String::new(),
                            tag: (!tag.is_empty()).then_some(tag),
                            selected: opt.model == picker.options[picker.selected_index].model
                                && opt.provider == picker.options[picker.selected_index].provider,
                            group_header,
                        }
                    })
                    .collect(),
                hint: PICKER_HINT.into(),
            });
        }
        if let Some(picker) = &self.pending_theme_picker {
            let filtered = filtered_picker_indices(&picker.query, &picker.options, |opt| {
                format!("{} {} {}", opt.theme, opt.label, opt.description)
            });
            return Some(PickerListView {
                title: "Select Theme".into(),
                query: picker.query.text(),
                entries: filtered
                    .into_iter()
                    .filter_map(|index| picker.options.get(index))
                    .map(|opt| PickerListEntry {
                        label: opt.label.clone(),
                        description: opt.description.clone(),
                        tag: opt.selected.then(|| "current".into()),
                        selected: opt.theme == picker.options[picker.selected_index].theme,
                        group_header: None,
                    })
                    .collect(),
                hint: PICKER_HINT.into(),
            });
        }
        if let Some(picker) = &self.pending_permission_picker {
            let filtered = filtered_picker_indices(&picker.query, &picker.options, |opt| {
                format!("{} {}", opt.label, opt.description)
            });
            let title = self
                .state
                .pending_tool_approval
                .as_ref()
                .map(|pending| {
                    format!(
                        "Tool: {} — allow?",
                        pending.pending_call.provider_call.tool_name
                    )
                })
                .unwrap_or_else(|| "Allow Tool?".into());
            return Some(PickerListView {
                title,
                query: picker.query.text(),
                entries: filtered
                    .into_iter()
                    .filter_map(|index| picker.options.get(index))
                    .map(|opt| PickerListEntry {
                        label: opt.label.clone(),
                        description: opt.description.clone(),
                        tag: opt.selected.then(|| "current".into()),
                        selected: opt.mode == picker.options[picker.selected_index].mode,
                        group_header: None,
                    })
                    .collect(),
                hint: PICKER_HINT.into(),
            });
        }
        if let Some(picker) = &self.pending_memory_picker {
            let filtered = filtered_picker_indices(&picker.query, &picker.options, |opt| {
                format!("{} {} {}", opt.label, opt.description, opt.path.display())
            });
            return Some(PickerListView {
                title: "Select Memory Target".into(),
                query: picker.query.text(),
                entries: filtered
                    .into_iter()
                    .filter_map(|index| picker.options.get(index))
                    .map(|opt| PickerListEntry {
                        label: opt.label.clone(),
                        description: format!("{} ({})", opt.description, opt.path.display()),
                        tag: opt.selected.then(|| "default".into()),
                        selected: opt.target == picker.options[picker.selected_index].target,
                        group_header: None,
                    })
                    .collect(),
                hint: PICKER_HINT.into(),
            });
        }
        if let Some(form) = &self.pending_provider_form {
            return Some(provider_form_picker_view(form));
        }
        if let Some(overlay) = &self.pending_setup_overlay {
            let readiness_hint = format!(
                "{PICKER_HINT}  ·  provider: {}  readiness: {}",
                overlay.provider_label, overlay.readiness_label
            );
            return Some(PickerListView {
                title: "Setup".into(),
                query: String::new(),
                entries: overlay
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| PickerListEntry {
                        label: item.label.clone(),
                        description: item.description.clone(),
                        tag: match &item.action {
                            SetupItemAction::Dispatch(_) => None,
                            SetupItemAction::Placeholder(_) => Some("coming soon".into()),
                            SetupItemAction::Deferred(_) => Some("deferred".into()),
                            SetupItemAction::ProviderForm(_) => None,
                            SetupItemAction::CopilotOAuth => None,
                        },
                        selected: i == overlay.selected_index,
                        group_header: None,
                    })
                    .collect(),
                hint: readiness_hint,
            });
        }
        if let Some(selector) = &self.pending_log_selector {
            return Some(PickerListView {
                title: "Select Session".into(),
                query: String::new(),
                entries: selector
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(i, entry)| {
                        let label = if entry.title.is_empty() {
                            entry.session_id.to_string()
                        } else {
                            entry.title.clone()
                        };
                        let time_label = relative_time_label(entry.updated_at);
                        let desc = format!("{} messages · {}", entry.message_count, time_label);
                        let tag = entry.tags.first().map(|t| format!("#{t}"));
                        PickerListEntry {
                            label,
                            description: desc,
                            tag,
                            selected: i == selector.selected_index,
                            group_header: None,
                        }
                    })
                    .collect(),
                hint: "↑↓/j/k navigate  Enter select  Esc cancel".into(),
            });
        }
        if let Some(selector) = &self.pending_message_selector {
            return Some(PickerListView {
                title: "Rewind to\u{2026}".into(),
                query: String::new(),
                entries: selector
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(i, entry)| PickerListEntry {
                        label: entry.preview.clone(),
                        description: String::new(),
                        tag: None,
                        selected: i == selector.selected_index,
                        group_header: None,
                    })
                    .collect(),
                hint: "\u{2191}\u{2193} navigate  Tab/Enter rewind  Esc cancel".into(),
            });
        }
        if let Some(picker) = &self.pending_export_dialog {
            if picker.mode == ExportDialogMode::PickOption {
                return Some(PickerListView {
                    title: "Export Session".into(),
                    query: String::new(),
                    entries: vec![
                        PickerListEntry {
                            label: "Copy to clipboard".into(),
                            description: "Copy full transcript as markdown".into(),
                            tag: None,
                            selected: picker.selected_index == 0,
                            group_header: None,
                        },
                        PickerListEntry {
                            label: "Save to file".into(),
                            description: "Write transcript to a file".into(),
                            tag: None,
                            selected: picker.selected_index == 1,
                            group_header: None,
                        },
                    ],
                    hint: "\u{2191}\u{2193} navigate  Enter select  Esc cancel".into(),
                });
            }
            // EnterFilename mode: show the filename buffer as query
            return Some(PickerListView {
                title: "Save Transcript As".into(),
                query: picker.filename.text().to_string(),
                entries: Vec::new(),
                hint: "Type filename  Enter confirm  Esc back".into(),
            });
        }
        if let Some(picker) = &self.pending_output_style_picker {
            return Some(PickerListView {
                title: "Select Output Style".into(),
                query: String::new(),
                entries: picker
                    .options
                    .iter()
                    .enumerate()
                    .map(|(i, opt)| PickerListEntry {
                        label: opt.name.to_string(),
                        description: opt.description.to_string(),
                        tag: (picker.current == opt.name).then(|| "current".into()),
                        selected: i == picker.selected_index,
                        group_header: None,
                    })
                    .collect(),
                hint: "\u{2191}\u{2193} navigate  Enter select  Esc cancel".into(),
            });
        }
        if let Some(picker) = &self.pending_memory_file_selector {
            return Some(PickerListView {
                title: "Select Memory File".into(),
                query: String::new(),
                entries: picker
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(i, entry)| {
                        let indent = "  ".repeat(entry.depth);
                        let exists_tag = if entry.exists {
                            None
                        } else {
                            Some("(new)".into())
                        };
                        PickerListEntry {
                            label: format!("{}{}", indent, entry.label),
                            description: entry.path.to_string_lossy().into_owned(),
                            tag: exists_tag,
                            selected: i == picker.selected_index,
                            group_header: None,
                        }
                    })
                    .collect(),
                hint: "\u{2191}\u{2193} navigate  Enter open  Esc cancel".into(),
            });
        }
        if let Some(menu) = &self.pending_hooks_menu {
            if menu.screen == HooksMenuScreen::Browse {
                if menu.entries.is_empty() {
                    return Some(PickerListView {
                        title: "Hooks Config".into(),
                        query: String::new(),
                        entries: vec![PickerListEntry {
                            label: if menu.disabled {
                                "All hooks disabled".into()
                            } else {
                                "No hooks configured".into()
                            },
                            description: format!("Config: {}", menu.config_path),
                            tag: None,
                            selected: false,
                            group_header: None,
                        }],
                        hint: "Esc close  /hooks open to configure".into(),
                    });
                }
                let mut last_event: Option<&str> = None;
                return Some(PickerListView {
                    title: "Hooks Config".into(),
                    query: String::new(),
                    entries: menu
                        .entries
                        .iter()
                        .enumerate()
                        .map(|(i, entry)| {
                            let group_header = if last_event != Some(entry.event.as_str()) {
                                last_event = Some(entry.event.as_str());
                                Some(entry.event.clone())
                            } else {
                                None
                            };
                            PickerListEntry {
                                label: format!(
                                    "{} → {}",
                                    if entry.matcher == "*" {
                                        "(all tools)".to_string()
                                    } else {
                                        entry.matcher.clone()
                                    },
                                    entry.target.chars().take(40).collect::<String>()
                                ),
                                description: format!("{} {}", entry.kind, entry.id),
                                tag: Some(match &entry.condition {
                                    Some(condition) => {
                                        format!("{} if {condition}", entry.status)
                                    }
                                    None => entry.status.clone(),
                                }),
                                selected: i == menu.selected_index,
                                group_header,
                            }
                        })
                        .collect(),
                    hint: "\u{2191}\u{2193} navigate  Enter details  Esc close".into(),
                });
            }
        }
        None
    }
    pub(super) fn should_confirm_exit(&self) -> bool {
        !self.prompt.text().trim().is_empty()
            || !self.state.messages.is_empty()
            || !self.state.background_tasks.is_empty()
    }
    pub(super) fn reset_history_recall(&mut self) {
        self.history_search = None;
        self.history_recall_index = None;
    }
    pub(super) fn needs_render(&self) -> bool {
        self.needs_render
    }
    /// Flips the sidebar panel on or off and sets a transient status note.
    pub(super) fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
        self.status_note = Some(if self.sidebar_visible {
            "sidebar on".into()
        } else {
            "sidebar off".into()
        });
        self.needs_render = true;
    }
    /// Flips inline tool output expansion and sets a transient status note.
    pub(super) fn toggle_expand_tool_output(&mut self) {
        self.expand_tool_output = !self.expand_tool_output;
        self.status_note = Some(if self.expand_tool_output {
            "tool output expanded".into()
        } else {
            "tool output collapsed".into()
        });
        self.notify_transcript_changed();
        self.needs_render = true;
    }
    pub(super) fn exit_requested(&self) -> bool {
        self.exit_requested
    }
    pub(super) fn take_external_editor_request(&mut self) -> Option<ExternalEditorRequest> {
        self.pending_external_editor.take()
    }
    pub(super) fn finish_external_editor_request(
        &mut self,
        request: &ExternalEditorRequest,
        result: Result<()>,
    ) {
        match result {
            Ok(()) => {
                self.status_note = Some(format!("edited {}", request.path.display()));
                self.needs_render = true;
            }
            Err(error) => {
                self.status_note = Some(format!("editor launch failed: {error}"));
                self.needs_render = true;
            }
        }
    }
    pub(super) fn mark_rendered(&mut self) {
        self.needs_render = false;
    }
    /// Recomputes the visible transcript height from terminal dimensions and
    /// calls [`TranscriptScrollState::on_resize`] to keep the scroll state
    /// consistent after the terminal is resized.
    ///
    /// Uses a rough prompt-height estimate derived from the current prompt text
    /// so that the scroll state stays accurate without building a full view.
    pub(super) fn on_terminal_resize(&mut self, width: u16, height: u16) {
        self.last_terminal_size = (width, height);
        // Mirror ShellView::prompt_height() + the renderer's one-third cap so
        // scroll_state.last_visible_lines matches the actual messages area.
        let uncapped = controller_prompt_height(&self.prompt.text());
        let cap = (height / 3).max(6); // matches (main_area.height / 3).max(6) in render_shell
        let warning_height = u16::from(self.context_warning_active());
        let prompt_height = uncapped
            .saturating_add(warning_height)
            .min(cap.saturating_add(warning_height));
        let layout = ShellLayout::split(
            Rect::new(
                0,
                0,
                shell_main_area_width(width, self.sidebar_visible),
                height,
            ),
            prompt_height,
        );
        let total = self.transcript_line_count(transcript_wrap_width(shell_main_area_width(
            width,
            self.sidebar_visible,
        )));
        // Subtract the loading row from visible height so the scroll max-offset
        // matches the actual renderable area (spinner sits on top of transcripts).
        let loading_row = u16::from(is_loading_turn_state(self.turn_state));
        let visible = layout.messages.height.saturating_sub(loading_row);
        self.scroll_state.on_resize(usize::from(visible), total);
    }
    /// Recomputes the total rendered transcript line count and notifies the
    /// scroll state so that follow-tail and scrolled-up modes remain correct.
    ///
    /// Call this after any operation that adds or removes messages from
    /// `self.state.messages`.
    pub(super) fn notify_transcript_changed(&mut self) {
        let total = self.transcript_line_count(transcript_wrap_width(shell_main_area_width(
            self.last_terminal_size.0.max(1),
            self.sidebar_visible,
        )));
        self.scroll_state.on_messages_changed(total);
    }
    pub(super) fn enter_selection_mode(&mut self) {
        if self.selection_mode {
            return;
        }
        self.selection_mode_sidebar_was_visible = self.sidebar_visible;
        self.sidebar_visible = false;
        self.selection_mode = true;
        self.status_note = Some("text selection — drag to select, press any key to resume".into());
        self.needs_render = true;
    }
    pub(super) fn exit_selection_mode(&mut self) {
        self.selection_mode = false;
        self.sidebar_visible = self.selection_mode_sidebar_was_visible;
        self.status_note = None;
        self.needs_render = true;
    }
}
