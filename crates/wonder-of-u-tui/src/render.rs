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

/// Renders shell
pub fn render_shell(frame: &mut FrameBuffer, view: &ShellView, theme: &Theme) {
    let area = frame.area();
    frame.fill_rect(area, ' ', theme.background);

    // Wide-terminal sidebar: when the terminal is at least MIN_SIDEBAR_WIDTH columns
    // wide and the view carries sidebar data, carve out a right-hand column.
    // All main-content drawing (transcript + prompt box + chrome) is then confined
    // to the narrower left column — the prompt box keeps its full allocated width.
    let main_area = if let Some(sidebar) = &view.sidebar {
        if area.width >= MIN_SIDEBAR_WIDTH {
            // The sidebar box itself provides the left border, replacing the
            // old bare │ separator.  Give it SIDEBAR_WIDTH + 1 columns so the
            // left ╭/│/╰ aligns exactly where the separator used to be.
            let main_w = area.width.saturating_sub(SIDEBAR_WIDTH + 1);
            let sep_x = area.x.saturating_add(main_w);
            let sidebar_area = Rect::new(sep_x, area.y, SIDEBAR_WIDTH + 1, area.height);
            draw_shell_sidebar(frame, sidebar_area, sidebar, theme);
            Rect::new(area.x, area.y, main_w, area.height)
        } else {
            area
        }
    } else {
        area
    };

    // Cap prompt height at roughly one third of the terminal so chat/history
    // always dominates the display.  Minimum of 4 rows (top border + one content
    // line + integrated footer row + bottom border) keeps the dedicated prompt
    // footer visible without sacrificing the rounded outer chrome.
    let warning_height = u16::from(view.prompt_warning.is_some());
    let max_prompt = (main_area.height / 3).max(6).saturating_add(warning_height);
    let prompt_height = view
        .prompt_height()
        .saturating_add(warning_height)
        .min(max_prompt);
    let layout = ShellLayout::split(main_area, prompt_height);

    // When loading, reserve the bottom row of the message area for the Claude-style
    // progress row (e.g. `✱ thinking… · 05s · esc to interrupt`).  This keeps the
    // loading indicator visually close to the prompt without cluttering the transcript.
    let (messages_render_area, loading_row_area) = if view.loading && layout.messages.height >= 1 {
        // Reduce the transcript area by one row and carve out the loading row just
        // above the prompt box (at layout.messages.bottom() - 1).
        let transcript_h = layout.messages.height.saturating_sub(1);
        let loading_y = layout.messages.y.saturating_add(transcript_h);
        (
            Rect::new(
                layout.messages.x,
                layout.messages.y,
                layout.messages.width,
                transcript_h,
            ),
            Some(Rect::new(
                layout.messages.x,
                loading_y,
                layout.messages.width,
                1,
            )),
        )
    } else {
        (layout.messages, None)
    };

    draw_message_view(frame, messages_render_area, view, theme);

    // Draw the dedicated loading progress row, if active.
    if let Some(loading_area) = loading_row_area {
        draw_loading_progress_row(frame, loading_area, view, theme);
    }

    let prompt_area = if let Some(warning) = view
        .prompt_warning
        .as_ref()
        .filter(|_| layout.prompt.height > 1)
    {
        let warning_area = Rect::new(layout.prompt.x, layout.prompt.y, layout.prompt.width, 1);
        draw_prompt_warning(frame, warning_area, warning, theme);
        Rect::new(
            layout.prompt.x,
            layout.prompt.y.saturating_add(1),
            layout.prompt.width,
            layout.prompt.height.saturating_sub(1),
        )
    } else {
        layout.prompt
    };
    draw_prompt_view(frame, prompt_area, view, theme);
    // layout.status is a zero-height placeholder (CHROME_HEIGHT = 1); this is a
    // no-op but kept so callers that still pass status data are unaffected.
    draw_status_line(frame, layout.status, &status_line_text(view), theme.status);
    // Compact footer: combine the caller-supplied hint with a scroll indicator
    // when the user has scrolled up from tail.
    let footer_display = compact_footer_text(view);
    draw_footer_line(frame, layout.footer, &footer_display, theme);
    draw_notification_stack(frame, messages_render_area, &view.notifications, theme);

    if let Some(dialog) = &view.dialog {
        draw_dialog(frame, layout.messages, dialog, theme);
    }

    if let Some(picker_list) = &view.picker_list {
        draw_picker_list(frame, layout.messages, picker_list, theme);
    }

    if let Some(pv) = &view.picker_view {
        if let Some(preview) = &pv.preview {
            draw_picker_preview(frame, layout.messages.inset(1), preview, theme);
        }
    }

    if let Some(overlay) = &view.slash_suggestions {
        if !overlay.entries.is_empty() {
            draw_slash_suggestions(frame, messages_render_area, prompt_area, overlay, theme);
        }
    }

    if let Some(overlay) = &view.global_search {
        draw_global_search_overlay(
            frame,
            layout.messages,
            &overlay.query,
            &overlay.results,
            overlay.selected,
            theme,
        );
    }
}
/// Renders snapshot
#[must_use]
pub fn render_snapshot(width: u16, height: u16, view: &ShellView, theme: &Theme) -> FrameBuffer {
    let mut frame = FrameBuffer::new(width, height);
    render_shell(&mut frame, view, theme);
    frame
}

fn draw_panel(
    frame: &mut FrameBuffer,
    area: Rect,
    title: Option<&str>,
    lines: &[StyledLine],
    theme: &Theme,
) {
    if area.is_empty() {
        return;
    }

    frame.fill_rect(area, ' ', theme.background);
    if area.width >= 2 && area.height >= 2 {
        draw_rounded_border(frame, area, theme.border);
        if let Some(title) = title.filter(|title| !title.is_empty()) {
            let max_title_width = area.width.saturating_sub(4);
            frame.write_str(
                area.x.saturating_add(2),
                area.y,
                title,
                theme.title,
                max_title_width,
            );
        }

        let inner = area.inset(1);
        for (offset, line) in lines.iter().take(usize::from(inner.height)).enumerate() {
            let y = inner
                .y
                .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
            draw_styled_line(frame, inner.x, y, line, inner.width);
        }
        return;
    }

    if let Some(first_line) = lines.first() {
        draw_styled_line(frame, area.x, area.y, first_line, area.width);
    }
}

fn draw_message_view(frame: &mut FrameBuffer, area: Rect, view: &ShellView, theme: &Theme) {
    if area.is_empty() {
        return;
    }

    frame.fill_rect(area, ' ', theme.background);

    // Only show the session title header on the welcome / empty-state screen.
    // Once the user has sent messages the transcript starts at the very top of
    // the area (Claude Code fullscreen style) — no persistent chrome header.
    let title_height =
        u16::from(!view.title.is_empty() && view.messages.is_empty() && area.height > 0);
    if title_height == 1 {
        frame.write_str(
            area.x,
            area.y,
            &shell_header_text(&view.title),
            theme.title,
            area.width,
        );
    }

    let docked_lines = if view.dialog.is_some() {
        Vec::new()
    } else {
        docked_panel_lines(view, theme)
    };
    let docked_height = u16::try_from(docked_lines.len()).unwrap_or(area.height);
    let transcript_y = area.y.saturating_add(title_height);
    let transcript_height = area
        .height
        .saturating_sub(title_height)
        .saturating_sub(docked_height.min(area.height.saturating_sub(title_height)));
    let transcript_area = Rect::new(area.x, transcript_y, area.width, transcript_height);

    let msg_lines = message_panel_lines(view, theme);
    if view.messages.is_empty() && !view.loading {
        // Welcome screen: always top-anchored, never windowed.
        draw_lines(frame, transcript_area, &msg_lines);
    } else if view.scroll.is_following_tail() {
        draw_lines_tail(frame, transcript_area, &msg_lines);
    } else {
        draw_lines_windowed(
            frame,
            transcript_area,
            &msg_lines,
            view.scroll.offset_from_bottom,
        );
    }

    if docked_height == 0 || docked_height > area.height.saturating_sub(title_height) {
        return;
    }

    let dock_y = area.bottom().saturating_sub(docked_height);
    let dock_area = Rect::new(area.x, dock_y, area.width, docked_height);
    draw_lines(frame, dock_area, &docked_lines);
}

fn draw_prompt_view(frame: &mut FrameBuffer, area: Rect, view: &ShellView, theme: &Theme) {
    if area.is_empty() {
        return;
    }

    frame.fill_rect(area, ' ', theme.background);

    // Narrow / short fallback: skip the box chrome and write lines directly.
    if area.width < 3 || area.height < 4 {
        draw_lines(frame, area, &prompt_fallback_lines(view, theme));
        return;
    }

    draw_rounded_border(frame, area, theme.border);
    frame.write_str(
        area.x.saturating_add(2),
        area.y,
        " prompt ",
        theme.footer,
        area.width.saturating_sub(4),
    );

    let content = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(3),
    );
    draw_lines(frame, content, &prompt_panel_lines(view, theme));

    let footer_area = Rect::new(
        area.x.saturating_add(1),
        area.bottom().saturating_sub(2),
        area.width.saturating_sub(2),
        1,
    );
    draw_prompt_footer(frame, footer_area, view, theme);
}

fn draw_prompt_warning(
    frame: &mut FrameBuffer,
    area: Rect,
    warning: &PromptWarningView,
    theme: &Theme,
) {
    if area.is_empty() {
        return;
    }

    frame.fill_rect(area, ' ', theme.background);
    let style = match warning.severity {
        PromptWarningSeverity::Warning => TextStyle::default().fg(Color::Yellow).bold(),
        PromptWarningSeverity::Critical => TextStyle::default().fg(Color::Red).bold(),
    };
    frame.write_str(area.x, area.y, &warning.text, style, area.width);
}

/// Minimum total terminal width required to activate the shell-level sidebar.
///
/// Raised from 100 to 120 so common 100-column terminals keep the full
/// transcript width (Ink/Claude Code visual parity on standard terminals).
/// Below this threshold all content occupies the full terminal width.
pub const MIN_SIDEBAR_WIDTH: u16 = 120;

/// Column width of the sidebar panel (excluding the `│` separator).
pub const SIDEBAR_WIDTH: u16 = 36;

/// Returns the effective main-column width used by [`render_shell`].
///
/// When `has_sidebar` is `true` and `terminal_width` meets the
/// [`MIN_SIDEBAR_WIDTH`] threshold, the sidebar column (and the `│` separator)
/// are subtracted exactly as the renderer does.  Pass this result as the layout
/// width to cursor-position helpers so they stay in sync with the renderer.
///
/// # Examples
///
/// ```
/// use wonder_of_u_tui::{shell_main_area_width, MIN_SIDEBAR_WIDTH, SIDEBAR_WIDTH};
///
/// // Narrow terminal: no deduction regardless of sidebar flag.
/// assert_eq!(shell_main_area_width(80, true), 80);
///
/// // Wide terminal with sidebar: subtracts sidebar + separator.
/// assert_eq!(shell_main_area_width(120, true), 120 - SIDEBAR_WIDTH - 1);
///
/// // Wide terminal without sidebar: no deduction.
/// assert_eq!(shell_main_area_width(120, false), 120);
/// ```
pub fn shell_main_area_width(terminal_width: u16, has_sidebar: bool) -> u16 {
    if has_sidebar && terminal_width >= MIN_SIDEBAR_WIDTH {
        terminal_width.saturating_sub(SIDEBAR_WIDTH + 1)
    } else {
        terminal_width
    }
}

/// Draws the right-side sidebar showing keybinding hints and session metadata.
///
/// The sidebar is drawn inside a rounded-corner box (matching the prompt input
/// style) so it blends naturally with the rest of the chrome.  Content is
/// rendered inside the inner area (inset 1 on all sides).
fn draw_shell_sidebar(frame: &mut FrameBuffer, area: Rect, sidebar: &SidebarView, theme: &Theme) {
    if area.is_empty() {
        return;
    }

    // Rounded border — same style as the prompt box.
    draw_rounded_border(frame, area, theme.border);

    // Content inside the border (inset 1 on all sides).
    let inner = area.inset(1);
    if !inner.is_empty() {
        draw_lines(frame, inner, &sidebar_section_lines(sidebar, theme));
    }
}

/// Builds the ordered list of styled lines for the sidebar panel.
///
/// Renders sections in the OpenCode-style integration-panel order.
/// Status is promoted to position 2 so the live turn-state indicator
/// is always visible at the top without scrolling:
///
/// ```text
/// 1. Session      – session id / title
/// 2. Status       – turn state / loading verb  (promoted)
/// 3. Context      – token usage bar
/// 4. Tools        – active tool names / call counts
/// 5. MCP          – connected MCP server states
/// 6. LSP          – language-server diagnostics summary
/// 7. Todo         – in-session checklist items
/// 8. Suggestions  – proactive context-saving hints
/// 9. Providers    – model + provider
/// 10. Workspace   – git branch, cwd
/// 11. Controls    – compact keybinding reference
/// 12. Tasks       – background task count
/// ```
///
/// Each non-empty section is prefixed by a dim section-header row and followed
/// by a blank separator row.  Lines are clamped to [`SIDEBAR_WIDTH`] columns by
/// [`draw_lines`].  Special line-prefix characters drive extra colour:
///
/// | prefix | colour |
/// |--------|--------|
/// | `✓`    | `Color::Green` |
/// | `⚠`    | `Color::Yellow` |
/// | `◈`    | `theme.prompt` |
fn sidebar_section_lines(sidebar: &SidebarView, theme: &Theme) -> Vec<StyledLine> {
    let dim = theme.footer;
    let accent = theme.prompt;

    let mut out: Vec<StyledLine> = Vec::new();

    // 1 – Session
    push_sidebar_text_section(
        &mut out,
        "─ Session ─",
        &sidebar.session_lines,
        dim,
        accent,
        theme,
    );
    // 2 – Status (promoted: turn-state is the most urgent real-time signal)
    push_sidebar_text_section(
        &mut out,
        "─ Status ─",
        &sidebar.status_lines,
        dim,
        accent,
        theme,
    );
    // 3 – Context
    push_sidebar_text_section(
        &mut out,
        "─ Context ─",
        &sidebar.context_lines,
        dim,
        accent,
        theme,
    );
    // 4 – Tools
    push_sidebar_text_section(
        &mut out,
        "─ Tools ─",
        &sidebar.tool_lines,
        dim,
        accent,
        theme,
    );
    // 5 – MCP
    push_sidebar_text_section(&mut out, "─ MCP ─", &sidebar.mcp_lines, dim, accent, theme);
    // 6 – LSP
    push_sidebar_text_section(&mut out, "─ LSP ─", &sidebar.lsp_lines, dim, accent, theme);
    // 7 – Todo
    push_sidebar_text_section(
        &mut out,
        "─ Todo ─",
        &sidebar.todo_lines,
        dim,
        accent,
        theme,
    );
    // 8 – Suggestions (span-coloured; handled by its own helper)
    push_sidebar_suggestions_section(&mut out, &sidebar.suggestions, theme);
    // 9 – Providers
    push_sidebar_text_section(
        &mut out,
        "─ Providers ─",
        &sidebar.provider_lines,
        dim,
        accent,
        theme,
    );
    // 10 – Workspace
    push_sidebar_text_section(
        &mut out,
        "─ Workspace ─",
        &sidebar.workspace_lines,
        dim,
        accent,
        theme,
    );
    // 11 – Controls
    push_sidebar_text_section(
        &mut out,
        "─ Controls ─",
        &sidebar.control_lines,
        dim,
        accent,
        theme,
    );
    // 12 – Tasks
    push_sidebar_text_section(
        &mut out,
        "─ Tasks ─",
        &sidebar.task_lines,
        dim,
        accent,
        theme,
    );

    out
}

fn push_sidebar_text_section(
    out: &mut Vec<StyledLine>,
    header: &str,
    body: &[String],
    dim: TextStyle,
    accent: TextStyle,
    theme: &Theme,
) {
    if body.is_empty() {
        return;
    }

    if !out.is_empty() {
        out.push(StyledLine::plain(String::new(), dim));
    }

    out.push(StyledLine::plain(header.to_owned(), theme.title));
    for line in body {
        let style = if line.starts_with('✓') {
            TextStyle::default().fg(Color::Green)
        } else if line.starts_with('⚠') {
            TextStyle::default().fg(Color::Yellow)
        } else if line.starts_with('◈') {
            accent
        } else {
            dim
        };
        out.push(StyledLine::plain(line.clone(), style));
    }
}

fn push_sidebar_suggestions_section(
    out: &mut Vec<StyledLine>,
    suggestions: &[ContextSuggestion],
    theme: &Theme,
) {
    let Some(suggestion) = suggestions.first() else {
        return;
    };

    if !out.is_empty() {
        out.push(StyledLine::plain(String::new(), theme.footer));
    }

    out.push(StyledLine::plain("─ Suggestions ─".to_owned(), theme.title));
    out.push(StyledLine {
        text: String::new(),
        style: TextStyle::default(),
        spans: vec![
            StyledSpan {
                text: format!("{} ", suggestion_icon(suggestion.severity)),
                style: suggestion_icon_style(suggestion.severity, theme),
            },
            StyledSpan {
                text: suggestion.title.clone(),
                style: suggestion_title_style(suggestion.severity, theme),
            },
        ],
    });
    out.push(StyledLine::plain(
        format!("  {}", suggestion.detail),
        theme.footer,
    ));
}

fn docked_panel_lines(view: &ShellView, theme: &Theme) -> Vec<StyledLine> {
    let mut lines = Vec::new();

    if let Some(task_panel) = &view.task_panel {
        lines.extend(panel_lines(task_panel, theme));
    }

    if let Some(queued_panel) = &view.queued_panel {
        if !lines.is_empty() {
            lines.push(StyledLine::plain(String::new(), theme.background));
        }
        lines.extend(panel_lines(queued_panel, theme));
    }

    lines
}

fn panel_lines(panel: &TaskPanelView, theme: &Theme) -> Vec<StyledLine> {
    let mut lines = vec![StyledLine::plain(panel.title.clone(), theme.title)];
    lines.extend(message_lines_to_styled(&panel.lines, theme));
    lines
}

fn draw_lines(frame: &mut FrameBuffer, area: Rect, lines: &[StyledLine]) {
    if area.is_empty() {
        return;
    }

    for (offset, line) in lines.iter().take(usize::from(area.height)).enumerate() {
        let y = area
            .y
            .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        draw_styled_line(frame, area.x, y, line, area.width);
    }
}

fn draw_lines_tail(frame: &mut FrameBuffer, area: Rect, lines: &[StyledLine]) {
    if area.is_empty() {
        return;
    }

    let visible = usize::from(area.height);
    let start = lines.len().saturating_sub(visible);
    draw_lines(frame, area, &lines[start..]);
}

/// Renders a windowed slice of `lines` anchored by `offset_from_bottom` rows
/// above the tail.
///
/// `start = (total - visible).saturating_sub(offset_from_bottom)` — saturating
/// arithmetic ensures the view never scrolls past the first line.
fn draw_lines_windowed(
    frame: &mut FrameBuffer,
    area: Rect,
    lines: &[StyledLine],
    offset_from_bottom: usize,
) {
    if area.is_empty() {
        return;
    }

    let visible = usize::from(area.height);
    let start = lines
        .len()
        .saturating_sub(visible)
        .saturating_sub(offset_from_bottom);
    draw_lines(frame, area, &lines[start..]);
}

fn draw_styled_line(frame: &mut FrameBuffer, x: u16, y: u16, line: &StyledLine, max_width: u16) {
    if line.spans.is_empty() {
        frame.write_str(x, y, &line.text, line.style, max_width);
        return;
    }

    let mut cursor_x = x;
    let mut remaining = max_width;
    for span in &line.spans {
        if remaining == 0 {
            break;
        }
        for symbol in span.text.chars().take(usize::from(remaining)) {
            frame.put(cursor_x, y, symbol, span.style);
            cursor_x = cursor_x.saturating_add(1);
            remaining = remaining.saturating_sub(1);
            if remaining == 0 {
                break;
            }
        }
    }
}

fn draw_rule(frame: &mut FrameBuffer, x: u16, y: u16, width: u16, style: TextStyle) {
    for offset in 0..width {
        frame.put(x.saturating_add(offset), y, '─', style);
    }
}

fn draw_rounded_border(frame: &mut FrameBuffer, area: Rect, style: TextStyle) {
    if area.is_empty() {
        return;
    }

    if area.width == 1 && area.height == 1 {
        frame.put(area.x, area.y, '•', style);
        return;
    }

    if area.height == 1 {
        draw_rule(frame, area.x, area.y, area.width, style);
        return;
    }

    if area.width == 1 {
        for y in area.y..area.bottom() {
            frame.put(area.x, y, '│', style);
        }
        return;
    }

    let right = area.right().saturating_sub(1);
    let bottom = area.bottom().saturating_sub(1);

    frame.put(area.x, area.y, '╭', style);
    frame.put(right, area.y, '╮', style);
    frame.put(area.x, bottom, '╰', style);
    frame.put(right, bottom, '╯', style);

    for x in area.x.saturating_add(1)..right {
        frame.put(x, area.y, '─', style);
        frame.put(x, bottom, '─', style);
    }

    for y in area.y.saturating_add(1)..bottom {
        frame.put(area.x, y, '│', style);
        frame.put(right, y, '│', style);
    }
}

fn draw_dialog(frame: &mut FrameBuffer, viewport: Rect, dialog: &DialogView, theme: &Theme) {
    if viewport.width < 12 || viewport.height < 5 {
        return;
    }

    frame.fill_rect(viewport, ' ', theme.background);

    let actions = dialog.action_hint();
    let content_width = dialog
        .body
        .iter()
        .map(|line| widest_line(line))
        .chain(std::iter::once(widest_line(&dialog.title)))
        .chain(std::iter::once(widest_line(&actions)))
        .max()
        .unwrap_or(0);
    let width = u16::try_from(content_width.saturating_add(4))
        .unwrap_or(viewport.width)
        .max(viewport.width.saturating_sub(2))
        .min(viewport.width);
    // +4: top border (title) + blank separator + action row + bottom border.
    let height = u16::try_from(dialog.body.len().saturating_add(4))
        .unwrap_or(viewport.height)
        .min(viewport.height);
    let rect = Rect::new(
        viewport.x + viewport.width.saturating_sub(width) / 2,
        viewport.y + viewport.height.saturating_sub(height) / 2,
        width,
        height,
    );

    let mut body = plain_lines(&dialog.body, theme.messages);
    // Blank separator line gives visual breathing room before the action row.
    body.push(StyledLine::plain(String::new(), theme.footer));
    body.push(StyledLine::plain(actions, theme.status));
    draw_panel(frame, rect, Some(&dialog.title), &body, theme);
}

fn draw_notification_stack(
    frame: &mut FrameBuffer,
    viewport: Rect,
    notifications: &[NotificationView],
    theme: &Theme,
) {
    if notifications.is_empty() || viewport.width < 18 || viewport.height < 3 {
        return;
    }
    let mut bottom = viewport.bottom();
    for notification in notifications.iter().rev() {
        let title = notification_title(notification);
        let lines = notification_panel_lines(notification, theme);
        let content_width = lines
            .iter()
            .map(|line| widest_line(&line.text))
            .chain(std::iter::once(widest_line(&title)))
            .max()
            .unwrap_or(0);
        let width = u16::try_from(content_width.saturating_add(4))
            .unwrap_or(viewport.width)
            .clamp(18, viewport.width);
        let height = u16::try_from(lines.len().saturating_add(2)).unwrap_or(viewport.height);
        if height > bottom.saturating_sub(viewport.y) {
            break;
        }

        let y = bottom.saturating_sub(height);
        let rect = Rect::new(viewport.right().saturating_sub(width), y, width, height);
        draw_notification(frame, rect, &title, notification, &lines, theme);
        bottom = y;
    }
}

fn draw_notification(
    frame: &mut FrameBuffer,
    area: Rect,
    title: &str,
    notification: &NotificationView,
    lines: &[StyledLine],
    theme: &Theme,
) {
    if area.is_empty() {
        return;
    }

    let border = notification_border_style(notification);
    let panel_theme = Theme {
        background: theme.background,
        border,
        title: border.bold(),
        messages: theme.messages,
        prompt: theme.prompt,
        status: theme.status,
        footer: theme.footer,
    };
    draw_panel(frame, area, Some(title), lines, &panel_theme);
}

fn notification_title(notification: &NotificationView) -> String {
    let title = format!("{} {}", notification.severity.label(), notification.title);
    if notification.focused {
        format!("{title} • focus")
    } else {
        title
    }
}

fn notification_panel_lines(notification: &NotificationView, theme: &Theme) -> Vec<StyledLine> {
    notification
        .lines
        .iter()
        .map(|line| {
            StyledLine::plain(
                line.clone(),
                if notification.focused {
                    theme.messages.bold()
                } else {
                    theme.messages
                },
            )
        })
        .collect()
}

fn notification_border_style(notification: &NotificationView) -> TextStyle {
    let accent = match notification.severity {
        NotificationSeverity::Info => Color::Cyan,
        NotificationSeverity::Success => Color::Green,
        NotificationSeverity::Warning => Color::Yellow,
        NotificationSeverity::Error => Color::Red,
    };
    let style = TextStyle::default().fg(accent);
    if notification.focused {
        style.bold()
    } else {
        style
    }
}

/// Draws a small bordered "Preview" panel anchored to the bottom of `viewport`.
///
/// The panel is only rendered when there is enough vertical space so it does
/// not collide with the dialog that sits above it.
fn draw_picker_preview(frame: &mut FrameBuffer, viewport: Rect, preview: &str, theme: &Theme) {
    const PREVIEW_HEIGHT: u16 = 3; // top border + one content row + bottom border
    if viewport.width < 12 || viewport.height < PREVIEW_HEIGHT {
        return;
    }
    let preview_width = u16::try_from(widest_line(preview).saturating_add(4))
        .unwrap_or(viewport.width)
        .max(viewport.width.saturating_sub(2))
        .min(viewport.width);
    let rect = Rect::new(
        viewport.x + viewport.width.saturating_sub(preview_width) / 2,
        viewport.y + viewport.height.saturating_sub(PREVIEW_HEIGHT),
        preview_width,
        PREVIEW_HEIGHT,
    );
    draw_panel(
        frame,
        rect,
        Some("Preview"),
        &[StyledLine::plain(preview, theme.messages)],
        theme,
    );
}

fn draw_slash_suggestions(
    frame: &mut FrameBuffer,
    viewport: Rect,
    prompt_area: Rect,
    overlay: &SlashSuggestionsOverlay,
    theme: &Theme,
) {
    const MAX_VISIBLE: usize = 8;
    const MIN_WIDTH: u16 = 30;

    let entries = &overlay.entries;
    let visible_count = entries.len().min(MAX_VISIBLE);
    if visible_count == 0 || viewport.width < MIN_WIDTH || viewport.height < 3 {
        return;
    }

    // Compute panel dimensions.
    let content_width = entries
        .iter()
        .take(MAX_VISIBLE)
        .map(|e| {
            let desc_part = if e.description.is_empty() {
                0
            } else {
                e.description.len() + 3 // " ─ " separator
            };
            e.display.len() + desc_part
        })
        .max()
        .unwrap_or(0);
    let panel_width = u16::try_from(content_width.saturating_add(4))
        .unwrap_or(viewport.width)
        .clamp(MIN_WIDTH, viewport.width);
    let panel_height = u16::try_from(visible_count.saturating_add(2))
        .unwrap_or(viewport.height)
        .min(viewport.height);

    // Anchor the overlay to the prompt so it reads as an autocomplete surface
    // instead of another transcript panel.
    let overlay_bottom = prompt_area.y.max(viewport.y.saturating_add(panel_height));
    let rect = Rect::new(
        viewport.x,
        overlay_bottom.saturating_sub(panel_height),
        panel_width,
        panel_height,
    );

    let lines: Vec<StyledLine> = entries
        .iter()
        .take(MAX_VISIBLE)
        .map(|e| {
            let text = if e.description.is_empty() {
                e.display.clone()
            } else {
                format!("{} ─ {}", e.display, e.description)
            };
            StyledLine::plain(
                text,
                if e.selected {
                    theme.prompt.bold()
                } else {
                    theme.messages
                },
            )
        })
        .collect();

    let border_theme = Theme {
        border: theme.prompt,
        title: theme.prompt.bold(),
        ..*theme
    };
    draw_modal_shadow(frame, rect, viewport, theme);
    draw_panel(frame, rect, Some("commands"), &lines, &border_theme);
}

fn draw_picker_list(
    frame: &mut FrameBuffer,
    viewport: Rect,
    picker: &PickerListView,
    theme: &Theme,
) {
    const MIN_WIDTH: u16 = 30;
    const MIN_HEIGHT: u16 = 6;

    if viewport.width < MIN_WIDTH || viewport.height < MIN_HEIGHT {
        return;
    }

    let width = viewport.width.saturating_mul(4) / 5;
    let width = width.clamp(MIN_WIDTH, viewport.width);
    let max_height = viewport.height.saturating_mul(3) / 5;
    let desired_height =
        u16::try_from(picker.entries.len().saturating_add(4)).unwrap_or(viewport.height);
    let height = desired_height.clamp(MIN_HEIGHT, max_height.max(MIN_HEIGHT));
    let rect = Rect::new(
        viewport.x + viewport.width.saturating_sub(width) / 2,
        viewport.y + viewport.height.saturating_sub(height) / 2,
        width,
        height,
    );

    draw_modal_shadow(frame, rect, viewport, theme);

    let panel_theme = Theme {
        border: theme.prompt,
        title: theme.prompt.bold(),
        ..*theme
    };
    draw_panel(frame, rect, Some(&picker.title), &[], &panel_theme);

    let inner = rect.inset(1);
    if inner.is_empty() {
        return;
    }

    frame.write_str(
        inner.x,
        inner.y,
        &format!("Search: {}", picker.query),
        theme.status,
        inner.width,
    );

    if inner.height <= 2 {
        return;
    }

    let hint_y = inner.bottom().saturating_sub(1);
    frame.write_str(inner.x, hint_y, &picker.hint, theme.footer, inner.width);

    let list_area = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(2),
    );
    if list_area.is_empty() {
        return;
    }

    if picker.entries.is_empty() {
        frame.write_str(
            list_area.x,
            list_area.y,
            "No matches.",
            theme.messages,
            list_area.width,
        );
        return;
    }

    let selected = picker
        .entries
        .iter()
        .position(|entry| entry.selected)
        .unwrap_or_default();
    let visible = usize::from(list_area.height);
    let scroll_padding = visible / 2;
    let start = selected
        .saturating_sub(scroll_padding)
        .min(picker.entries.len().saturating_sub(visible.max(1)));

    for (offset, entry) in picker.entries.iter().skip(start).take(visible).enumerate() {
        let y = list_area
            .y
            .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));

        // Build the optional right-aligned tag badge, e.g. "[current]".
        // This mirrors the upstream Ink ✓-after-label pattern: the badge
        // sits at the right edge of the row so it never crowds the label.
        let tag_str: Option<String> = entry
            .tag
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(|t| format!("[{t}]"));

        // Tag width in columns (ASCII-only assumption is safe for our tag values).
        let tag_cols = tag_str
            .as_deref()
            .map_or(0_u16, |t| u16::try_from(t.chars().count()).unwrap_or(0));

        // Budget for the label+description part: leave 1-space gap before the
        // tag (only when a tag is actually present).
        let main_budget = if tag_cols > 0 {
            list_area.width.saturating_sub(tag_cols + 1)
        } else {
            list_area.width
        };

        let cursor = if entry.selected { "▸ " } else { "  " };
        let label = &entry.label;
        let description = if entry.description.is_empty() {
            String::new()
        } else {
            format!(" — {}", entry.description)
        };

        // Compose: cursor + label + description, truncated to main_budget.
        let main_text: String = format!("{cursor}{label}{description}")
            .chars()
            .take(usize::from(main_budget))
            .collect();

        let label_style = if entry.selected {
            theme.prompt.reversed().bold()
        } else {
            theme.messages
        };
        // Description uses a dim style on non-selected rows to match the
        // upstream two-column layout where descriptions are rendered dimColor.
        let desc_style = if entry.selected {
            theme.prompt.reversed().bold()
        } else {
            theme.footer
        };
        // Tag badge uses the dim footer style (non-selected) or reverses to
        // match the selection highlight background (selected).
        let tag_style = if entry.selected {
            theme.prompt.reversed().bold()
        } else {
            theme.footer
        };

        // Fill selection background across the whole row.
        if entry.selected {
            frame.fill_rect(
                Rect::new(list_area.x, y, list_area.width, 1),
                ' ',
                theme.prompt.reversed().bold(),
            );
        }

        // Write cursor + label in the primary style.
        let cursor_label = format!("{cursor}{label}");
        let cursor_label_cols =
            u16::try_from(cursor_label.chars().count()).unwrap_or(list_area.width);
        frame.write_str(list_area.x, y, &cursor_label, label_style, main_budget);

        // Write description in dim style immediately after the label.
        if !description.is_empty() {
            let desc_x = list_area.x.saturating_add(cursor_label_cols);
            // Guard against overflowing into the tag column.
            let desc_budget = main_budget.saturating_sub(cursor_label_cols);
            if desc_budget > 0 {
                let desc_text: String =
                    description.chars().take(usize::from(desc_budget)).collect();
                frame.write_str(desc_x, y, &desc_text, desc_style, desc_budget);
            }
        }

        // Write right-aligned tag badge at the far right of the row.
        if let Some(tag) = tag_str {
            // +1 for the mandatory gap space.
            let tag_x = list_area.x + list_area.width - tag_cols;
            frame.write_str(tag_x, y, &tag, tag_style, tag_cols);
        }

        // Fallback: if there is no description and no tag, ensure the label
        // isn't partially truncated by the budget calculation.  `main_text` is
        // kept for the no-description, no-tag path (tag_cols == 0 ⟹
        // main_budget == list_area.width, so no data is lost).
        let _ = main_text; // silences the unused-variable lint
    }
}

/// Draws the centered workspace search overlay.
///
/// The overlay reserves a title row and query row, then renders a truncated
/// list of `file:line` matches beneath them.
pub fn draw_global_search_overlay(
    frame: &mut FrameBuffer,
    viewport: Rect,
    query: &str,
    results: &[SearchMatch],
    selected: usize,
    theme: &Theme,
) {
    const MIN_WIDTH: u16 = 40;
    const MIN_HEIGHT: u16 = 8;

    if viewport.width < MIN_WIDTH || viewport.height < MIN_HEIGHT {
        return;
    }

    let width = (viewport.width.saturating_mul(4) / 5).clamp(MIN_WIDTH, viewport.width);
    let height = (viewport.height.saturating_mul(3) / 5).clamp(MIN_HEIGHT, viewport.height);
    let rect = Rect::new(
        viewport.x + viewport.width.saturating_sub(width) / 2,
        viewport.y + viewport.height.saturating_sub(height) / 2,
        width,
        height,
    );

    draw_modal_shadow(frame, rect, viewport, theme);

    let panel_theme = Theme {
        border: theme.prompt,
        title: theme.prompt.bold(),
        ..*theme
    };
    draw_panel(
        frame,
        rect,
        Some("Search workspace (ctrl+f)"),
        &[],
        &panel_theme,
    );

    let inner = rect.inset(1);
    if inner.is_empty() {
        return;
    }

    frame.write_str(
        inner.x,
        inner.y,
        &format!("Query: {query}"),
        theme.status,
        inner.width,
    );

    if inner.height <= 1 {
        return;
    }

    let list_area = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(1),
    );
    if list_area.is_empty() {
        return;
    }

    let lines = if results.is_empty() {
        let message = if query.trim().is_empty() {
            "Type to search workspace."
        } else {
            "No matches."
        };
        vec![StyledLine::plain(message, theme.messages)]
    } else {
        results
            .iter()
            .enumerate()
            .take(usize::from(list_area.height))
            .map(|(index, result)| {
                StyledLine::plain(
                    format_global_search_result(result, list_area.width),
                    if index == selected.min(results.len().saturating_sub(1)) {
                        theme.prompt.reversed().bold()
                    } else {
                        theme.messages
                    },
                )
            })
            .collect()
    };
    draw_lines(frame, list_area, &lines);
}

fn draw_modal_shadow(frame: &mut FrameBuffer, rect: Rect, viewport: Rect, theme: &Theme) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let shadow_style = TextStyle::default()
        .bg(Color::DarkGrey)
        .fg(Color::DarkGrey)
        .dim();
    let shadow = Rect::new(
        rect.x
            .saturating_add(1)
            .min(viewport.right().saturating_sub(1)),
        rect.y
            .saturating_add(1)
            .min(viewport.bottom().saturating_sub(1)),
        rect.width
            .min(viewport.right().saturating_sub(rect.x.saturating_add(1))),
        rect.height
            .min(viewport.bottom().saturating_sub(rect.y.saturating_add(1))),
    );
    if !shadow.is_empty() {
        frame.fill_rect(shadow, ' ', shadow_style);
    }
    frame.fill_rect(rect, ' ', theme.background);
}

fn draw_status_line(frame: &mut FrameBuffer, area: Rect, text: &str, style: TextStyle) {
    if area.is_empty() {
        return;
    }

    frame.fill_rect(area, ' ', style);
    frame.write_str(area.x, area.y, text, style, area.width);
}

fn draw_footer_line(frame: &mut FrameBuffer, area: Rect, text: &str, theme: &Theme) {
    if area.is_empty() {
        return;
    }

    frame.fill_rect(area, ' ', theme.background);
    let badges = footer_badges(text);
    let max_width = usize::from(area.width);
    let display = if badges.len() > max_width {
        text.to_string()
    } else {
        badges
    };
    let display_width = u16::try_from(display.chars().count()).unwrap_or(area.width);
    let x = area
        .right()
        .saturating_sub(display_width.min(area.width))
        .min(area.x.saturating_add(area.width.saturating_sub(1)));
    frame.write_str(x, area.y, &display, theme.footer, area.width);
}

fn footer_badges(text: &str) -> String {
    let badges = text
        .split('|')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| format!(" {segment} "))
        .collect::<Vec<_>>();
    badges.join("·")
}

fn status_line_text(view: &ShellView) -> String {
    let indicator = if view.loading { '●' } else { '○' };
    let base = if view.status.is_empty() {
        indicator.to_string()
    } else {
        format!("{indicator} {}", view.status)
    };

    // Append a compact scroll indicator when the user has scrolled up from tail.
    if !view.scroll.is_following_tail() {
        format!(
            "{}  ↑ {} lines · Ctrl+End bottom",
            base, view.scroll.offset_from_bottom
        )
    } else {
        base
    }
}

/// Builds the single compact footer row displayed at the bottom of the shell.
///
/// Combines the caller-supplied hint text from [`ShellView::footer`] with a
/// lightweight scroll indicator when the transcript is scrolled up from tail.
/// The scroll indicator is prepended so it appears on the left (or near the
/// left after right-alignment in [`draw_footer_line`]).
fn compact_footer_text(view: &ShellView) -> String {
    if view.scroll.is_following_tail() {
        view.footer.clone()
    } else {
        let scroll_badge = format!(
            "↑ {} lines · Ctrl+End bottom",
            view.scroll.offset_from_bottom
        );
        if view.footer.is_empty() {
            scroll_badge
        } else {
            format!("{scroll_badge} | {}", view.footer)
        }
    }
}

fn shell_header_text(title: &str) -> String {
    let session = title.strip_prefix("Session: ").unwrap_or(title).trim();
    if session.is_empty() {
        "▸ wonder-of-u".into()
    } else {
        format!("▸ wonder-of-u  {session}")
    }
}

fn message_panel_lines(view: &ShellView, theme: &Theme) -> Vec<StyledLine> {
    if view.messages.is_empty() {
        welcome_panel_lines(theme)
    } else {
        message_lines_to_styled(&view.messages, theme)
    }
}

/// Renders the Claude-style progress row directly above the prompt box.
///
/// Format: `{spinner} {verb}… · {elapsed} · {tokens} tokens · esc to interrupt`
///
/// # Layout
///
/// The row is a single terminal line carved from the bottom of the message area
/// by [`render_shell`] whenever `view.loading` is `true`.  It degrades gracefully
/// on narrow terminals — the text is simply clipped at `area.width`.
fn draw_loading_progress_row(frame: &mut FrameBuffer, area: Rect, view: &ShellView, theme: &Theme) {
    if area.is_empty() {
        return;
    }

    let text = build_loading_progress_text(view);
    frame.fill_rect(area, ' ', theme.background);
    // Dim progress style — matches the transcript-level progress role colour.
    let style = {
        let mut s = theme.status;
        s.dim = true;
        s
    };
    frame.write_str(area.x, area.y, &text, style, area.width);
}

/// Builds the plain-text content of the loading progress row.
///
/// Produces a spinner-animated string of the form:
/// `{glyph} {verb}… · {elapsed} · {tokens} tokens · esc to interrupt`
///
/// The elapsed and token segments are omitted when their values are zero.
fn build_loading_progress_text(view: &ShellView) -> String {
    let mode = match view.loading_verb.as_deref() {
        Some("running") => SpinnerMode::Requesting,
        Some("waiting") => SpinnerMode::Stalled,
        _ => SpinnerMode::Thinking,
    };
    let verb = view.loading_verb.as_deref().unwrap_or("thinking");
    let mut spinner = SpinnerView::new(mode, format!("{verb}…"))
        .frame(view.spinner_frame)
        .suffix("esc to interrupt");

    // Include elapsed time when loading has been active for at least 1 second.
    if view.loading_elapsed_secs > 0 {
        spinner = spinner.elapsed_ms(view.loading_elapsed_secs.saturating_mul(1_000));
    }

    // Include cumulative token count when non-zero.
    if view.loading_total_tokens > 0 {
        #[allow(clippy::cast_possible_truncation)]
        let token_count = view.loading_total_tokens.min(usize::MAX as u64) as usize;
        spinner = spinner.token_count(token_count);
    }

    spinner.render_line()
}

fn draw_prompt_footer(frame: &mut FrameBuffer, area: Rect, view: &ShellView, theme: &Theme) {
    if area.is_empty() {
        return;
    }

    let style = if view.history_search.is_some() {
        theme.status
    } else {
        theme.footer
    };
    frame.write_str(area.x, area.y, &prompt_footer_text(view), style, area.width);
}

fn welcome_panel_lines(theme: &Theme) -> Vec<StyledLine> {
    let art = theme.title;
    let dim = theme.footer;
    let body = theme.messages;
    macro_rules! l {
        ($s:expr, $st:expr) => {
            StyledLine::plain($s, $st)
        };
    }
    vec![
        l!("", body),
        l!("   ██╗    ██╗  ██████╗  ██╗   ██╗", art),
        l!("   ██║    ██║ ██╔═══██╗ ██║   ██║", art),
        l!("   ██║ █╗ ██║ ██║   ██║ ██║   ██║", art),
        l!("   ██║███╗██║ ██║   ██║ ╚██╗ ██╔╝", art),
        l!("   ╚███╔███╔╝ ╚██████╔╝  ╚████╔╝ ", art),
        l!("    ╚══╝╚══╝   ╚═════╝    ╚═══╝  ", art),
        l!("", body),
        l!("     wonder-of-u  ·  AI coding assistant", dim),
        l!("", body),
        l!("   ╭────────────────────────────────────╮", dim),
        l!("   │  Type a message to get started     │", dim),
        l!("   │  /  for slash commands             │", dim),
        l!("   │  ?  for help                       │", dim),
        l!("   │  Shift+Enter  insert newline        │", dim),
        l!("   ╰────────────────────────────────────╯", dim),
        l!("", body),
    ]
}

fn message_lines_to_styled(lines: &[MessageLineView], theme: &Theme) -> Vec<StyledLine> {
    lines
        .iter()
        .map(|line| {
            let base_style = style_for_message(theme, line.role);
            let spans = line
                .spans
                .iter()
                .map(|span| StyledSpan {
                    text: span.text.clone(),
                    style: span.style.unwrap_or(base_style),
                })
                .collect();
            StyledLine {
                text: line.text.clone(),
                style: base_style,
                spans,
            }
        })
        .collect()
}

fn prompt_fallback_lines(view: &ShellView, theme: &Theme) -> Vec<StyledLine> {
    let mut lines = prompt_panel_lines(view, theme);
    lines.push(StyledLine::plain(
        prompt_footer_text(view),
        if view.history_search.is_some() {
            theme.status
        } else {
            theme.footer
        },
    ));
    lines
}

fn prompt_panel_lines(view: &ShellView, theme: &Theme) -> Vec<StyledLine> {
    let Some(search) = &view.history_search else {
        let mut lines = split_lines(&view.prompt);
        if let Some(first) = lines.first_mut() {
            *first = format!("› {first}");
        }
        return plain_lines(&lines, theme.prompt);
    };

    plain_lines(
        &split_lines(search.match_text.as_deref().unwrap_or(&view.prompt)),
        theme.prompt,
    )
}

fn prompt_footer_text(view: &ShellView) -> String {
    const DEFAULT_HINTS: &str = "Enter send · Shift+Enter newline · / commands · Ctrl+R history";
    const SLASH_HINTS: &str = "slash commands · ↑↓ select · Tab/Enter apply · Esc cancel";
    const HISTORY_ACCEPT_HINTS: &str = "↑↓/Ctrl+R cycle · Enter accept · Esc cancel";

    if let Some(search) = &view.history_search {
        let query = if search.query.is_empty() {
            "type to search".into()
        } else {
            search.query.clone()
        };
        return format!(
            "history: {query} · {} · {HISTORY_ACCEPT_HINTS}",
            history_match_label(search)
        );
    }

    if view
        .slash_suggestions
        .as_ref()
        .is_some_and(|overlay| !overlay.entries.is_empty())
    {
        return SLASH_HINTS.into();
    }

    if view.status.is_empty() {
        DEFAULT_HINTS.into()
    } else {
        let status = prompt_status_summary(&view.status);
        if status.is_empty() {
            DEFAULT_HINTS.into()
        } else {
            format!("{status} · {DEFAULT_HINTS}")
        }
    }
}

fn prompt_body_line_count(view: &ShellView) -> usize {
    match &view.history_search {
        Some(search) => text_line_count(search.match_text.as_deref().unwrap_or(&view.prompt)),
        None => text_line_count(&view.prompt),
    }
}

fn prompt_status_summary(status: &str) -> String {
    status
        .split('|')
        .map(str::trim)
        .filter(|segment| {
            !segment.is_empty() && !segment.starts_with("storage=") && !segment.starts_with("turn=")
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ContextUsageView {
    used_tokens: u64,
    max_tokens: u64,
    percentage: u64,
}

fn context_usage_view(used_tokens: u64, max_tokens: Option<u64>) -> Option<ContextUsageView> {
    let max_tokens = max_tokens.filter(|max_tokens| *max_tokens > 0)?;
    Some(ContextUsageView {
        used_tokens,
        max_tokens,
        percentage: used_tokens.saturating_mul(100) / max_tokens,
    })
}

fn context_sidebar_lines(used_tokens: u64, max_tokens: Option<u64>) -> Vec<String> {
    let Some(context) = context_usage_view(used_tokens, max_tokens) else {
        return vec!["Context: unknown".into()];
    };
    let filled = ((context.used_tokens.saturating_mul(24)) / context.max_tokens).min(24) as usize;
    vec![
        format!(
            "{} / {} tokens",
            format_token_count(context.used_tokens),
            format_token_count(context.max_tokens)
        ),
        format!(
            "[{}{}] {}%",
            "#".repeat(filled),
            "-".repeat(24usize.saturating_sub(filled)),
            context.percentage.min(100)
        ),
    ]
}

fn context_suggestions(state: &AppState) -> Vec<ContextSuggestion> {
    let Some(context) =
        context_usage_view(state.costs.usage.total_tokens(), state.context_window_size)
    else {
        return Vec::new();
    };

    let percentage = context.percentage.min(100);
    if percentage > 70 {
        vec![ContextSuggestion {
            severity: SuggestionSeverity::Warning,
            title: "Context nearing limit".into(),
            detail: "Run /compact to reduce context usage".into(),
        }]
    } else if percentage > 50 {
        vec![ContextSuggestion {
            severity: SuggestionSeverity::Info,
            title: "Consider /compact to free up context".into(),
            detail: "Run /compact to reduce context usage".into(),
        }]
    } else {
        Vec::new()
    }
}

fn context_warning_banner(used_tokens: u64, max_tokens: Option<u64>) -> Option<PromptWarningView> {
    let context = context_usage_view(used_tokens, max_tokens)?;
    let percentage = context.percentage.min(100);
    if percentage >= 90 {
        Some(PromptWarningView {
            text: format!(
                "⚠ Context window nearly full ({percentage}%) — responses may be truncated"
            ),
            severity: PromptWarningSeverity::Critical,
        })
    } else if percentage >= 75 {
        Some(PromptWarningView {
            text: format!("⚠ Context window {percentage}% full — consider starting a new session"),
            severity: PromptWarningSeverity::Warning,
        })
    } else {
        None
    }
}

fn format_token_count(value: u64) -> String {
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

fn suggestion_icon(severity: SuggestionSeverity) -> &'static str {
    match severity {
        SuggestionSeverity::Warning => "⚠",
        SuggestionSeverity::Info => "→",
    }
}

fn suggestion_icon_style(severity: SuggestionSeverity, theme: &Theme) -> TextStyle {
    match severity {
        SuggestionSeverity::Warning => TextStyle::default().fg(Color::Yellow).bold(),
        SuggestionSeverity::Info => theme.prompt,
    }
}

fn suggestion_title_style(severity: SuggestionSeverity, theme: &Theme) -> TextStyle {
    match severity {
        SuggestionSeverity::Warning => TextStyle::default().fg(Color::Yellow).bold(),
        SuggestionSeverity::Info => theme.title,
    }
}

fn history_match_label(search: &HistorySearchView) -> String {
    if search.match_total == 0 {
        return "no matches".into();
    }

    format!(
        "match {}/{}",
        search.match_index.saturating_add(1),
        search.match_total
    )
}

fn format_global_search_result(result: &SearchMatch, width: u16) -> String {
    let location_width = usize::from((width / 3).max(16));
    let location = truncate_inline(
        &format!("{}:{}", result.file, result.line),
        location_width.min(usize::from(width)),
    );
    let preview_width = usize::from(width).saturating_sub(location.chars().count() + 2);
    let preview = truncate_inline(result.text.trim(), preview_width);
    if preview.is_empty() {
        location
    } else {
        format!("{location}  {preview}")
    }
}

fn truncate_inline(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width == 1 {
        return "…".into();
    }
    let mut truncated = text
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn plain_lines(lines: &[String], style: TextStyle) -> Vec<StyledLine> {
    lines
        .iter()
        .map(|line| StyledLine::plain(line.clone(), style))
        .collect()
}

fn style_for_message(theme: &Theme, role: MessageRole) -> TextStyle {
    match role {
        MessageRole::User => theme.prompt,
        MessageRole::Assistant => theme.messages,
        MessageRole::System => theme.footer,
        MessageRole::Tool => {
            let mut style = theme.status.fg(Color::Green);
            style.bold = true;
            style.dim = false;
            style
        }
        MessageRole::Progress => {
            let mut style = theme.status;
            style.dim = true;
            style
        }
        MessageRole::Error => TextStyle::default().fg(Color::Red).bold(),
    }
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    text.lines().map(ToString::to_string).collect()
}

fn text_line_count(text: &str) -> usize {
    text.lines().count().max(1)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use wonder_of_u_core::{
        AppState, MessageEnvelope, MessagePayload, TaskState, TaskStatus, TokenUsage,
    };

    use super::*;
    use crate::{dialog::DialogView, message::MessageLineView};

    /// Guard against accidentally lowering `MIN_SIDEBAR_WIDTH` back to 100.
    ///
    /// The threshold was raised to 120 so that common 100-column terminals keep
    /// the full main area.  A regression here would reintroduce the narrow-render
    /// issue reported in the visual parity audit.
    #[test]
    fn sidebar_threshold_is_120_columns() {
        assert_eq!(
            MIN_SIDEBAR_WIDTH, 120,
            "MIN_SIDEBAR_WIDTH must be 120; raising or lowering this changes sidebar activation"
        );
    }

    /// At exactly `MIN_SIDEBAR_WIDTH` the sidebar activates and the main area
    /// shrinks by `SIDEBAR_WIDTH + 1`.  One column below the threshold the
    /// sidebar must NOT activate even when `has_sidebar = true`.
    #[test]
    fn shell_main_area_width_activates_at_threshold_boundary() {
        // Exactly at threshold → sidebar deduction applies.
        assert_eq!(
            shell_main_area_width(MIN_SIDEBAR_WIDTH, true),
            MIN_SIDEBAR_WIDTH - SIDEBAR_WIDTH - 1,
            "sidebar must activate at exactly MIN_SIDEBAR_WIDTH={MIN_SIDEBAR_WIDTH}"
        );
        // One below threshold → full width even with sidebar flag set.
        assert_eq!(
            shell_main_area_width(MIN_SIDEBAR_WIDTH - 1, true),
            MIN_SIDEBAR_WIDTH - 1,
            "sidebar must NOT activate one column below the threshold"
        );
        // Threshold without sidebar flag → full width.
        assert_eq!(
            shell_main_area_width(MIN_SIDEBAR_WIDTH, false),
            MIN_SIDEBAR_WIDTH,
            "sidebar flag=false must never deduct columns"
        );
    }

    #[test]
    fn global_search_result_format_includes_location_and_truncates_preview() {
        let formatted = format_global_search_result(
            &SearchMatch {
                file: "src/main.rs".into(),
                line: 42,
                text: "let very_long_identifier_name = search_target();".into(),
            },
            24,
        );

        assert!(formatted.contains("src/main.rs:42"));
        assert!(formatted.ends_with('…'));
    }

    #[test]
    fn empty_shell_snapshot_renders_placeholder_message() {
        let view = ShellView {
            title: "Session: Empty".into(),
            messages: Vec::new(),
            prompt: String::new(),
            history_search: None,
            status: "prompt | 0 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(30, 9, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Empty",
                "",
                "   ██╗    ██╗  ██████╗  ██╗",
                "   ██║    ██║ ██╔═══██╗ ██║",
                "╭─ prompt ───────────────────╮",
                "│›                           │",
                "│prompt · 0 messages · Enter │",
                "╰────────────────────────────╯",
                "             ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn prompt_shell_snapshot_renders_messages_and_tasks() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![
                MessageLineView::new("system> ready", MessageRole::System),
                MessageLineView::new("hello", MessageRole::Assistant),
            ],
            prompt: "/status".into(),
            history_search: None,
            status: "prompt | 2 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: Some(TaskPanelView {
                title: "Tasks".into(),
                lines: vec![MessageLineView::new(
                    "[running] shell: index workspace",
                    MessageRole::Progress,
                )],
            }),
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(48, 10, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "system> ready",
                "hello",
                "",
                "Tasks",
                "[running] shell: index workspace",
                "╭─ prompt ─────────────────────────────────────╮",
                "│› /status                                     │",
                "│prompt · 2 messages · Enter send · Shift+Enter│",
                "╰──────────────────────────────────────────────╯",
                "              cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn shell_view_from_app_state_adapts_messages_and_chrome() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.session.title = "Demo".into();
        app.session.git_branch = None;
        app.provider = Some("copilot".into());
        app.model = Some("gpt-5".into());
        app.record_cost_usage(
            TokenUsage {
                input_tokens: 12,
                output_tokens: 3,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );
        app.push_message(MessageEnvelope::system(app.session.id, "ready"))
            .expect("push system");
        app.push_message(MessageEnvelope::new(
            app.session.id,
            MessagePayload::AssistantText {
                content: "working".into(),
            },
        ))
        .expect("push assistant");
        app.background_tasks.insert(
            Default::default(),
            TaskState {
                description: "index workspace".into(),
                status: TaskStatus::Running,
                ..TaskState::pending("index workspace")
            },
        );
        app.queue_command("/status", wonder_of_u_core::QueuePlacement::Later);

        let view = ShellView::from_app_state(&app, "/help", false);

        assert_eq!(view.title, "Session: Demo");
        assert_eq!(view.messages[0].text, "system> ready");
        assert!(view.status.contains("copilot:gpt-5"));
        assert!(view.footer.contains("cwd=/workspace"));
        assert!(view.queued_panel.is_some());
        assert!(view.task_panel.is_some());
    }

    #[test]
    fn shell_snapshot_renders_queued_commands_overlay() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: "/plan".into(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: Some(TaskPanelView {
                title: "Queued".into(),
                lines: vec![
                    MessageLineView::new("1. /status", MessageRole::Progress),
                    MessageLineView::new("2. draft migration plan", MessageRole::Progress),
                    MessageLineView::new("+2 more queued", MessageRole::System),
                ],
            }),
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(42, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "ready",
                "",
                "",
                "Queued",
                "1. /status",
                "2. draft migration plan",
                "+2 more queued",
                "╭─ prompt ───────────────────────────────╮",
                "│› /plan                                 │",
                "│prompt · 1 messages · Enter send · Shift│",
                "╰────────────────────────────────────────╯",
                "        cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn dialog_snapshot_renders_confirm_overlay() {
        let view = ShellView {
            title: "Session: Dialog".into(),
            messages: vec![MessageLineView::new("system> ready", MessageRole::System)],
            prompt: "continue?".into(),
            history_search: None,
            status: "permission | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: Some(DialogView::confirm(
                "Confirm action",
                ["Approve command execution", "This cannot be undone"],
            )),
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(42, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                " ╭─Confirm action───────────────────────╮",
                " │Approve command execution             │",
                " │This cannot be undone                 │",
                " │                                      │",
                " │[Confirm]  Cancel                     │",
                " ╰──────────────────────────────────────╯",
                "",
                "╭─ prompt ───────────────────────────────╮",
                "│› continue?                             │",
                "│permission · 1 messages · Enter send · S│",
                "╰────────────────────────────────────────╯",
                "        cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn history_search_snapshot_renders_overlay_and_preview() {
        let view = ShellView {
            title: "Session: Search".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: "draft".into(),
            history_search: Some(HistorySearchView {
                query: "pla".into(),
                match_text: Some("draft plan".into()),
                match_index: 1,
                match_total: 3,
            }),
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(42, 16, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "ready",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "╭─ prompt ───────────────────────────────╮",
                "│draft plan                              │",
                "│history: pla · match 2/3 · ↑↓/Ctrl+R cyc│",
                "╰────────────────────────────────────────╯",
                "        cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn slash_suggestions_snapshot_renders_above_prompt_footer() {
        let view = ShellView {
            title: "Session: Slash".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: "/".into(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: Some(SlashSuggestionsOverlay {
                entries: vec![
                    SlashSuggestionEntry {
                        display: "/status".into(),
                        description: "show session stats".into(),
                        selected: true,
                    },
                    SlashSuggestionEntry {
                        display: "/theme".into(),
                        description: "pick theme".into(),
                        selected: false,
                    },
                ],
            }),
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(48, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "ready",
                "",
                "",
                "╭─commands─────────────────────╮",
                "│/status ─ show session stats  │",
                "│/theme ─ pick theme           │",
                "╰──────────────────────────────╯",
                "╭─ prompt ─────────────────────────────────────╮",
                "│› /                                           │",
                "│slash commands · ↑↓ select · Tab/Enter apply ·│",
                "╰──────────────────────────────────────────────╯",
                "              cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn slash_suggestions_with_argument_hints_render_in_display_column() {
        // Entries whose `display` already contains a hint (e.g. `/fast [on|off]`)
        // must be rendered verbatim in the left column of the overlay.
        let view = ShellView {
            title: "Session: Hints".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: "/f".into(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: Some(SlashSuggestionsOverlay {
                entries: vec![
                    SlashSuggestionEntry {
                        display: "/fast [on|off]".into(),
                        description: "fast-mode model remapping".into(),
                        selected: true,
                    },
                    SlashSuggestionEntry {
                        display: "/effort [low|medium|high|max|auto]".into(),
                        description: "active effort level".into(),
                        selected: false,
                    },
                ],
            }),
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(64, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Argument hints must appear in the rendered overlay.
        assert!(
            text.contains("/fast [on|off]"),
            "hint for /fast not found in:\n{text}"
        );
        assert!(
            text.contains("/effort [low|medium|high|max|auto]"),
            "hint for /effort not found in:\n{text}"
        );
    }

    #[test]
    fn transcript_snapshot_keeps_latest_visible_lines() {
        // height=9, CHROME_HEIGHT=1 → available=8, prompt_height=3 (boxed),
        // messages_height=5.  With 8 messages only the tail 5 are visible, so
        // "line 1" is pushed off the top.
        let view = ShellView {
            title: "Session: Tail".into(),
            messages: (1..=8)
                .map(|index| MessageLineView::new(format!("line {index}"), MessageRole::Assistant))
                .collect(),
            prompt: "tail".into(),
            history_search: None,
            status: "prompt | 8 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(32, 9, &view, &Theme::default());

        assert!(!frame.to_plain_text().contains("line 1"));
        assert!(frame.to_plain_text().contains("line 6"));
    }

    #[test]
    fn shell_snapshot_renders_grouped_tool_summary_lines() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![
                MessageLineView::new("● Run(Tests)", MessageRole::Tool),
                MessageLineView::new("  └ cargo test -p wonder-of-u-tui", MessageRole::System),
                MessageLineView::new("  └ tests passed", MessageRole::System),
            ],
            prompt: String::new(),
            history_search: None,
            status: "prompt | 3 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(48, 10, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "● Run(Tests)",
                "  └ cargo test -p wonder-of-u-tui",
                "  └ tests passed",
                "",
                "",
                "╭─ prompt ─────────────────────────────────────╮",
                "│›                                             │",
                "│prompt · 3 messages · Enter send · Shift+Enter│",
                "╰──────────────────────────────────────────────╯",
                "              cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn shell_snapshot_renders_notification_stack_overlay() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: String::new(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: vec![
                NotificationView {
                    key: "source-status".into(),
                    title: "Source status".into(),
                    lines: vec!["workspace index refreshed".into()],
                    severity: NotificationSeverity::Info,
                    focused: false,
                },
                NotificationView {
                    key: "task:1".into(),
                    title: "Task update".into(),
                    lines: vec!["tests passed".into()],
                    severity: NotificationSeverity::Success,
                    focused: true,
                },
            ],
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(48, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "ready",
                "                   ╭─info Source status────────╮",
                "                   │workspace index refreshed  │",
                "                   ╰───────────────────────────╯",
                "                      ╭─ok Task update • focus─╮",
                "                      │tests passed            │",
                "                      ╰────────────────────────╯",
                "╭─ prompt ─────────────────────────────────────╮",
                "│›                                             │",
                "│prompt · 1 messages · Enter send · Shift+Enter│",
                "╰──────────────────────────────────────────────╯",
                "              cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn shell_snapshot_renders_picker_list_overlay() {
        let view = ShellView {
            title: "Session: Picker".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::Assistant)],
            prompt: String::new(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: Some(PickerListView {
                title: "Select Theme".into(),
                query: "mid".into(),
                entries: vec![
                    crate::message::PickerListEntry {
                        label: "Midnight".into(),
                        description: "Dark theme".into(),
                        tag: Some("current".into()),
                        selected: true,
                    },
                    crate::message::PickerListEntry {
                        label: "Light".into(),
                        description: "Bright theme".into(),
                        tag: None,
                        selected: false,
                    },
                ],
                hint: "↑↓ navigate  Tab/Enter select  Esc cancel".into(),
            }),
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(60, 16, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(text.contains("Select Theme"));
        assert!(text.contains("Search: mid"));
        // Tag is now right-aligned at the row's far-right edge; label and tag
        // appear on the same row but are no longer adjacent — verify each part
        // is present in the rendered frame.
        assert!(
            text.contains("▸ Midnight"),
            "selected cursor + label must be present"
        );
        assert!(text.contains("Dark theme"), "description must be visible");
        assert!(
            text.contains("[current]"),
            "right-aligned tag badge must be present"
        );
        // The tag must not appear immediately after the label (it was moved to
        // the right edge, so there are padding spaces between them).
        assert!(
            !text.contains("Midnight [current]"),
            "tag must not be inline immediately after label"
        );
        assert!(text.contains("↑↓ navigate  Tab/Enter select  Esc cancel"));
    }

    /// Verifies that the picker tag badge is right-aligned at the row's far-right
    /// edge and that the description is rendered with a visual gap before the tag.
    ///
    /// Layout (inner width = 46, tag "[current]" = 9 cols):
    /// ```text
    /// ▸ Midnight — Dark theme              [current]
    ///   Light — Bright theme
    /// ```
    /// The tag must be separated from the description by at least one space.
    #[test]
    fn picker_list_tag_is_right_aligned() {
        let view = ShellView {
            title: "Test".into(),
            messages: Vec::new(),
            prompt: String::new(),
            history_search: None,
            status: String::new(),
            loading: false,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: String::new(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: Some(PickerListView {
                title: "Pick".into(),
                query: String::new(),
                entries: vec![
                    crate::message::PickerListEntry {
                        label: "Alpha".into(),
                        description: "first option".into(),
                        tag: Some("active".into()),
                        selected: true,
                    },
                    crate::message::PickerListEntry {
                        label: "Beta".into(),
                        description: "second option".into(),
                        tag: None,
                        selected: false,
                    },
                ],
                hint: "Esc cancel".into(),
            }),
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(60, 16, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Label + description appear normally.
        assert!(text.contains("▸ Alpha"), "cursor + label must render");
        assert!(text.contains("first option"), "description must render");
        // Tag is present but not adjacent to the label.
        assert!(text.contains("[active]"), "tag badge must render");
        assert!(
            !text.contains("Alpha [active]"),
            "tag must not be immediately after label"
        );
        // Find the line that contains the tag and verify the label appears to
        // its left (tag is right-aligned, so it comes after the label in the line).
        let tag_line = text
            .lines()
            .find(|l| l.contains("[active]"))
            .expect("a line containing the tag must exist");
        let label_pos = tag_line
            .find("Alpha")
            .expect("label must be on the same line");
        let tag_pos = tag_line
            .find("[active]")
            .expect("tag must be on the same line");
        assert!(
            label_pos < tag_pos,
            "label must appear before right-aligned tag (label@{label_pos} tag@{tag_pos})"
        );
        // There must be at least one space between the description and the tag.
        assert!(
            tag_pos > label_pos + "Alpha".len() + 2,
            "tag must be separated from label by at least 2 positions"
        );
    }

    #[test]
    fn shell_snapshot_prefixes_loading_status() {
        let view = ShellView {
            title: "Session: Loading".into(),
            messages: Vec::new(),
            prompt: String::new(),
            history_search: None,
            status: "turn=active".into(),
            loading: true,
            loading_verb: Some("thinking".into()),
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            footer: String::new(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(32, 8, &view, &Theme::default());

        let text = frame.to_plain_text();
        // The spinner prefix still appears in the transcript (status row removed with CHROME_HEIGHT=1).
        assert!(text.contains("· thinking…"));
    }

    #[test]
    fn loading_progress_row_appears_above_prompt_when_loading() {
        // The loading row should be rendered in the message area, directly above
        // the prompt box — separate from the transcript content.
        let view = ShellView {
            title: "Session: Loading".into(),
            messages: vec![MessageLineView::new("hello", MessageRole::Assistant)],
            prompt: String::new(),
            history_search: None,
            status: "turn=active".into(),
            loading: true,
            loading_verb: Some("thinking".into()),
            spinner_frame: 4,
            loading_elapsed_secs: 3,
            loading_total_tokens: 150,
            footer: "▸▸ default (shift+tab to cycle) · ⌃C exit".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            global_search: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
            prompt_warning: None,
        };

        let frame = render_snapshot(60, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Loading progress row must contain the spinner verb and "esc to interrupt".
        assert!(
            text.contains("thinking"),
            "loading row must contain verb: {text}"
        );
        assert!(
            text.contains("esc to interrupt"),
            "loading row must contain suffix: {text}"
        );
        // Loading row must NOT appear inside the transcript (spinner moved out of messages).
        // The transcript message "hello" must still appear.
        assert!(
            text.contains("hello"),
            "transcript message must still appear"
        );
        // Compact footer hint must appear.
        assert!(
            text.contains("shift+tab to cycle"),
            "compact footer hint must appear: {text}"
        );
        assert!(
            !text.contains("storage="),
            "verbose footer fields must not appear: {text}"
        );
    }

    #[test]
    fn loading_progress_row_includes_elapsed_and_tokens_when_nonzero() {
        let view = ShellView {
            loading: true,
            loading_verb: Some("thinking".into()),
            spinner_frame: 2,
            loading_elapsed_secs: 7,
            loading_total_tokens: 512,
            ..ShellView::default()
        };

        let text = build_loading_progress_text(&view);
        // Elapsed must appear (7 seconds → "00:07" mm:ss format).
        assert!(
            text.contains("00:07"),
            "elapsed seconds must appear in loading row text: {text}"
        );
        // Token count must appear.
        assert!(
            text.contains("512"),
            "token count must appear in loading row text: {text}"
        );
    }

    #[test]
    fn loading_progress_row_omits_elapsed_and_tokens_when_zero() {
        let view = ShellView {
            loading: true,
            loading_verb: None,
            spinner_frame: 0,
            loading_elapsed_secs: 0,
            loading_total_tokens: 0,
            ..ShellView::default()
        };

        let text = build_loading_progress_text(&view);
        // Neither elapsed nor token data should appear.
        assert!(
            !text.contains("tok"),
            "token count must not appear when zero: {text}"
        );
        // The "esc to interrupt" suffix must always appear.
        assert!(
            text.contains("esc to interrupt"),
            "suffix must always appear: {text}"
        );
    }

    // ── scroll rendering ─────────────────────────────────────────────────────

    fn make_long_transcript(count: usize) -> Vec<MessageLineView> {
        (1..=count)
            .map(|i| MessageLineView::new(format!("line {i:02}"), MessageRole::Assistant))
            .collect()
    }

    #[test]
    fn transcript_tail_mode_shows_newest_lines() {
        // 10 lines of content → tail mode (offset 0) must show the newest line.
        let view = ShellView {
            title: "Session: Tail".into(),
            messages: make_long_transcript(10),
            prompt: String::new(),
            status: "tail".into(),
            scroll: TranscriptScrollView::default(), // offset_from_bottom = 0
            ..ShellView::default()
        };

        let frame = render_snapshot(20, 7, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(!text.contains("line 01"), "oldest line must not appear");
        assert!(text.contains("line 10"), "newest line must appear");
        // Status line must NOT show the scroll indicator in tail mode.
        assert!(
            !text.contains("Ctrl+End"),
            "scroll indicator must be absent in tail mode"
        );
    }

    #[test]
    fn transcript_scrolled_up_shows_earlier_window() {
        // height=10, CHROME_HEIGHT=1 → available=9, prompt_height=4 (boxed with
        // integrated footer), messages_height=5. offset_from_bottom=1:
        //   start = (10-5).saturating_sub(1) = 4 → shows lines 05–09.
        let view = ShellView {
            title: "Session: Scrolled".into(),
            messages: make_long_transcript(10),
            prompt: String::new(),
            status: "scrolled".into(),
            scroll: TranscriptScrollView {
                offset_from_bottom: 1,
                total_lines: 10,
                visible_lines: 8,
            },
            ..ShellView::default()
        };

        let frame = render_snapshot(20, 10, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(text.contains("line 05"), "window start must be visible");
        assert!(text.contains("line 09"), "window end must be visible");
        assert!(
            !text.contains("line 04"),
            "line before window must be hidden"
        );
        assert!(!text.contains("line 10"), "newest line must not appear");
    }

    #[test]
    fn scroll_indicator_appears_in_status_when_scrolled_up() {
        // Scrolling up by 5 lines must add "↑ 5 lines · Ctrl+End bottom" to the
        // status row.
        let view = ShellView {
            title: "Session: Ind".into(),
            messages: make_long_transcript(10),
            prompt: String::new(),
            status: "scrolled".into(),
            scroll: TranscriptScrollView {
                offset_from_bottom: 5,
                total_lines: 10,
                visible_lines: 4,
            },
            ..ShellView::default()
        };

        let frame = render_snapshot(60, 8, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(
            text.contains("5 lines"),
            "line count must appear in indicator; got: {text:?}"
        );
        assert!(
            text.contains("Ctrl+End bottom"),
            "hint text must appear in indicator; got: {text:?}"
        );
    }

    #[test]
    fn scroll_indicator_absent_when_at_tail() {
        let view = ShellView {
            title: "Session: Tail".into(),
            messages: make_long_transcript(5),
            prompt: String::new(),
            status: "tail".into(),
            scroll: TranscriptScrollView::default(),
            ..ShellView::default()
        };

        let frame = render_snapshot(60, 8, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(
            !text.contains("Ctrl+End"),
            "scroll indicator must not appear in tail mode"
        );
    }

    #[test]
    fn welcome_screen_unchanged_when_messages_empty() {
        // The welcome screen is always top-anchored regardless of any scroll offset,
        // because `draw_lines` (not the windowed path) is used for empty messages.
        let view = ShellView {
            title: "Session: Empty".into(),
            messages: Vec::new(),
            prompt: String::new(),
            status: "welcome".into(),
            // Even if a non-zero offset somehow leaks in, the welcome path is unchanged.
            scroll: TranscriptScrollView {
                offset_from_bottom: 99,
                total_lines: 0,
                visible_lines: 10,
            },
            ..ShellView::default()
        };

        // Use a tall frame so the welcome tagline (line 8 of the welcome screen) is
        // actually visible in the transcript area.
        let frame = render_snapshot(42, 20, &view, &Theme::default());
        let text = frame.to_plain_text();
        // The ASCII-art banner should appear at the top.
        assert!(text.contains("██╗"), "welcome banner must be present");
        assert!(
            text.contains("wonder-of-u  ·  AI coding assistant"),
            "welcome tagline must be present"
        );
    }

    #[test]
    fn scrolled_up_past_top_is_clamped_to_first_line() {
        // 3 lines of content, offset = 999 (far beyond max).
        // render_snapshot(20, 12) → 6 visible transcript rows.
        // start = (3 - 6).saturating_sub(999) = 0 → all 3 lines are shown.
        let view = ShellView {
            title: "Session: Clamp".into(),
            messages: make_long_transcript(3),
            prompt: String::new(),
            status: "clamped".into(),
            scroll: TranscriptScrollView {
                offset_from_bottom: 999,
                total_lines: 3,
                visible_lines: 6,
            },
            ..ShellView::default()
        };

        let frame = render_snapshot(20, 12, &view, &Theme::default());
        let text = frame.to_plain_text();
        // All three lines must appear since start is clamped to 0.
        assert!(
            text.contains("line 01"),
            "first line must be visible when clamped"
        );
        assert!(
            text.contains("line 03"),
            "last line must be visible when clamped"
        );
    }

    // ── prompt height and cap ────────────────────────────────────────────────

    #[test]
    fn prompt_height_single_line_is_line_count() {
        // Rounded prompt: a single-line prompt occupies one content row, an
        // integrated footer row, and the borders.
        let view = ShellView {
            prompt: "hello".into(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 4);
    }

    #[test]
    fn prompt_height_multiline_counts_all_lines() {
        // 3-line prompt → 3 content rows + footer row + 2 border rows.
        let view = ShellView {
            prompt: "line one\nline two\nline three".into(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 6);
    }

    #[test]
    fn prompt_height_empty_prompt_returns_one() {
        // An empty prompt still needs one input row, the integrated footer row,
        // and the rounded borders.
        let view = ShellView {
            prompt: String::new(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 4);
    }

    #[test]
    fn render_shell_caps_prompt_to_one_third_of_terminal_height() {
        // Terminal height = 9 rows; one-third cap = max(9/3, 3) = 3 prompt rows.
        // A 10-line prompt would request prompt_height() = 12, but the renderer
        // must cap it at 3 so the history/transcript area always gets space.
        let ten_line_prompt = (0..10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let view = ShellView {
            title: "Session: Cap".into(),
            messages: vec![MessageLineView::new("hello", MessageRole::Assistant)],
            prompt: ten_line_prompt,
            status: "prompt".into(),
            ..ShellView::default()
        };

        // The uncapped prompt would be 11 rows tall; with a 9-row terminal the
        // render should not panic and the transcript line must still be visible.
        let frame = render_snapshot(40, 9, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(
            text.contains("hello"),
            "transcript must survive tall prompt; rendered:\n{text}"
        );
    }

    // ── multiline prompt marker and cursor offset ────────────────────────────

    #[test]
    fn multiline_prompt_first_line_has_marker_continuation_lines_do_not() {
        // Rounded prompt: prompt_height = 6 (3 content lines + prompt footer +
        // borders). A slightly taller frame keeps all three prompt lines visible.
        let view = ShellView {
            title: "Session: ML".into(),
            messages: Vec::new(),
            prompt: "first line\nsecond line\nthird line".into(),
            status: "prompt".into(),
            ..ShellView::default()
        };

        let frame = render_snapshot(30, 18, &view, &Theme::default());
        let text = frame.to_plain_text();

        // First line must carry the prompt marker.
        assert!(
            text.contains("› first line"),
            "first prompt line must have the '› ' marker; rendered:\n{text}"
        );
        // Continuation lines must not carry the marker — they should start
        // flush with the content column without '› '.
        assert!(
            text.contains("second line") && !text.contains("› second line"),
            "second prompt line must NOT have the '› ' marker; rendered:\n{text}"
        );
        assert!(
            text.contains("third line") && !text.contains("› third line"),
            "third prompt line must NOT have the '› ' marker; rendered:\n{text}"
        );
    }

    // ── provider error rendering via ShellView::from_app_state ──────────────

    #[test]
    fn provider_error_payload_appears_as_error_role_in_shell_view() {
        // Verify that from_app_state maps a ProviderError payload through
        // message_lines() into a MessageLineView with MessageRole::Error.
        // This complements the lower-level render_message test by confirming
        // the full pipeline from AppState → ShellView → transcript line.
        let mut app = AppState::new(PathBuf::from("/workspace"));
        let envelope = MessageEnvelope::new(
            app.session.id,
            MessagePayload::ProviderError {
                kind: "provider".into(),
                message: "connection refused".into(),
            },
        );
        app.push_message(envelope).expect("push provider error");

        let view = ShellView::from_app_state(&app, "", false);
        assert_eq!(view.messages.len(), 1, "one transcript line expected");
        assert_eq!(
            view.messages[0].role,
            MessageRole::Error,
            "ProviderError must map to MessageRole::Error"
        );
        assert!(
            view.messages[0].text.contains("connection refused"),
            "message body must appear in the transcript line; got: {:?}",
            view.messages[0].text
        );
    }

    // ── sidebar panel ─────────────────────────────────────────────────────────

    #[test]
    fn sidebar_absent_on_narrow_terminal_below_min_width() {
        // Width 119 is one below the MIN_SIDEBAR_WIDTH threshold — the sidebar
        // must not appear even when `view.sidebar` is Some.
        let view = ShellView {
            prompt: "hello".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["openai · gpt-4o".into()],
                status_lines: vec!["✓ ready".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(119, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Sidebar section headers must not appear on a narrow terminal.
        assert!(
            !text.contains("─ Providers ─"),
            "sidebar hints must be absent on narrow terminal; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_renders_on_wide_terminal_at_min_width() {
        // Width == MIN_SIDEBAR_WIDTH (120) must activate the sidebar.
        // The sidebar spans the full terminal height so a single-line prompt is
        // sufficient — no need for a tall prompt to expose sidebar rows.
        let view = ShellView {
            prompt: "say hello".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ copilot · gpt-4".into()],
                control_lines: vec!["↵ send  ⇧↵ newline".into(), "⎋ cancel  ? help".into()],
                status_lines: vec!["✓ ready".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        // Use a taller terminal so the rounded-border box has enough inner rows
        // to show all three sidebar sections (Providers, Status, Controls).
        // The rounded border takes 2 rows (top + bottom), leaving 12 inner rows.
        let frame = render_snapshot(120, 14, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Controls ─"),
            "Controls header must appear in sidebar; rendered:\n{text}"
        );
        assert!(
            text.contains("─ Providers ─"),
            "Providers header must appear in sidebar; rendered:\n{text}"
        );
        assert!(
            text.contains("copilot · gpt-4"),
            "model hint must appear in sidebar; rendered:\n{text}"
        );
        assert!(
            text.contains("ready"),
            "auth-ok label must appear; rendered:\n{text}"
        );
        // Sidebar rounded border corners must be visible.
        assert!(
            text.contains('╭') && text.contains('╰'),
            "rounded border corners must appear; rendered:\n{text}"
        );
        // Prompt content must still be visible in the main (left) column.
        assert!(
            text.contains("say hello"),
            "prompt text must survive sidebar split; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_absent_when_field_is_none() {
        // sidebar = None must suppress the panel even on a wide terminal.
        let view = ShellView {
            prompt: "test".into(),
            sidebar: None,
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 6, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ Providers ─") && !text.contains("─ Controls ─"),
            "sidebar must not render when field is None; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_shows_auth_missing_warning_when_not_authenticated() {
        let view = ShellView {
            prompt: "help".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["anthropic · claude-3".into()],
                status_lines: vec!["⚠ auth missing".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("auth missing"),
            "auth-missing warning must appear; rendered:\n{text}"
        );
        assert!(
            !text.contains("✓"),
            "ready checkmark must not appear when not authenticated; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_shows_mode_hint_for_bash_mode() {
        let view = ShellView {
            prompt: "!ls".into(),
            sidebar: Some(SidebarView {
                status_lines: vec!["● bash mode".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("bash"),
            "bash mode hint must appear in sidebar; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_shows_git_branch_when_configured() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                workspace_lines: vec!["⎇  feat/my-feature".into(), "✓ ready".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("feat/my-feature"),
            "git branch must appear in sidebar; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_shows_cwd_hint_when_configured() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                workspace_lines: vec!["  myproject".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("myproject"),
            "cwd hint must appear in sidebar; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_shows_task_count_when_nonzero() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                task_lines: vec!["⚙  3 tasks".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("3 tasks"),
            "task count must appear in sidebar; rendered:\n{text}"
        );
    }

    #[test]
    fn from_app_state_populates_sidebar_with_provider_and_auth() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.provider = Some("openai".into());
        app.model = Some("gpt-4o".into());
        app.context_window_size = Some(128_000);
        app.record_cost_usage(
            TokenUsage {
                input_tokens: 32_000,
                output_tokens: 8_000,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );

        let view = ShellView::from_app_state(&app, "", false);

        let sidebar = view.sidebar.expect("sidebar must be Some from_app_state");
        assert!(
            sidebar
                .provider_lines
                .iter()
                .any(|l| l.contains("openai · gpt-4o")),
            "provider_lines must combine provider and model; got: {:?}",
            sidebar.provider_lines
        );
        assert!(
            !sidebar.control_lines.is_empty(),
            "control_lines must be populated"
        );
        // Default status is the idle stub.
        assert!(
            sidebar.status_lines.iter().any(|l| l.contains("idle")),
            "default status must be idle stub; got: {:?}",
            sidebar.status_lines
        );
        assert!(
            sidebar
                .context_lines
                .iter()
                .any(|line| line.contains("40,000 / 128,000 tokens")),
            "context_lines must show token totals; got: {:?}",
            sidebar.context_lines
        );
        assert!(
            sidebar.suggestions.is_empty(),
            "suggestions must stay hidden below threshold; got: {:?}",
            sidebar.suggestions
        );
    }

    #[test]
    fn from_app_state_populates_sidebar_git_branch_and_cwd() {
        let mut app = AppState::new(PathBuf::from("/workspace/myproject"));
        app.session.git_branch = Some("feat/my-feature".into());

        let view = ShellView::from_app_state(&app, "", false);

        let sidebar = view.sidebar.expect("sidebar must be Some from_app_state");
        assert!(
            sidebar
                .workspace_lines
                .iter()
                .any(|l| l.contains("feat/my-feature")),
            "workspace_lines must contain git branch; got: {:?}",
            sidebar.workspace_lines
        );
        assert!(
            sidebar
                .workspace_lines
                .iter()
                .any(|l| l.contains("myproject")),
            "workspace_lines must contain the last cwd component; got: {:?}",
            sidebar.workspace_lines
        );
    }

    #[test]
    fn context_suggestions_shown_above_threshold() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.set_context_window_size(Some(100_000));
        state.record_cost_usage(
            TokenUsage {
                input_tokens: 60_000,
                output_tokens: 15_000,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );

        let suggestions = context_suggestions(&state);

        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.detail.contains("/compact")),
            "expected /compact guidance, got: {suggestions:?}"
        );
    }

    #[test]
    fn context_suggestions_hidden_when_context_window_unknown() {
        let mut state = AppState::new(PathBuf::from("/workspace"));
        state.record_cost_usage(
            TokenUsage {
                input_tokens: 60_000,
                output_tokens: 15_000,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            None,
        );

        assert!(
            context_suggestions(&state).is_empty(),
            "suggestions must be hidden when the context window is unknown"
        );
    }

    #[test]
    fn sidebar_multiline_prompt_uses_full_main_column_width() {
        // The sidebar lives at shell level; the prompt takes the entire
        // main-column width — no internal split should compress the typing area.
        let view = ShellView {
            prompt: "first line\nsecond line\nthird line".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ copilot · gpt-4".into()],
                control_lines: vec!["↵ send  ⇧↵ newline".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        // width=120 → main_w=83, sidebar=36, sep=1.
        // height=16 keeps the multiline prompt plus the integrated footer visible.
        let frame = render_snapshot(120, 16, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("› first line"),
            "first prompt line must carry the marker; rendered:\n{text}"
        );
        assert!(
            text.contains("second line"),
            "second prompt line must be visible; rendered:\n{text}"
        );
        assert!(
            text.contains("─ Controls ─"),
            "sidebar Controls header must be present at shell level; rendered:\n{text}"
        );
    }

    #[test]
    fn prompt_spans_full_terminal_width_on_narrow_terminal() {
        // Width 119 is one below MIN_SIDEBAR_WIDTH=120.  Even when `sidebar` is Some,
        // no column is carved out — the prompt must use the full 119 columns.
        let view = ShellView {
            prompt: "hello".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ copilot · gpt-4".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(119, 6, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Prompt marker must appear inside the rounded box with no sidebar offset.
        let prompt_line = text
            .lines()
            .find(|l| l.contains("› hello"))
            .expect("prompt marker must be present");

        // Sidebar must be suppressed on narrow terminal.
        assert!(
            !text.contains("─ Providers ─"),
            "sidebar must be suppressed below MIN_SIDEBAR_WIDTH; rendered:\n{text}"
        );
        // The prompt box must be at the start of the line (no sidebar indentation).
        assert!(
            prompt_line.starts_with("│›"),
            "prompt content must start in the full-width prompt box on narrow terminal; got: {prompt_line:?}"
        );
    }

    #[test]
    fn prompt_view_does_not_panic_on_tiny_terminal() {
        // Tiny prompt areas fall back to direct content rendering when there is no
        // room for rounded chrome. Verify no panic and that the '›' marker appears.
        let view = ShellView {
            prompt: "hi".into(),
            ..ShellView::default()
        };

        // height=3 → chrome=1, available=2, prompt is reduced to keep one message row.
        // The prompt area is height=1, which draws directly without any box chrome.
        let frame = render_snapshot(10, 3, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Must not panic; the prompt marker '›' must still appear.
        assert!(
            text.contains('›'),
            "prompt marker must appear on tiny terminal; rendered:\n{text}"
        );
        // No rounded-box corners should appear in the tiny fallback path.
        assert!(
            !text.contains('╭'),
            "box corners must not appear in tiny fallback prompt; rendered:\n{text}"
        );
    }

    // ── new sectioned-sidebar tests ──────────────────────────────────────────

    #[test]
    fn sidebar_section_header_providers_appears_on_wide_terminal() {
        // A non-empty `provider_lines` section must produce a "─ Providers ─"
        // header row visible in the rendered output on a wide terminal.
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ anthropic · claude-3-5-sonnet".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Providers ─"),
            "Providers section header must appear on wide terminal; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_provider_line_with_diamond_prefix_rendered() {
        // A `provider_lines` entry with the `◈` prefix must appear in the output.
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ openai · gpt-4o".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("◈"),
            "◈ prefix must appear in sidebar provider line; rendered:\n{text}"
        );
        assert!(
            text.contains("openai · gpt-4o"),
            "provider body text must appear; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_absent_on_narrow_terminal_below_120_cols() {
        // Narrow terminal (width < MIN_SIDEBAR_WIDTH = 120): sidebar must be
        // completely suppressed even when `sidebar` field is `Some`.
        let view = ShellView {
            prompt: "narrow".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ copilot · gpt-4".into()],
                control_lines: vec!["↵ send  ⇧↵ newline".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(119, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ Providers ─"),
            "Providers header must NOT appear on narrow (119-col) terminal; rendered:\n{text}"
        );
        assert!(
            !text.contains("─ Controls ─"),
            "Controls header must NOT appear on narrow terminal; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_empty_sections_are_omitted() {
        // Sections with empty Vec must not produce stray headers or blank rows.
        let view = ShellView {
            prompt: "check".into(),
            sidebar: Some(SidebarView {
                // Only status_lines is populated; everything else is empty.
                status_lines: vec!["● idle".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Status ─"),
            "Status header must appear; rendered:\n{text}"
        );
        // All empty sections — including the four new panel sections — must
        // produce no headers.
        for absent in &[
            "─ Session ─",
            "─ Context ─",
            "─ Tools ─",
            "─ MCP ─",
            "─ LSP ─",
            "─ Todo ─",
            "─ Providers ─",
            "─ Controls ─",
            "─ Workspace ─",
            "─ Tasks ─",
        ] {
            assert!(
                !text.contains(absent),
                "{absent} header must not appear when section is empty; rendered:\n{text}"
            );
        }
    }

    #[test]
    fn shell_snapshot_renders_context_visualization_in_sidebar() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ anthropic · claude-3-7-sonnet-latest".into()],
                context_lines: vec![
                    "45,000 / 200,000 tokens".into(),
                    "[#####-------------------] 22%".into(),
                ],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Context ─"),
            "missing context header:\n{text}"
        );
        assert!(
            text.contains("45,000 / 200,000 tokens"),
            "missing token totals:\n{text}"
        );
        assert!(
            text.contains("[#####-------------------] 22%"),
            "missing progress bar:\n{text}"
        );
    }

    #[test]
    fn shell_snapshot_renders_context_suggestions_in_sidebar() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                context_lines: vec![
                    "160,000 / 200,000 tokens".into(),
                    "[###################-----] 80%".into(),
                ],
                suggestions: vec![ContextSuggestion {
                    severity: SuggestionSeverity::Warning,
                    title: "Context nearing limit".into(),
                    detail: "Run /compact to reduce context usage".into(),
                }],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Suggestions ─"),
            "missing suggestions header:\n{text}"
        );
        assert!(
            text.contains("Context nearing limit"),
            "missing suggestion title:\n{text}"
        );
        assert!(
            text.contains("/compact"),
            "missing suggestion detail:\n{text}"
        );
    }

    #[test]
    fn shell_snapshot_renders_context_warning_above_prompt() {
        let view = ShellView {
            prompt: "continue".into(),
            prompt_warning: Some(PromptWarningView {
                text: "⚠ Context window 80% full — consider starting a new session".into(),
                severity: PromptWarningSeverity::Warning,
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(80, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("⚠ Context window 80% full"),
            "missing warning banner:\n{text}"
        );
        let warning_row = text
            .lines()
            .position(|line| line.contains("⚠ Context window 80% full"))
            .expect("warning row");
        let prompt_border_row = text
            .lines()
            .position(|line| line.starts_with('╭'))
            .expect("prompt border row");
        assert_eq!(warning_row.saturating_add(1), prompt_border_row);
    }

    // ── Claude visual parity: no persistent header in normal chat mode ───────

    #[test]
    fn session_title_header_absent_when_messages_present() {
        // Once the conversation has messages the "▸ wonder-of-u …" header must
        // NOT appear — Claude Code fullscreen layout starts the transcript at row 0.
        let view = ShellView {
            title: "Session: MyProject".into(),
            messages: vec![MessageLineView::new(
                "● hello world",
                MessageRole::Assistant,
            )],
            prompt: "ask me".into(),
            footer: "▸▸ default (shift+tab to cycle)".into(),
            ..ShellView::default()
        };

        let frame = render_snapshot(60, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("▸ wonder-of-u"),
            "persistent header must not appear when messages are present; rendered:\n{text}"
        );
        // The transcript content should start at the very top row.
        let first_line = text.lines().next().unwrap_or("");
        assert!(
            first_line.contains("hello world"),
            "transcript must begin at row 0 when messages exist; rendered:\n{text}"
        );
    }

    #[test]
    fn session_title_header_present_on_welcome_screen() {
        // On the empty/welcome state the "▸ wonder-of-u …" header must appear.
        let view = ShellView {
            title: "Session: MyProject".into(),
            messages: Vec::new(),
            prompt: String::new(),
            footer: String::new(),
            ..ShellView::default()
        };

        let frame = render_snapshot(40, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("▸ wonder-of-u"),
            "welcome header must appear when there are no messages; rendered:\n{text}"
        );
    }

    // ── Claude visual parity: compact footer (▸▸ … shift+tab …) ────────────

    #[test]
    fn compact_footer_uses_hint_text_not_verbose_metadata() {
        // The footer row must surface the compact hint supplied by ShellView::footer,
        // not verbose fields like `storage=` or `turn=` from the old status bar.
        let view = ShellView {
            messages: vec![MessageLineView::new("● hi", MessageRole::User)],
            prompt: String::new(),
            footer: "▸▸ default (shift+tab to cycle) · ⌃C exit".into(),
            status: "turn=idle storage=/home/user/.wonder cwd=/workspace".into(),
            ..ShellView::default()
        };

        let frame = render_snapshot(60, 6, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("shift+tab to cycle"),
            "compact footer hint must appear; rendered:\n{text}"
        );
        // Verbose session metadata fields must stay out of the footer row.
        assert!(
            !text.contains("storage="),
            "verbose storage field must not appear in footer; rendered:\n{text}"
        );
        assert!(
            !text.contains("turn="),
            "verbose turn field must not appear in footer; rendered:\n{text}"
        );
    }

    #[test]
    fn compact_footer_scroll_badge_prepended_when_scrolled_up() {
        // When the transcript is scrolled away from the tail, the footer row
        // should include the scroll badge ("↑ N lines · Ctrl+End bottom").
        let view = ShellView {
            messages: (0..20)
                .map(|i| MessageLineView::new(format!("line {i}"), MessageRole::Assistant))
                .collect(),
            prompt: String::new(),
            footer: "▸▸ default".into(),
            scroll: TranscriptScrollView {
                offset_from_bottom: 5,
                total_lines: 20,
                visible_lines: 10,
            },
            ..ShellView::default()
        };

        let frame = render_snapshot(80, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("↑ 5 lines"),
            "scroll badge must appear when scrolled up; rendered:\n{text}"
        );
        assert!(
            text.contains("Ctrl+End bottom"),
            "scroll badge must contain Ctrl+End hint; rendered:\n{text}"
        );
    }

    // ── Claude visual parity: ● bullet rows for user/assistant ──────────────

    #[test]
    fn user_message_rendered_with_bullet_prefix() {
        // User messages must open with "● " so they match the Claude Code style.
        // This tests the render path that uses MessageRole::User colouring.
        let view = ShellView {
            messages: vec![MessageLineView::new("● fix the tests", MessageRole::User)],
            prompt: String::new(),
            ..ShellView::default()
        };

        let frame = render_snapshot(40, 7, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("● fix the tests"),
            "user message bullet must appear in transcript; rendered:\n{text}"
        );
    }

    #[test]
    fn assistant_message_rendered_with_bullet_prefix() {
        // Assistant messages must also open with "● " (Claude Code style).
        let view = ShellView {
            messages: vec![MessageLineView::new(
                "● I've updated the file",
                MessageRole::Assistant,
            )],
            prompt: String::new(),
            ..ShellView::default()
        };

        let frame = render_snapshot(40, 7, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("● I've updated the file"),
            "assistant message bullet must appear in transcript; rendered:\n{text}"
        );
    }

    // ── Claude visual parity: tool rows ─────────────────────────────────────

    #[test]
    fn tool_row_headline_and_detail_rendered() {
        // Tool calls must produce a "● Tool(args)" headline row followed by an
        // indented "  └ detail" row — matching the Claude Code tool activity style.
        let view = ShellView {
            messages: vec![
                MessageLineView::new("● Bash(ls -la)", MessageRole::Tool),
                MessageLineView::new("  └ success", MessageRole::Tool),
            ],
            prompt: String::new(),
            ..ShellView::default()
        };

        let frame = render_snapshot(40, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("● Bash(ls -la)"),
            "tool headline must appear; rendered:\n{text}"
        );
        assert!(
            text.contains("└ success"),
            "tool detail must appear indented; rendered:\n{text}"
        );
    }

    // ── Claude visual parity: loading row position ───────────────────────────

    #[test]
    fn loading_row_appears_directly_above_prompt_not_in_transcript() {
        // The spinner row must be rendered between the last transcript line and
        // the prompt box top border — never mixed into the scrollable transcript.
        let view = ShellView {
            messages: vec![MessageLineView::new(
                "● earlier message",
                MessageRole::Assistant,
            )],
            prompt: String::new(),
            loading: true,
            loading_verb: Some("thinking".into()),
            spinner_frame: 1,
            loading_elapsed_secs: 5,
            loading_total_tokens: 0,
            footer: "▸▸ default (shift+tab to cycle) · ⌃C exit".into(),
            ..ShellView::default()
        };

        let frame = render_snapshot(60, 8, &view, &Theme::default());
        let text = frame.to_plain_text();
        let lines: Vec<&str> = text.lines().collect();

        // The loading indicator must appear above the rounded prompt.
        let spinner_row = lines
            .iter()
            .position(|l| l.contains("thinking"))
            .expect("spinner row must appear");
        let prompt_border_row = lines
            .iter()
            .position(|l| l.starts_with('╭'))
            .expect("prompt border must appear");

        assert!(
            spinner_row < prompt_border_row,
            "loading row ({spinner_row}) must be above prompt marker ({prompt_border_row}); rendered:\n{text}"
        );

        // The earlier transcript message must still be visible.
        assert!(
            text.contains("earlier message"),
            "transcript must still show earlier message during loading; rendered:\n{text}"
        );
    }

    // ── new integration-panel sections (tool / mcp / lsp / todo) ────────────

    /// `tool_lines` must produce a `─ Tools ─` header followed by its body on a
    /// wide terminal, and must be silently omitted when the slice is empty.
    #[test]
    fn sidebar_tools_section_renders_when_populated() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                tool_lines: vec!["✓ Bash".into(), "✓ FileRead".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Tools ─"),
            "Tools header must appear when tool_lines is non-empty; rendered:\n{text}"
        );
        assert!(
            text.contains("Bash"),
            "tool body line must appear; rendered:\n{text}"
        );
        assert!(
            text.contains("FileRead"),
            "second tool body line must appear; rendered:\n{text}"
        );
    }

    /// An empty `tool_lines` must not produce a `─ Tools ─` header.
    #[test]
    fn sidebar_tools_section_omitted_when_empty() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                status_lines: vec!["● idle".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ Tools ─"),
            "Tools header must not appear when tool_lines is empty; rendered:\n{text}"
        );
    }

    /// `mcp_lines` must produce a `─ MCP ─` header followed by its body.
    #[test]
    fn sidebar_mcp_section_renders_when_populated() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                mcp_lines: vec!["✓ filesystem".into(), "⚠ github (reconnecting)".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ MCP ─"),
            "MCP header must appear when mcp_lines is non-empty; rendered:\n{text}"
        );
        assert!(
            text.contains("filesystem"),
            "MCP body line must appear; rendered:\n{text}"
        );
        assert!(
            text.contains("github"),
            "second MCP body line must appear; rendered:\n{text}"
        );
    }

    /// An empty `mcp_lines` must not produce a `─ MCP ─` header.
    #[test]
    fn sidebar_mcp_section_omitted_when_empty() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                status_lines: vec!["● idle".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ MCP ─"),
            "MCP header must not appear when mcp_lines is empty; rendered:\n{text}"
        );
    }

    /// `lsp_lines` must produce a `─ LSP ─` header followed by its body.
    #[test]
    fn sidebar_lsp_section_renders_when_populated() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                lsp_lines: vec!["✓ rust-analyzer".into(), "⚠ 3 errors".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ LSP ─"),
            "LSP header must appear when lsp_lines is non-empty; rendered:\n{text}"
        );
        assert!(
            text.contains("rust-analyzer"),
            "LSP body line must appear; rendered:\n{text}"
        );
    }

    /// An empty `lsp_lines` must not produce a `─ LSP ─` header.
    #[test]
    fn sidebar_lsp_section_omitted_when_empty() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                status_lines: vec!["● idle".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ LSP ─"),
            "LSP header must not appear when lsp_lines is empty; rendered:\n{text}"
        );
    }

    /// `todo_lines` must produce a `─ Todo ─` header followed by its body.
    #[test]
    fn sidebar_todo_section_renders_when_populated() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                todo_lines: vec!["✓ Write tests".into(), "◈ Refactor module".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 12, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Todo ─"),
            "Todo header must appear when todo_lines is non-empty; rendered:\n{text}"
        );
        assert!(
            text.contains("Write tests"),
            "Todo body line must appear; rendered:\n{text}"
        );
        assert!(
            text.contains("Refactor module"),
            "second Todo body line must appear; rendered:\n{text}"
        );
    }

    /// An empty `todo_lines` must not produce a `─ Todo ─` header.
    #[test]
    fn sidebar_todo_section_omitted_when_empty() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                status_lines: vec!["● idle".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ Todo ─"),
            "Todo header must not appear when todo_lines is empty; rendered:\n{text}"
        );
    }

    /// Verify the full render order: sections must appear in the documented
    /// sequence (Session → Status → Context → Tools → MCP → LSP → Todo →
    /// Suggestions → Providers → Workspace → Controls → Tasks).
    ///
    /// Status is promoted to slot 2 so the live turn-state indicator is always
    /// visible at the top of the sidebar without scrolling.
    ///
    /// We populate every section and assert that each header appears *after* its
    /// predecessor in the rendered output, using byte-offset positions.
    #[test]
    fn sidebar_sections_render_in_opencode_panel_order() {
        let view = ShellView {
            prompt: "order-check".into(),
            sidebar: Some(SidebarView {
                session_lines: vec!["◈ abc12345".into()],
                context_lines: vec!["100 / 200,000 tokens".into()],
                tool_lines: vec!["✓ Bash".into()],
                mcp_lines: vec!["✓ filesystem".into()],
                lsp_lines: vec!["✓ rust-analyzer".into()],
                todo_lines: vec!["◈ Fix lint".into()],
                suggestions: vec![ContextSuggestion {
                    severity: SuggestionSeverity::Info,
                    title: "Consider /compact".into(),
                    detail: "Free up context".into(),
                }],
                provider_lines: vec!["◈ openai · gpt-4o".into()],
                workspace_lines: vec!["⎇  main".into()],
                status_lines: vec!["● idle".into()],
                control_lines: vec!["↵ send".into()],
                task_lines: vec!["⚙  1 task".into()],
            }),
            ..ShellView::default()
        };

        // Use a very tall terminal so all sections fit in the sidebar inner area.
        let frame = render_snapshot(120, 60, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Helper: position of first occurrence in rendered text.
        let pos = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("'{needle}' not found in sidebar output:\n{text}"))
        };

        // Assert the strict ordering of every section header.
        let session_pos = pos("─ Session ─");
        let status_pos = pos("─ Status ─");
        let context_pos = pos("─ Context ─");
        let tools_pos = pos("─ Tools ─");
        let mcp_pos = pos("─ MCP ─");
        let lsp_pos = pos("─ LSP ─");
        let todo_pos = pos("─ Todo ─");
        let suggestions_pos = pos("─ Suggestions ─");
        let providers_pos = pos("─ Providers ─");
        let workspace_pos = pos("─ Workspace ─");
        let controls_pos = pos("─ Controls ─");
        let tasks_pos = pos("─ Tasks ─");

        assert!(session_pos < status_pos, "Session must precede Status");
        assert!(status_pos < context_pos, "Status must precede Context");
        assert!(context_pos < tools_pos, "Context must precede Tools");
        assert!(tools_pos < mcp_pos, "Tools must precede MCP");
        assert!(mcp_pos < lsp_pos, "MCP must precede LSP");
        assert!(lsp_pos < todo_pos, "LSP must precede Todo");
        assert!(todo_pos < suggestions_pos, "Todo must precede Suggestions");
        assert!(
            suggestions_pos < providers_pos,
            "Suggestions must precede Providers"
        );
        assert!(
            providers_pos < workspace_pos,
            "Providers must precede Workspace"
        );
        assert!(
            workspace_pos < controls_pos,
            "Workspace must precede Controls"
        );
        assert!(controls_pos < tasks_pos, "Controls must precede Tasks");
    }

    /// Status is the first section after Session so the turn-state indicator is
    /// immediately visible without scrolling, even when other sections are empty.
    #[test]
    fn sidebar_status_renders_before_context() {
        let view = ShellView {
            prompt: "hi".into(),
            sidebar: Some(SidebarView {
                session_lines: vec!["◈ abc12345".into()],
                status_lines: vec!["⟳ streaming".into()],
                context_lines: vec!["500 / 128,000 tokens".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 20, &view, &Theme::default());
        let text = frame.to_plain_text();

        let session_pos = text.find("─ Session ─").expect("Session header not found");
        let status_pos = text.find("─ Status ─").expect("Status header not found");
        let context_pos = text.find("─ Context ─").expect("Context header not found");

        assert!(
            session_pos < status_pos,
            "Session must precede Status; rendered:\n{text}"
        );
        assert!(
            status_pos < context_pos,
            "Status must precede Context (Status is promoted to slot 2); rendered:\n{text}"
        );
        // The turn-state content must also be visible.
        assert!(
            text.contains("streaming"),
            "turn-state label must be visible; rendered:\n{text}"
        );
    }

    /// Dialogs must render a blank separator line between the body text and the
    /// action-hint row so the call-to-action has visual breathing room.
    #[test]
    fn dialog_has_blank_separator_before_action_row() {
        let view = ShellView {
            title: "Session: spacing".into(),
            messages: vec![MessageLineView::new("ready", MessageRole::System)],
            prompt: "test".into(),
            dialog: Some(DialogView::notice("Info", ["This is a notice."])),
            ..ShellView::default()
        };

        let frame = render_snapshot(44, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        // The blank separator must separate the body text from "[Close]".
        // We look for an interior border row that contains only spaces (the blank line),
        // sandwiched between the body content and the action hint.
        let body_pos = text.find("This is a notice").expect("body text not found");
        let action_pos = text.find("[Close]").expect("[Close] not found");
        let between = &text[body_pos..action_pos];
        assert!(
            between.contains("│  ") || between.contains("│\n"),
            "a blank interior row must appear between body and action hint; rendered:\n{text}"
        );
    }

    /// The controller owns population of these fields each frame, so the model
    /// layer must not pre-fill them.
    #[test]
    fn from_app_state_initialises_new_panel_fields_as_empty() {
        let app = AppState::new(std::path::PathBuf::from("/workspace"));

        let view = ShellView::from_app_state(&app, "", false);
        let sidebar = view
            .sidebar
            .expect("sidebar must be Some from from_app_state");

        assert!(
            sidebar.tool_lines.is_empty(),
            "tool_lines must be empty from from_app_state; got: {:?}",
            sidebar.tool_lines
        );
        assert!(
            sidebar.mcp_lines.is_empty(),
            "mcp_lines must be empty from from_app_state; got: {:?}",
            sidebar.mcp_lines
        );
        assert!(
            sidebar.lsp_lines.is_empty(),
            "lsp_lines must be empty from from_app_state; got: {:?}",
            sidebar.lsp_lines
        );
        assert!(
            sidebar.todo_lines.is_empty(),
            "todo_lines must be empty from from_app_state; got: {:?}",
            sidebar.todo_lines
        );
    }

    /// All four new sections must be absent from a narrow terminal even when populated.
    #[test]
    fn new_sidebar_sections_absent_on_narrow_terminal() {
        let view = ShellView {
            prompt: "narrow".into(),
            sidebar: Some(SidebarView {
                tool_lines: vec!["✓ Bash".into()],
                mcp_lines: vec!["✓ filesystem".into()],
                lsp_lines: vec!["✓ rust-analyzer".into()],
                todo_lines: vec!["◈ Fix lint".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        // Width 119 is one below MIN_SIDEBAR_WIDTH=120 – sidebar is fully suppressed.
        let frame = render_snapshot(119, 20, &view, &Theme::default());
        let text = frame.to_plain_text();

        for absent in &["─ Tools ─", "─ MCP ─", "─ LSP ─", "─ Todo ─"] {
            assert!(
                !text.contains(absent),
                "{absent} must not appear on narrow terminal; rendered:\n{text}"
            );
        }
    }

    /// Each section header must appear exactly once even when all sections are
    /// populated.  A previous merge accidentally inserted duplicate render calls
    /// for Tools/MCP/LSP/Todo at the end of `sidebar_section_lines`; this test
    /// is the regression guard.
    #[test]
    fn sidebar_section_headers_appear_exactly_once() {
        let view = ShellView {
            prompt: "dup-check".into(),
            sidebar: Some(SidebarView {
                tool_lines: vec!["✓ Bash".into()],
                mcp_lines: vec!["✓ filesystem".into()],
                lsp_lines: vec!["✓ rust-analyzer".into()],
                todo_lines: vec!["◈ Fix lint".into()],
                session_lines: vec!["◈ abc12345".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(120, 40, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Every integration-panel header must appear at most once.
        for header in &["─ Tools ─", "─ MCP ─", "─ LSP ─", "─ Todo ─", "─ Session ─"]
        {
            let count = text.matches(header).count();
            assert_eq!(
                count, 1,
                "'{header}' must appear exactly once; found {count} times in:\n{text}"
            );
        }
    }
}
