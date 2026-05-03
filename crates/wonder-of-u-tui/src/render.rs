use wonder_of_u_core::AppState;

use crate::{
    dialog::DialogView,
    frame::{FrameBuffer, Rect},
    layout::ShellLayout,
    measure::widest_line,
    message::{
        HistorySearchView, MessageLineView, MessageRole, PickerListView, PickerView, TaskPanelView,
        footer_text, message_lines, queued_panel_view, status_text, task_panel_view,
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
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SidebarView {
    /// Section 1 – Session: turn state, title, status note etc.
    /// Each entry is one display row.
    pub session_lines: Vec<String>,
    /// Section 2 – Context: token/cost usage summary.
    pub context_lines: Vec<String>,
    /// Section 3 – Providers: one entry per line, active model marked with `◈`.
    pub provider_lines: Vec<String>,
    /// Section 4 – Status: turn detail, loading verb, error snippets.
    pub status_lines: Vec<String>,
    /// Section 5 – Controls: compact keybindings.
    pub control_lines: Vec<String>,
    /// Section 6 – Workspace: cwd, git, storage, runtime labels.
    pub workspace_lines: Vec<String>,
    /// Section 7 – Tasks: background task count + hints.
    pub task_lines: Vec<String>,
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
    /// Scroll position snapshot for windowed transcript rendering.
    pub scroll: TranscriptScrollView,
    /// Right-side companion panel shown beside all shell content on wide terminals.
    ///
    /// `None` suppresses the panel entirely (e.g. when constructed manually in
    /// tests or when no provider context is available yet).
    pub sidebar: Option<SidebarView>,
}

impl ShellView {
    /// Returns the height the prompt area should occupy.
    ///
    /// Bordered-box layout: 1 top-border row + content rows + 1 bottom-border row.
    #[must_use]
    pub fn prompt_height(&self) -> u16 {
        let line_count = match &self.history_search {
            Some(search) => history_search_line_count(search),
            None => text_line_count(&self.prompt),
        };
        u16::try_from(line_count)
            .unwrap_or(u16::MAX.saturating_sub(2))
            .saturating_add(2)
    }
    /// Handles from app state
    #[must_use]
    pub fn from_app_state(app: &AppState, prompt: impl Into<String>) -> Self {
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

            // Section 2 – Context: placeholder for future token/cost usage.
            let context_lines: Vec<String> = Vec::new();

            // Section 3 – Providers: one line combining provider and model.
            let provider_lines = match (&app.provider, &app.model) {
                (Some(provider), Some(model)) => vec![format!("{provider} · {model}")],
                _ => Vec::new(),
            };

            // Section 4 – Status: stub idle line; controller overwrites this each frame.
            let status_lines = vec!["● idle".into()];

            // Section 5 – Controls: compact keybinding reference.
            let control_lines = vec![
                "↵ send  ⇧↵ newline".into(),
                "⎋ cancel  ? help".into(),
                "⌃B sidebar  ⌃C exit".into(),
            ];

            // Section 6 – Workspace: git branch and short cwd label.
            let mut workspace_lines: Vec<String> = Vec::new();
            if let Some(branch) = &app.session.git_branch {
                workspace_lines.push(format!("⎇  {branch}"));
            }
            // Use the last path component as a short cwd label.
            if let Some(cwd) = app.session.cwd.file_name().and_then(|n| n.to_str()) {
                workspace_lines.push(format!("  {cwd}"));
            }

            // Section 7 – Tasks: background task count (omitted when none).
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
                provider_lines,
                status_lines,
                control_lines,
                workspace_lines,
                task_lines,
            }
        };
        Self {
            title: format!("Session: {}", app.session.title),
            messages: message_lines(&app.messages),
            prompt: prompt.into(),
            history_search: None,
            status: status_text(app),
            loading: false,
            loading_verb: None,
            footer: footer_text(app),
            queued_panel: queued_panel_view(app),
            task_panel: task_panel_view(app),
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            // Default to follow-tail; the controller will override this each frame.
            scroll: TranscriptScrollView::default(),
            sidebar: Some(sidebar),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StyledLine {
    text: String,
    style: TextStyle,
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
            let main_w = area.width.saturating_sub(SIDEBAR_WIDTH + 1);
            let sep_x = area.x.saturating_add(main_w);
            // Full-height vertical separator between main content and sidebar.
            for row in 0..area.height {
                frame.put(sep_x, area.y.saturating_add(row), '│', theme.border);
            }
            let sidebar_area =
                Rect::new(sep_x.saturating_add(1), area.y, SIDEBAR_WIDTH, area.height);
            draw_shell_sidebar(frame, sidebar_area, sidebar, theme);
            Rect::new(area.x, area.y, main_w, area.height)
        } else {
            area
        }
    } else {
        area
    };

    // Cap prompt height at roughly one third of the terminal so chat/history
    // always dominates the display.  Minimum of 3 rows (top border + one content
    // line + bottom border) to keep the box chrome intact.
    let max_prompt = (main_area.height / 3).max(3);
    let prompt_height = view.prompt_height().min(max_prompt);
    let layout = ShellLayout::split(main_area, prompt_height);

    draw_message_view(frame, layout.messages, view, theme);
    draw_prompt_view(frame, layout.prompt, view, theme);
    draw_status_line(frame, layout.status, &status_line_text(view), theme.status);
    draw_footer_line(frame, layout.footer, &view.footer, theme);
    draw_notification_stack(frame, layout.messages, &view.notifications, theme);

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
            draw_slash_suggestions(frame, layout.messages, overlay, theme);
        }
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
            frame.write_str(inner.x, y, &line.text, line.style, inner.width);
        }
        return;
    }

    if let Some(first_line) = lines.first() {
        frame.write_str(
            area.x,
            area.y,
            &first_line.text,
            first_line.style,
            area.width,
        );
    }
}

fn draw_message_view(frame: &mut FrameBuffer, area: Rect, view: &ShellView, theme: &Theme) {
    if area.is_empty() {
        return;
    }

    frame.fill_rect(area, ' ', theme.background);

    let title_height = u16::from(!view.title.is_empty() && area.height > 0);
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
    if view.messages.is_empty() {
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
    if area.width < 3 || area.height < 3 {
        draw_lines(frame, area, &prompt_panel_lines(view, theme));
        return;
    }

    // Bordered-box layout: rounded corners with a " prompt " label on the top
    // border, matching the pre-compact UX.
    draw_rounded_border(frame, area, theme.border);
    frame.write_str(
        area.x.saturating_add(2),
        area.y,
        " prompt ",
        theme.footer,
        area.width.saturating_sub(4),
    );

    // Prompt content fills the full inner width — the sidebar (when shown) lives
    // in its own column at shell level, so nothing shrinks the typing area.
    let content = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    draw_lines(frame, content, &prompt_panel_lines(view, theme));
}

/// Minimum total terminal width required to activate the shell-level sidebar.
///
/// Below this threshold all content occupies the full terminal width,
/// preserving readability on narrow terminals.
pub const MIN_SIDEBAR_WIDTH: u16 = 100;

/// Column width of the sidebar panel (excluding the `│` separator).
pub const SIDEBAR_WIDTH: u16 = 22;

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
/// assert_eq!(shell_main_area_width(100, true), 100 - SIDEBAR_WIDTH - 1);
///
/// // Wide terminal without sidebar: no deduction.
/// assert_eq!(shell_main_area_width(100, false), 100);
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
/// The sidebar receives the full terminal height, so all hint rows are always
/// visible regardless of the current prompt size.
fn draw_shell_sidebar(frame: &mut FrameBuffer, area: Rect, sidebar: &SidebarView, theme: &Theme) {
    if area.is_empty() {
        return;
    }
    draw_lines(frame, area, &sidebar_section_lines(sidebar, theme));
}

/// Builds the ordered list of styled lines for the sidebar panel.
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

    // (header text, section body lines)
    let sections: &[(&str, &[String])] = &[
        ("─ Session ─", &sidebar.session_lines),
        ("─ Context ─", &sidebar.context_lines),
        ("─ Providers ─", &sidebar.provider_lines),
        ("─ Status ─", &sidebar.status_lines),
        ("─ Controls ─", &sidebar.control_lines),
        ("─ Workspace ─", &sidebar.workspace_lines),
        ("─ Tasks ─", &sidebar.task_lines),
    ];

    let mut out: Vec<StyledLine> = Vec::new();

    for (header, body) in sections {
        if body.is_empty() {
            continue;
        }

        // Blank separator before each section (except at the very top).
        if !out.is_empty() {
            out.push(StyledLine {
                text: String::new(),
                style: dim,
            });
        }

        // Section header row.
        out.push(StyledLine {
            text: (*header).to_owned(),
            style: theme.title,
        });

        // Body lines with prefix-driven colouring.
        for line in *body {
            let style = if line.starts_with('✓') {
                TextStyle::default().fg(Color::Green)
            } else if line.starts_with('⚠') {
                TextStyle::default().fg(Color::Yellow)
            } else if line.starts_with('◈') {
                accent
            } else {
                dim
            };
            out.push(StyledLine {
                text: line.clone(),
                style,
            });
        }
    }

    out
}

fn docked_panel_lines(view: &ShellView, theme: &Theme) -> Vec<StyledLine> {
    let mut lines = Vec::new();

    if let Some(task_panel) = &view.task_panel {
        lines.extend(panel_lines(task_panel, theme));
    }

    if let Some(queued_panel) = &view.queued_panel {
        if !lines.is_empty() {
            lines.push(StyledLine {
                text: String::new(),
                style: theme.background,
            });
        }
        lines.extend(panel_lines(queued_panel, theme));
    }

    lines
}

fn panel_lines(panel: &TaskPanelView, theme: &Theme) -> Vec<StyledLine> {
    let mut lines = vec![StyledLine {
        text: panel.title.clone(),
        style: theme.title,
    }];
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
        frame.write_str(area.x, y, &line.text, line.style, area.width);
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
    let height = u16::try_from(dialog.body.len().saturating_add(3))
        .unwrap_or(viewport.height)
        .min(viewport.height);
    let rect = Rect::new(
        viewport.x + viewport.width.saturating_sub(width) / 2,
        viewport.y + viewport.height.saturating_sub(height) / 2,
        width,
        height,
    );

    let mut body = plain_lines(&dialog.body, theme.messages);
    body.push(StyledLine {
        text: actions,
        style: theme.status,
    });
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

    let mut y = viewport.y.saturating_add(u16::from(viewport.height > 3));
    for notification in notifications {
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
        if y.saturating_add(height) > viewport.bottom() {
            break;
        }

        let rect = Rect::new(viewport.right().saturating_sub(width), y, width, height);
        draw_notification(frame, rect, &title, notification, &lines, theme);
        y = y.saturating_add(height);
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
        .map(|line| StyledLine {
            text: line.clone(),
            style: if notification.focused {
                theme.messages.bold()
            } else {
                theme.messages
            },
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
        &[StyledLine {
            text: preview.into(),
            style: theme.messages,
        }],
        theme,
    );
}

fn draw_slash_suggestions(
    frame: &mut FrameBuffer,
    viewport: Rect,
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

    // Anchor to bottom-left of viewport, just above the prompt rule.
    let rect = Rect::new(
        viewport.x,
        viewport.bottom().saturating_sub(panel_height),
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
            StyledLine {
                text,
                style: if e.selected {
                    theme.prompt.bold()
                } else {
                    theme.messages
                },
            }
        })
        .collect();

    let border_theme = Theme {
        border: theme.prompt,
        title: theme.prompt.bold(),
        ..*theme
    };
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
        let tag = entry
            .tag
            .as_deref()
            .filter(|tag| !tag.is_empty())
            .map(|tag| format!(" [{tag}]"))
            .unwrap_or_default();
        let description = if entry.description.is_empty() {
            String::new()
        } else {
            format!(" — {}", entry.description)
        };
        if entry.selected {
            frame.fill_rect(
                Rect::new(list_area.x, y, list_area.width, 1),
                ' ',
                theme.prompt.reversed().bold(),
            );
        }
        let text = format!(
            "{} {}{}{}",
            if entry.selected { "▸" } else { " " },
            entry.label,
            tag,
            description
        );
        let style = if entry.selected {
            theme.prompt.reversed().bold()
        } else {
            theme.messages
        };
        frame.write_str(list_area.x, y, &text, style, list_area.width);
    }
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
    let base = match (
        view.loading,
        view.loading_verb.as_deref(),
        view.status.is_empty(),
    ) {
        (true, Some(verb), false) => format!("⠿ {verb}… | {}", view.status),
        (true, Some(verb), true) => format!("⠿ {verb}…"),
        (true, None, false) => format!("⠿ {}", view.status),
        (true, None, true) => "⠿".into(),
        (false, _, false) => format!("◆ {}", view.status),
        (false, _, true) => "◆".into(),
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
        return welcome_panel_lines(theme);
    }
    message_lines_to_styled(&view.messages, theme)
}

fn welcome_panel_lines(theme: &Theme) -> Vec<StyledLine> {
    let art = theme.title;
    let dim = theme.footer;
    let body = theme.messages;
    macro_rules! l {
        ($s:expr, $st:expr) => {
            StyledLine {
                text: $s.into(),
                style: $st,
            }
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
        .map(|line| StyledLine {
            text: line.text.clone(),
            style: style_for_message(theme, line.role),
        })
        .collect()
}

fn prompt_panel_lines(view: &ShellView, theme: &Theme) -> Vec<StyledLine> {
    let Some(search) = &view.history_search else {
        let mut lines = split_lines(&view.prompt);
        if let Some(first) = lines.first_mut() {
            *first = format!("› {first}");
        }
        return plain_lines(&lines, theme.prompt);
    };

    let mut lines = vec![
        StyledLine {
            text: format!("search: {}", search.query),
            style: theme.status,
        },
        StyledLine {
            text: history_match_label(search),
            style: theme.footer,
        },
    ];
    lines.extend(plain_lines(
        &split_lines(search.match_text.as_deref().unwrap_or("")),
        theme.prompt,
    ));
    lines
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

fn plain_lines(lines: &[String], style: TextStyle) -> Vec<StyledLine> {
    lines
        .iter()
        .map(|line| StyledLine {
            text: line.clone(),
            style,
        })
        .collect()
}

fn style_for_message(theme: &Theme, role: MessageRole) -> TextStyle {
    match role {
        MessageRole::User => {
            let mut style = theme.prompt;
            style.bold = true;
            style
        }
        MessageRole::Assistant => theme.messages,
        MessageRole::System => theme.footer,
        MessageRole::Tool => {
            let mut style = theme.status;
            style.bold = false;
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

fn history_search_line_count(search: &HistorySearchView) -> usize {
    2usize.saturating_add(text_line_count(search.match_text.as_deref().unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use wonder_of_u_core::{
        AppState, MessageEnvelope, MessagePayload, TaskState, TaskStatus, TokenUsage,
    };

    use super::*;
    use crate::{dialog::DialogView, message::MessageLineView};

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
            footer: "ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
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
                "╰────────────────────────────╯",
                "◆ prompt | 0 messages",
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
                MessageLineView::new("assistant> hello", MessageRole::Assistant),
            ],
            prompt: "/status".into(),
            history_search: None,
            status: "prompt | 2 messages".into(),
            loading: false,
            loading_verb: None,
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
            scroll: TranscriptScrollView::default(),
            sidebar: None,
        };

        let frame = render_snapshot(48, 10, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Demo",
                "system> ready",
                "assistant> hello",
                "Tasks",
                "[running] shell: index workspace",
                "╭─ prompt ─────────────────────────────────────╮",
                "│› /status                                     │",
                "╰──────────────────────────────────────────────╯",
                "◆ prompt | 2 messages",
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

        let view = ShellView::from_app_state(&app, "/help");

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
            messages: vec![MessageLineView::new(
                "assistant> ready",
                MessageRole::Assistant,
            )],
            prompt: "/plan".into(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
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
            scroll: TranscriptScrollView::default(),
            sidebar: None,
        };

        let frame = render_snapshot(42, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Demo",
                "assistant> ready",
                "",
                "Queued",
                "1. /status",
                "2. draft migration plan",
                "+2 more queued",
                "╭─ prompt ───────────────────────────────╮",
                "│› /plan                                 │",
                "╰────────────────────────────────────────╯",
                "◆ prompt | 1 messages",
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
            scroll: TranscriptScrollView::default(),
            sidebar: None,
        };

        let frame = render_snapshot(42, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "",
                " ╭─Confirm action───────────────────────╮",
                " │Approve command execution             │",
                " │This cannot be undone                 │",
                " │[Confirm]  Cancel                     │",
                " ╰──────────────────────────────────────╯",
                "",
                "╭─ prompt ───────────────────────────────╮",
                "│› continue?                             │",
                "╰────────────────────────────────────────╯",
                "◆ permission | 1 messages",
                "        cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn history_search_snapshot_renders_overlay_and_preview() {
        let view = ShellView {
            title: "Session: Search".into(),
            messages: vec![MessageLineView::new(
                "assistant> ready",
                MessageRole::Assistant,
            )],
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
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
        };

        let frame = render_snapshot(42, 16, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Search",
                "assistant> ready",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "╭─ prompt ───────────────────────────────╮",
                "│search: pla                             │",
                "│match 2/3                               │",
                "│draft plan                              │",
                "╰────────────────────────────────────────╯",
                "◆ prompt | 1 messages",
                "        cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn transcript_snapshot_keeps_latest_visible_lines() {
        let view = ShellView {
            title: "Session: Tail".into(),
            messages: (1..=6)
                .map(|index| {
                    MessageLineView::new(format!("assistant> line {index}"), MessageRole::Assistant)
                })
                .collect(),
            prompt: "tail".into(),
            history_search: None,
            status: "prompt | 6 messages".into(),
            loading: false,
            loading_verb: None,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
        };

        let frame = render_snapshot(32, 9, &view, &Theme::default());

        assert!(!frame.to_plain_text().contains("assistant> line 1"));
        assert!(frame.to_plain_text().contains("assistant> line 6"));
    }

    #[test]
    fn shell_snapshot_renders_grouped_tool_summary_lines() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![
                MessageLineView::new("tools[bash]> 1 call", MessageRole::Tool),
                MessageLineView::new(
                    "  • #00000000 ok · command=\"echo hi\" → done",
                    MessageRole::Tool,
                ),
            ],
            prompt: String::new(),
            history_search: None,
            status: "prompt | 2 messages".into(),
            loading: false,
            loading_verb: None,
            footer: "cwd=/workspace | ctrl-c interrupt".into(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
        };

        let frame = render_snapshot(48, 8, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Demo",
                "tools[bash]> 1 call",
                "  • #00000000 ok · command=\"echo hi\" → done",
                "╭─ prompt ─────────────────────────────────────╮",
                "│›                                             │",
                "╰──────────────────────────────────────────────╯",
                "◆ prompt | 2 messages",
                "              cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn shell_snapshot_renders_notification_stack_overlay() {
        let view = ShellView {
            title: "Session: Demo".into(),
            messages: vec![MessageLineView::new(
                "assistant> ready",
                MessageRole::Assistant,
            )],
            prompt: String::new(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
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
            scroll: TranscriptScrollView::default(),
            sidebar: None,
        };

        let frame = render_snapshot(48, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Demo",
                "assistant> ready   ╭─info Source status────────╮",
                "                   │workspace index refreshed  │",
                "                   ╰───────────────────────────╯",
                "                      ╭─ok Task update • focus─╮",
                "                      │tests passed            │",
                "                      ╰────────────────────────╯",
                "╭─ prompt ─────────────────────────────────────╮",
                "│›                                             │",
                "╰──────────────────────────────────────────────╯",
                "◆ prompt | 1 messages",
                "              cwd=/workspace · ctrl-c interrupt",
            ]
            .join("\n")
        );
    }

    #[test]
    fn shell_snapshot_renders_picker_list_overlay() {
        let view = ShellView {
            title: "Session: Picker".into(),
            messages: vec![MessageLineView::new(
                "assistant> ready",
                MessageRole::Assistant,
            )],
            prompt: String::new(),
            history_search: None,
            status: "prompt | 1 messages".into(),
            loading: false,
            loading_verb: None,
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
            scroll: TranscriptScrollView::default(),
            sidebar: None,
        };

        let frame = render_snapshot(60, 16, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(text.contains("Select Theme"));
        assert!(text.contains("Search: mid"));
        assert!(text.contains("Midnight [current] — Dark theme"));
        assert!(text.contains("↑↓ navigate  Tab/Enter select  Esc cancel"));
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
            footer: String::new(),
            queued_panel: None,
            task_panel: None,
            dialog: None,
            picker_view: None,
            picker_list: None,
            notifications: Vec::new(),
            slash_suggestions: None,
            scroll: TranscriptScrollView::default(),
            sidebar: None,
        };

        let frame = render_snapshot(32, 8, &view, &Theme::default());

        assert!(frame.to_plain_text().contains("⠿ thinking… | turn=active"));
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
        // 10 lines, render_snapshot(20, 12) → 6 visible transcript rows (box layout).
        // offset_from_bottom = 3 → start = (10 - 6) - 3 = 1 → shows lines 02–07.
        let view = ShellView {
            title: "Session: Scrolled".into(),
            messages: make_long_transcript(10),
            prompt: String::new(),
            status: "scrolled".into(),
            scroll: TranscriptScrollView {
                offset_from_bottom: 3,
                total_lines: 10,
                visible_lines: 6,
            },
            ..ShellView::default()
        };

        let frame = render_snapshot(20, 12, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(text.contains("line 02"), "window start must be visible");
        assert!(text.contains("line 07"), "window end must be visible");
        assert!(
            !text.contains("line 01"),
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
    fn prompt_height_single_line_is_box_row_count() {
        // Box layout: 1 top border + 1 content row + 1 bottom border = 3.
        let view = ShellView {
            prompt: "hello".into(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 3);
    }

    #[test]
    fn prompt_height_multiline_counts_all_lines() {
        // 3-line prompt → 1 top border + 3 content rows + 1 bottom border = 5.
        let view = ShellView {
            prompt: "line one\nline two\nline three".into(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 5);
    }

    #[test]
    fn prompt_height_empty_prompt_returns_three() {
        // An empty prompt has 1 content row (the › marker) → 1 + 2 borders = 3.
        let view = ShellView {
            prompt: String::new(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 3);
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
            messages: vec![MessageLineView::new(
                "assistant> hello",
                MessageRole::Assistant,
            )],
            prompt: ten_line_prompt,
            status: "prompt".into(),
            ..ShellView::default()
        };

        // The uncapped prompt would be 11 rows tall; with a 9-row terminal the
        // render should not panic and the transcript line must still be visible.
        let frame = render_snapshot(40, 9, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(
            text.contains("assistant> hello"),
            "transcript must survive tall prompt; rendered:\n{text}"
        );
    }

    // ── multiline prompt marker and cursor offset ────────────────────────────

    #[test]
    fn multiline_prompt_first_line_has_marker_continuation_lines_do_not() {
        // Box prompt for 3 lines → prompt_height() = 5 (top border + 3 content +
        // bottom border).  At height=16 the one-third cap = max(16/3, 3) = 5,
        // which exactly fits all three content lines inside the box.
        let view = ShellView {
            title: "Session: ML".into(),
            messages: Vec::new(),
            prompt: "first line\nsecond line\nthird line".into(),
            status: "prompt".into(),
            ..ShellView::default()
        };

        let frame = render_snapshot(30, 16, &view, &Theme::default());
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

        let view = ShellView::from_app_state(&app, "");
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
        // Width 99 is one below the MIN_SIDEBAR_WIDTH threshold — the sidebar
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

        let frame = render_snapshot(99, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Sidebar section headers must not appear on a narrow terminal.
        assert!(
            !text.contains("─ Providers ─"),
            "sidebar hints must be absent on narrow terminal; rendered:\n{text}"
        );
    }

    #[test]
    fn sidebar_renders_on_wide_terminal_at_min_width() {
        // Width == MIN_SIDEBAR_WIDTH (100) must activate the sidebar.
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

        let frame = render_snapshot(100, 8, &view, &Theme::default());
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

        let frame = render_snapshot(100, 6, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("Enter"),
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

        let frame = render_snapshot(100, 8, &view, &Theme::default());
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

        let frame = render_snapshot(100, 8, &view, &Theme::default());
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

        let frame = render_snapshot(100, 10, &view, &Theme::default());
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

        let frame = render_snapshot(100, 10, &view, &Theme::default());
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

        let frame = render_snapshot(100, 10, &view, &Theme::default());
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

        let view = ShellView::from_app_state(&app, "");

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
    }

    #[test]
    fn from_app_state_populates_sidebar_git_branch_and_cwd() {
        let mut app = AppState::new(PathBuf::from("/workspace/myproject"));
        app.session.git_branch = Some("feat/my-feature".into());

        let view = ShellView::from_app_state(&app, "");

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
    fn sidebar_multiline_prompt_uses_full_main_column_width() {
        // The sidebar lives at shell level; the prompt box takes the entire
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

        // width=100 → main_w=77, sidebar=22, sep=1.
        // height=14 → max_prompt=4; prompt_height=5 (3 content+2 border).min(4)=4.
        // Box shows 2 content rows: "first line" and "second line".
        let frame = render_snapshot(100, 14, &view, &Theme::default());
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
    fn prompt_box_spans_full_terminal_width_on_narrow_terminal() {
        // Width 99 is one below MIN_SIDEBAR_WIDTH.  Even when `sidebar` is Some,
        // no column is carved out — the prompt box must use the full 99 columns.
        let view = ShellView {
            prompt: "hello".into(),
            sidebar: Some(SidebarView {
                provider_lines: vec!["◈ copilot · gpt-4".into()],
                ..SidebarView::default()
            }),
            ..ShellView::default()
        };

        let frame = render_snapshot(99, 6, &view, &Theme::default());
        let text = frame.to_plain_text();

        // The bottom border line starts with '╰' and ends with '╯'.
        // to_plain_text trims trailing spaces, so '╯' is preserved as the last char.
        let bottom_border = text
            .lines()
            .find(|l| l.starts_with('╰'))
            .expect("bottom border line must be present");

        assert!(
            bottom_border.ends_with('╯'),
            "prompt box right corner must be at column 98; got: {bottom_border:?}"
        );
        // The border line must span all 99 columns — no sidebar column was carved out.
        let col_count = bottom_border.chars().count();
        assert_eq!(
            col_count, 99,
            "prompt box border must be 99 cols wide on narrow terminal; got {col_count}"
        );
    }

    #[test]
    fn prompt_view_does_not_panic_on_tiny_terminal() {
        // When the prompt area is width < 3 or height < 3, draw_prompt_view falls back
        // to writing content lines directly without box chrome.  Verify no panic and
        // that the '›' marker still appears (content is written to the area).
        let view = ShellView {
            prompt: "hi".into(),
            ..ShellView::default()
        };

        // height=3 → chrome=2, available=1, prompt_height capped to 1 → area.height=1
        // which is < 3, so the no-box fallback path is taken.
        let frame = render_snapshot(10, 3, &view, &Theme::default());
        let text = frame.to_plain_text();

        // Must not panic; the prompt marker '›' must still appear.
        assert!(
            text.contains('›'),
            "prompt marker must appear even without box chrome; rendered:\n{text}"
        );
        // No rounded-box corners should be present in the fallback path.
        assert!(
            !text.contains('╭'),
            "box corners must be absent in tiny-terminal fallback; rendered:\n{text}"
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

        let frame = render_snapshot(100, 10, &view, &Theme::default());
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

        let frame = render_snapshot(100, 10, &view, &Theme::default());
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
    fn sidebar_absent_on_narrow_terminal_below_100_cols() {
        // Narrow terminal (width < MIN_SIDEBAR_WIDTH = 100): sidebar must be
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

        let frame = render_snapshot(99, 8, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            !text.contains("─ Providers ─"),
            "Providers header must NOT appear on narrow (99-col) terminal; rendered:\n{text}"
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

        let frame = render_snapshot(100, 10, &view, &Theme::default());
        let text = frame.to_plain_text();

        assert!(
            text.contains("─ Status ─"),
            "Status header must appear; rendered:\n{text}"
        );
        // Empty sections must produce no headers.
        for absent in &[
            "─ Session ─",
            "─ Context ─",
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
}
