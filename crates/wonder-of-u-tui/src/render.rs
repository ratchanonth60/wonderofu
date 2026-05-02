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
    pub entries: Vec<SlashSuggestionEntry>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellView {
    pub title: String,
    pub messages: Vec<MessageLineView>,
    pub prompt: String,
    pub history_search: Option<HistorySearchView>,
    pub status: String,
    pub loading: bool,
    pub loading_verb: Option<String>,
    pub footer: String,
    pub queued_panel: Option<TaskPanelView>,
    pub task_panel: Option<TaskPanelView>,
    pub dialog: Option<DialogView>,
    pub picker_view: Option<PickerView>,
    pub picker_list: Option<PickerListView>,
    pub notifications: Vec<NotificationView>,
    /// When `Some`, display the slash-command autocomplete overlay.
    pub slash_suggestions: Option<SlashSuggestionsOverlay>,
}

impl ShellView {
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
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StyledLine {
    text: String,
    style: TextStyle,
}

pub fn render_shell(frame: &mut FrameBuffer, view: &ShellView, theme: &Theme) {
    let layout = ShellLayout::split(frame.area(), view.prompt_height());
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

    draw_lines_tail(frame, transcript_area, &message_panel_lines(view, theme));

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
    if area.width < 4 || area.height < 3 {
        draw_lines(frame, area, &prompt_panel_lines(view, theme));
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
        area.x.saturating_add(2),
        area.y.saturating_add(1),
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
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
    match (
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
        return vec![StyledLine {
            text: "No messages yet.".into(),
            style: style_for_message(theme, MessageRole::System),
        }];
    }

    message_lines_to_styled(&view.messages, theme)
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
        };

        let frame = render_snapshot(30, 9, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Empty",
                "No messages yet.",
                "",
                "",
                "╭─ prompt ───────────────────╮",
                "│ ›                          │",
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
                "│ › /status                                    │",
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
                "│ › /plan                                │",
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
                "│ › continue?                            │",
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
                "╭─ prompt ───────────────────────────────╮",
                "│ search: pla                            │",
                "│ match 2/3                              │",
                "│ draft plan                             │",
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
        };

        let frame = render_snapshot(48, 8, &view, &Theme::default());

        assert_eq!(
            frame.to_plain_text(),
            [
                "▸ wonder-of-u  Demo",
                "tools[bash]> 1 call",
                "  • #00000000 ok · command=\"echo hi\" → done",
                "╭─ prompt ─────────────────────────────────────╮",
                "│ ›                                            │",
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
                "│ ›                                            │",
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
        };

        let frame = render_snapshot(32, 8, &view, &Theme::default());

        assert!(frame.to_plain_text().contains("⠿ thinking… | turn=active"));
    }
}
