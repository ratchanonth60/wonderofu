use wonder_of_u_core::AppState;

use crate::{
    SpinnerMode, SpinnerView,
    dialog::DialogView,
    frame::{FrameBuffer, Rect},
    layout::ShellLayout,
    measure::widest_line,
    message::{
        HistorySearchView, MessageLineView, MessageRole, PickerListView, PickerView, SearchMatch,
        TaskPanelView, footer_text, message_lines, queued_panel_view, status_text, task_panel_view,
    },
    notification::{NotificationSeverity, NotificationView},
    style::{Color, TextStyle, Theme},
};

/// A single entry shown in the slash-command autocomplete overlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashSuggestionEntry {
    /// The text shown in the left column (e.g. `/status`).
    pub display: String,
    /// Short description shown in the right column.
    pub description: String,
    /// Whether this entry is currently highlighted.
    pub selected: bool,
}

/// State passed to the renderer when the slash-autocomplete overlay should be visible.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SlashSuggestionsOverlay {
    /// Stores the entries
    pub entries: Vec<SlashSuggestionEntry>,
}

/// State passed to the renderer when the workspace search overlay is open.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GlobalSearchOverlayView {
    /// Current query shown in the input row.
    pub query: String,
    /// Search matches shown in the result list.
    pub results: Vec<SearchMatch>,
    /// Highlighted result index.
    pub selected: usize,
}
/// Scroll metadata passed from the controller to the renderer each frame.
///
/// The renderer uses this to decide which slice of the transcript to display
/// and whether to show the "scrolled up" indicator in the status line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TranscriptScrollView {
    /// Lines from the bottom that have been scrolled away.
    /// `0` means "follow tail" (newest lines always visible).
    pub offset_from_bottom: usize,
    /// Total rendered transcript lines (used for the indicator count).
    pub total_lines: usize,
    /// Visible transcript rows available for rendering.
    pub visible_lines: usize,
}

impl TranscriptScrollView {
    /// Returns `true` when the view is pinned to the newest content.
    #[must_use]
    pub fn is_following_tail(self) -> bool {
        self.offset_from_bottom == 0
    }
}

/// Sectioned data model for the right-side companion panel shown on wide
/// terminals (≥ [`MIN_SIDEBAR_WIDTH`] columns).
///
/// Each field is one *section*; non-empty sections are rendered with a styled
/// section-header row (`"─ Name ─"`) followed by their body lines, separated by
/// blank rows.  Empty sections are silently omitted so callers do not need to
/// check before populating.
///
/// All strings are intentionally plain so the renderer has no coupling to
/// `wonder-of-u-core` types.  Special line prefixes drive extra colour:
///
/// | prefix | colour |
/// |--------|--------|
/// | `✓`    | green  |
/// | `⚠`    | yellow |
/// | `◈`    | `theme.prompt` (accent) |
///
/// ## Render order (OpenCode-style integration panel)
///
/// Status is rendered second (immediately below Session) so that the
/// real-time turn state is visible at a glance without scrolling.
///
/// ```text
/// ─ Session ─       (session_lines)
/// ─ Status ─        (status_lines)   ← promoted: most-urgent real-time info
/// ─ Context ─       (context_lines)
/// ─ Tools ─         (tool_lines)
/// ─ MCP ─           (mcp_lines)
/// ─ LSP ─           (lsp_lines)
/// ─ Todo ─          (todo_lines)
/// ─ Suggestions ─   (suggestions)
/// ─ Providers ─     (provider_lines)
/// ─ Workspace ─     (workspace_lines)
/// ─ Controls ─      (control_lines)
/// ─ Tasks ─         (task_lines)
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SidebarView {
    /// Section 1 – Session: turn state, title, status note etc.
    /// Each entry is one display row.
    pub session_lines: Vec<String>,
    /// Section 2 – Context: token/cost usage summary.
    pub context_lines: Vec<String>,
    /// Section 3 – Tools: active built-in tool names / call counts.
    ///
    /// Populated by the controller each frame from the live tool-use registry.
    /// Each entry is one display row, e.g. `"✓ Bash"` or `"⚠ FileWrite (3)"`.
    pub tool_lines: Vec<String>,
    /// Section 4 – MCP: connected MCP server names and connection state.
    ///
    /// Each entry is one display row, e.g. `"✓ filesystem"` or `"⚠ github (reconnecting)"`.
    pub mcp_lines: Vec<String>,
    /// Section 5 – LSP: active language-server diagnostics summary.
    ///
    /// Each entry is one display row, e.g. `"✓ rust-analyzer"` or `"⚠ 3 errors"`.
    pub lsp_lines: Vec<String>,
    /// Section 6 – Todo: in-session task checklist items.
    ///
    /// Each entry is one display row, e.g. `"✓ Write tests"` or `"◈ Refactor module"`.
    pub todo_lines: Vec<String>,
    /// Section 7 – Suggestions: proactive context-saving hints.
    pub suggestions: Vec<ContextSuggestion>,
    /// Section 8 – Providers: one entry per line, active model marked with `◈`.
    pub provider_lines: Vec<String>,
    /// Section 9 – Workspace: cwd, git, storage, runtime labels.
    pub workspace_lines: Vec<String>,
    /// Section 10 – Status: turn detail, loading verb, error snippets.
    pub status_lines: Vec<String>,
    /// Section 11 – Controls: compact keybindings.
    pub control_lines: Vec<String>,
    /// Section 12 – Tasks: background task count + hints.
    pub task_lines: Vec<String>,
}

/// Context-saving suggestions shown below the context visualization bar.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContextSuggestionsView {
    /// Suggestions shown in the panel.
    pub suggestions: Vec<ContextSuggestion>,
}

/// A single context-saving suggestion for the sidebar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextSuggestion {
    /// Severity used for icon and color treatment.
    pub severity: SuggestionSeverity,
    /// Short suggestion title.
    pub title: String,
    /// Follow-up detail explaining the action.
    pub detail: String,
}

/// Severity used when rendering a context suggestion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuggestionSeverity {
    /// The context window is nearing its limit.
    Warning,
    /// The context window is moderately full.
    Info,
}

/// Severity for the context warning banner above the prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptWarningSeverity {
    /// Context usage crossed the warning threshold.
    Warning,
    /// Context usage is close to exhaustion.
    Critical,
}

/// State for the context warning banner above the prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptWarningView {
    /// Banner text
    pub text: String,
    /// Banner severity
    pub severity: PromptWarningSeverity,
}

/// Represents shell view
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellView {
    /// Stores the title
    pub title: String,
    /// Stores the messages
    pub messages: Vec<MessageLineView>,
    /// Stores the prompt
    pub prompt: String,
    /// Stores the history search
    pub history_search: Option<HistorySearchView>,
    /// Stores the status
    pub status: String,
    /// Stores the loading
    pub loading: bool,
    /// Stores the loading verb
    pub loading_verb: Option<String>,
    /// Stores the animated spinner frame for transcript-local loading rows.
    pub spinner_frame: u64,
    /// Elapsed seconds since loading started; `0` when not loading.
    ///
    /// Shown in the Claude-style progress row as `· 27s` when non-zero.
    pub loading_elapsed_secs: u64,
    /// Current cumulative token count to show inside the loading progress row.
    ///
    /// `0` suppresses the token segment entirely.
    pub loading_total_tokens: u64,
    /// Stores the footer
    pub footer: String,
    /// Stores the queued panel
    pub queued_panel: Option<TaskPanelView>,
    /// Stores the task panel
    pub task_panel: Option<TaskPanelView>,
    /// Stores the dialog
    pub dialog: Option<DialogView>,
    /// Stores the picker view
    pub picker_view: Option<PickerView>,
    /// Stores the picker list
    pub picker_list: Option<PickerListView>,
    /// Stores the notifications
    pub notifications: Vec<NotificationView>,
    /// When `Some`, display the slash-command autocomplete overlay.
    pub slash_suggestions: Option<SlashSuggestionsOverlay>,
    /// When `Some`, display the workspace search overlay.
    pub global_search: Option<GlobalSearchOverlayView>,
    /// Scroll position snapshot for windowed transcript rendering.
    pub scroll: TranscriptScrollView,
    /// Right-side companion panel shown beside all shell content on wide terminals.
    ///
    /// `None` suppresses the panel entirely (e.g. when constructed manually in
    /// tests or when no provider context is available yet).
    pub sidebar: Option<SidebarView>,
    /// Warning banner shown immediately above the prompt when context usage is high.
    pub prompt_warning: Option<PromptWarningView>,
    /// Live stdout lines from a currently-executing shell tool.
    ///
    /// Shown in the loading area above the prompt.  Empty when no tool is running.
    pub tool_progress: Vec<String>,
    /// Index of the message that is currently highlighted by the message cursor.
    /// `None` when message cursor mode is not active.
    pub message_cursor_index: Option<usize>,
}

impl ShellView {
    /// Returns the height the prompt area should occupy.
    ///
    /// Rounded prompt layout: content rows plus top border, integrated footer row,
    /// and bottom border.
    /// Minimum height is 4 so the box can render a single input row and keep the
    /// dedicated footer surface visible on normal terminal sizes.
    #[must_use]
    pub fn prompt_height(&self) -> u16 {
        let line_count = prompt_body_line_count(self);
        u16::try_from(line_count)
            .unwrap_or(u16::MAX)
            .saturating_add(3)
            .max(4)
    }
    /// Handles from app state
    #[must_use]
    pub fn from_app_state(
        app: &AppState,
        prompt: impl Into<String>,
        expand_tool_output: bool,
    ) -> Self {
        let sidebar = {
            // Section 1 – Session: show a short session id prefix.
            let session_lines = vec![format!(
                "◈ {}",
                app.session
                    .id
                    .to_string()
                    .chars()
                    .take(8)
                    .collect::<String>()
            )];

            // Section 2 – Context: current token usage summary.
            let context_lines =
                context_sidebar_lines(app.costs.usage.total_tokens(), app.context_window_size);

            // Section 3 – Suggestions: proactive context-saving hints.
            let suggestions = context_suggestions(app);

            // Section 4 – Providers: one line combining provider and model.
            let provider_lines = match (&app.provider, &app.model) {
                (Some(provider), Some(model)) => vec![format!("{provider} · {model}")],
                _ => Vec::new(),
            };

            // Section 5 – Status: stub idle line; controller overwrites this each frame.
            let status_lines = vec!["● idle".into()];

            // Section 6 – Controls: compact keybinding reference.
            let control_lines = vec![
                "↵ send  ⇧↵ newline".into(),
                "⎋ cancel  ? help".into(),
                "⌃B sidebar  ⌃C exit".into(),
            ];

            // Section 7 – Workspace: git branch and short cwd label.
            let mut workspace_lines: Vec<String> = Vec::new();
            if let Some(branch) = &app.session.git_branch {
                workspace_lines.push(format!("⎇  {branch}"));
            }
            // Use the last path component as a short cwd label.
            if let Some(cwd) = app.session.cwd.file_name().and_then(|n| n.to_str()) {
                workspace_lines.push(format!("  {cwd}"));
            }

            // Section 8 – Tasks: background task count (omitted when none).
            let task_lines = if app.background_tasks.is_empty() {
                Vec::new()
            } else {
                let n = app.background_tasks.len();
                let label = if n == 1 { "task" } else { "tasks" };
                vec![format!("⚙  {n} {label}")]
            };

            SidebarView {
                session_lines,
                context_lines,
                // tool_lines / mcp_lines / lsp_lines / todo_lines are ephemeral;
                // the controller overwrites them from live registries each frame.
                tool_lines: Vec::new(),
                mcp_lines: Vec::new(),
                lsp_lines: Vec::new(),
                todo_lines: Vec::new(),
                suggestions,
                provider_lines,
                workspace_lines,
                status_lines,
                control_lines,
                task_lines,
            }
        };
        Self {
            title: format!("Session: {}", app.session.title),
            messages: message_lines(&app.messages, expand_tool_output),
            prompt: prompt.into(),
            history_search: None,
            status: status_text(app),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: footer_text(app),
            queued_panel: queued_panel_view(app),
            task_panel: task_panel_view(app),
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            // Default to follow-tail; the controller will override this each frame.
            scroll: TranscriptScrollView::default(),
            sidebar: Some(sidebar),
            prompt_warning: context_warning_banner(
                app.costs.usage.total_tokens(),
                app.context_window_size,
            ),
            tool_progress: Vec::new(),
            message_cursor_index: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StyledSpan {
    text: String,
    style: TextStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StyledLine {
    text: String,
    style: TextStyle,
    spans: Vec<StyledSpan>,
}

impl StyledLine {
    fn plain(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
            spans: Vec::new(),
        }
    }
}

enum DisplayRow {
    Header(String),
    Entry(usize),
}

include!("render_chunks/chunk_0.rs");
include!("render_chunks/chunk_1.rs");
include!("render_chunks/chunk_2.rs");
include!("render_chunks/tests.rs");
