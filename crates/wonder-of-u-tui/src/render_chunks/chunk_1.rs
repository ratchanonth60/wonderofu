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

    // Build styled action rows: focused action gets accent colour + ▶ prefix;
    // others are dimmed.  Each action occupies its own row for arrow navigation.
    let action_lines: Vec<StyledLine> = dialog
        .actions
        .iter()
        .enumerate()
        .map(|(i, action)| {
            let focused = i == dialog.selected_action;
            let prefix = if focused { "▶ " } else { "  " };
            let text = format!("{prefix}{}", action.label);
            if focused {
                let mut style = theme.prompt; // accent / cyan
                style.bold = true;
                StyledLine::plain(text, style)
            } else {
                StyledLine::plain(text, theme.footer)
            }
        })
        .collect();

    let hint_text = if dialog.actions.len() > 1 {
        "↑↓ navigate · Enter confirm · Esc cancel".to_string()
    } else {
        "Enter / Esc to close".to_string()
    };

    let content_width = dialog
        .body
        .iter()
        .map(|line| widest_line(line))
        .chain(action_lines.iter().map(|l| widest_line(&l.text)))
        .chain(std::iter::once(widest_line(&dialog.title)))
        .chain(std::iter::once(widest_line(&hint_text)))
        .max()
        .unwrap_or(0);
    let width = u16::try_from(content_width.saturating_add(4))
        .unwrap_or(viewport.width)
        .max(viewport.width.saturating_sub(2))
        .min(viewport.width);
    // body rows + blank + action rows + hint row + bottom border (+3 overhead).
    let height = u16::try_from(
        dialog
            .body
            .len()
            .saturating_add(2) // blank separator + hint row
            .saturating_add(dialog.actions.len())
            .saturating_add(2), // top border + bottom border
    )
    .unwrap_or(viewport.height)
    .min(viewport.height);
    let rect = Rect::new(
        viewport.x + viewport.width.saturating_sub(width) / 2,
        viewport.y + viewport.height.saturating_sub(height) / 2,
        width,
        height,
    );

    let mut body = plain_lines(&dialog.body, theme.messages);
    body.push(StyledLine::plain(String::new(), theme.footer));
    body.extend(action_lines);
    // Keyboard hint row in dim style.
    body.push(StyledLine::plain(
        hint_text,
        TextStyle::default().fg(Color::DarkGrey).dim(),
    ));
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
    let header_count = picker
        .entries
        .iter()
        .filter(|e| e.group_header.is_some())
        .count();
    let desired_height = u16::try_from(
        picker
            .entries
            .len()
            .saturating_add(header_count)
            .saturating_add(4),
    )
    .unwrap_or(viewport.height);
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

    let selected_entry_idx = picker
        .entries
        .iter()
        .position(|entry| entry.selected)
        .unwrap_or_default();

    // Build flat display rows: group headers + entries interleaved.
    let mut display_rows: Vec<DisplayRow> = Vec::with_capacity(picker.entries.len() * 2);
    for (entry_idx, entry) in picker.entries.iter().enumerate() {
        if let Some(header) = &entry.group_header {
            display_rows.push(DisplayRow::Header(header.clone()));
        }
        display_rows.push(DisplayRow::Entry(entry_idx));
    }

    // Find the selected entry's position in display_rows.
    let selected_display_pos = display_rows
        .iter()
        .position(|row| matches!(row, DisplayRow::Entry(idx) if *idx == selected_entry_idx))
        .unwrap_or_default();

    let visible = usize::from(list_area.height);
    let scroll_padding = visible / 2;
    let start = selected_display_pos
        .saturating_sub(scroll_padding)
        .min(display_rows.len().saturating_sub(visible.max(1)));

    let mut row_y = list_area.y;
    for row in display_rows.iter().skip(start).take(visible) {
        match row {
            DisplayRow::Header(header) => {
                let header_text: String = format!("── {header} ──")
                    .chars()
                    .take(usize::from(list_area.width))
                    .collect();
                frame.write_str(
                    list_area.x,
                    row_y,
                    &header_text,
                    theme.footer,
                    list_area.width,
                );
                row_y = row_y.saturating_add(1);
            }
            DisplayRow::Entry(entry_idx) => {
                let Some(entry) = picker.entries.get(*entry_idx) else {
                    continue;
                };
                // Build the optional right-aligned tag badge, e.g. "[current]".
                let tag_str: Option<String> = entry
                    .tag
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .map(|t| format!("[{t}]"));

                let tag_cols = tag_str
                    .as_deref()
                    .map_or(0_u16, |t| u16::try_from(t.chars().count()).unwrap_or(0));

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

                let main_text: String = format!("{cursor}{label}{description}")
                    .chars()
                    .take(usize::from(main_budget))
                    .collect();

                let label_style = if entry.selected {
                    theme.prompt.reversed().bold()
                } else {
                    theme.messages
                };
                frame.write_str(list_area.x, row_y, &main_text, label_style, list_area.width);

                if let Some(ref tag) = tag_str {
                    let tag_x = list_area
                        .x
                        .saturating_add(list_area.width)
                        .saturating_sub(tag_cols);
                    frame.write_str(
                        tag_x,
                        row_y,
                        tag,
                        if entry.selected {
                            theme.prompt.reversed()
                        } else {
                            theme.footer
                        },
                        tag_cols,
                    );
                }
                row_y = row_y.saturating_add(1);
            }
        }
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

