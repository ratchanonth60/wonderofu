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
}

impl ShellView {
    /// Returns the height the prompt area should occupy.
    ///
    /// Compact layout: 1 separator row + content rows (no border bottom).
    #[must_use]
    pub fn prompt_height(&self) -> u16 {
        let line_count = match &self.history_search {
            Some(search) => history_search_line_count(search),
            None => text_line_count(&self.prompt),
        };
        u16::try_from(line_count)
            .unwrap_or(u16::MAX.saturating_sub(1))
            .saturating_add(1)
    }
    /// Handles from app state
    #[must_use]
    pub fn from_app_state(app: &AppState, prompt: impl Into<String>) -> Self {
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
    // Cap prompt height at roughly one third of the terminal so chat/history
    // always dominates the display.  Minimum of 2 rows (separator + one line).
    let max_prompt = (frame.area().height / 3).max(2);
    let prompt_height = view.prompt_height().min(max_prompt);
    let layout = ShellLayout::split(frame.area(), prompt_height);
    frame.fill_rect(frame.area(), ' ', theme.background);

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

    // Compact layout: a single separator rule on the top row, then content below.
    // This replaces the old rounded-border box and saves two rows for transcript.
    draw_rule(frame, area.x, area.y, area.width, theme.border);

    if area.height < 2 {
        return;
    }

    let content = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    );
    draw_lines(frame, content, &prompt_panel_lines(view, theme));
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
        };

        let frame = render_snapshot(30, 9, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Empty",
                "",
                "   ██╗    ██╗  ██████╗  ██╗",
                "   ██║    ██║ ██╔═══██╗ ██║",
                "   ██║ █╗ ██║ ██║   ██║ ██║",
                "──────────────────────────────",
                "›",
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
        };

        let frame = render_snapshot(48, 10, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Demo",
                "system> ready",
                "assistant> hello",
                "",
                "Tasks",
                "[running] shell: index workspace",
                "────────────────────────────────────────────────",
                "› /status",
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
        };

        let frame = render_snapshot(42, 12, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Demo",
                "assistant> ready",
                "",
                "",
                "Queued",
                "1. /status",
                "2. draft migration plan",
                "+2 more queued",
                "──────────────────────────────────────────",
                "› /plan",
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
                "",
                "──────────────────────────────────────────",
                "› continue?",
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
        };

        let frame = render_snapshot(42, 14, &view, &Theme::default());

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
                "──────────────────────────────────────────",
                "search: pla",
                "match 2/3",
                "draft plan",
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
        };

        let frame = render_snapshot(48, 8, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Demo",
                "tools[bash]> 1 call",
                "  • #00000000 ok · command=\"echo hi\" → done",
                "",
                "────────────────────────────────────────────────",
                "›",
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
                "",
                "────────────────────────────────────────────────",
                "›",
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
        // 10 lines, render_snapshot(20, 12) → 7 visible transcript rows (compact layout).
        // offset_from_bottom = 2 → start = (10 - 7) - 2 = 1 → shows lines 02–08.
        let view = ShellView {
            title: "Session: Scrolled".into(),
            messages: make_long_transcript(10),
            prompt: String::new(),
            status: "scrolled".into(),
            scroll: TranscriptScrollView {
                offset_from_bottom: 2,
                total_lines: 10,
                visible_lines: 7,
            },
            ..ShellView::default()
        };

        let frame = render_snapshot(20, 12, &view, &Theme::default());
        let text = frame.to_plain_text();
        assert!(text.contains("line 02"), "window start must be visible");
        assert!(text.contains("line 08"), "window end must be visible");
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
    fn prompt_height_single_line_is_separator_plus_one() {
        // Compact layout: 1 separator row + 1 content row = 2.
        let view = ShellView {
            prompt: "hello".into(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 2);
    }

    #[test]
    fn prompt_height_multiline_counts_all_lines() {
        // 3-line prompt → 1 separator + 3 content rows = 4.
        let view = ShellView {
            prompt: "line one\nline two\nline three".into(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 4);
    }

    #[test]
    fn prompt_height_empty_prompt_returns_two() {
        // An empty prompt still needs 1 separator + 1 blank content row.
        let view = ShellView {
            prompt: String::new(),
            ..ShellView::default()
        };
        assert_eq!(view.prompt_height(), 2);
    }

    #[test]
    fn render_shell_caps_prompt_to_one_third_of_terminal_height() {
        // Terminal height = 9 rows; one-third cap = max(9/3, 2) = 3 prompt rows.
        // A 10-line prompt would request prompt_height() = 11, but the renderer
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
        // The compact prompt renders the first line prefixed with '› ' and
        // subsequent lines without the marker so cursor-column arithmetic for
        // lines after the first does not need to compensate for the marker width.
        // Use a 14-row terminal so the one-third cap (14/3 = 4) matches the
        // 3-line prompt height (1 sep + 3 content = 4), ensuring all lines render.
        let view = ShellView {
            title: "Session: ML".into(),
            messages: Vec::new(),
            prompt: "first line\nsecond line\nthird line".into(),
            status: "prompt".into(),
            ..ShellView::default()
        };

        let frame = render_snapshot(30, 14, &view, &Theme::default());
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
}
