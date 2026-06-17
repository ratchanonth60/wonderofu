/// Renders shell
pub fn render_shell(frame: &mut FrameBuffer, view: &ShellView, theme: &Theme) {
    let area = frame.area();
    frame.fill_rect(area, ' ', theme.background);

    // Wide-terminal sidebar: when the terminal is at least MIN_SIDEBAR_WIDTH columns
    // wide and the view carries sidebar data, carve out a right-hand column.
    // All main-content drawing (transcript + prompt box + chrome) is then confined
    // to the narrower left column — the prompt box keeps its full allocated width.
    let (main_area, sidebar_overlay) = if let Some(sidebar) = &view.sidebar {
        if area.width >= MIN_SIDEBAR_WIDTH {
            match view.sidebar_mode {
                SidebarMode::Push => {
                    // Solid-panel sidebar with a 1-column left gutter for visual
                    // separation from the main transcript area.  SIDEBAR_WIDTH + 1
                    // columns total: the gutter plus the panel fill.
                    let main_w = area.width.saturating_sub(SIDEBAR_WIDTH + 1);
                    let sep_x = area.x.saturating_add(main_w);
                    let sidebar_area = Rect::new(sep_x, area.y, SIDEBAR_WIDTH + 1, area.height);
                    draw_shell_sidebar(frame, sidebar_area, sidebar, theme);
                    (Rect::new(area.x, area.y, main_w, area.height), None)
                }
                SidebarMode::Overlay => {
                    // Full-width main area; the sidebar will be drawn as an overlay
                    // *after* the transcript and prompt, painting over the right edge.
                    // The +1 reserves a 1-column left gutter matching push mode.
                    let main_w = area.width;
                    let sidebar_area =
                        Rect::new(
                            area.x.saturating_add(main_w.saturating_sub(SIDEBAR_WIDTH + 1)),
                            area.y,
                            SIDEBAR_WIDTH + 1,
                            area.height,
                        );
                    (area, Some((sidebar_area, sidebar)))
                }
            }
        } else {
            (area, None)
        }
    } else {
        (area, None)
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

    // When loading, reserve rows at the bottom of the message area for the Claude-style
    // progress row and any live shell output lines.
    let progress_line_count = if view.loading && !view.tool_progress.is_empty() {
        view.tool_progress.len().min(3) as u16 // show at most 3 live lines
    } else {
        0
    };
    // 1 row for the spinner + optional N rows for live output above it.
    let reserved_rows = if view.loading && layout.messages.height >= 1 {
        1 + progress_line_count
    } else {
        0
    };
    let (messages_render_area, loading_row_area) = if reserved_rows > 0
        && layout.messages.height > reserved_rows
    {
        let transcript_h = layout.messages.height.saturating_sub(reserved_rows);
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
                reserved_rows,
            )),
        )
    } else {
        (layout.messages, None)
    };

    draw_message_view(frame, messages_render_area, view, theme);

    // Draw the loading area (live progress lines + spinner row), if active.
    // Padded like the transcript so the spinner column lines up with messages.
    if let Some(loading_area) = loading_row_area {
        draw_loading_area(frame, transcript_padded(loading_area), view, theme);
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

    if let Some(overlay) = &view.fleet_panel {
        draw_fleet_panel(frame, layout.messages, overlay, theme);
    }

    // In overlay mode the sidebar is drawn last so it paints over the transcript.
    if let Some((sidebar_area, sidebar)) = sidebar_overlay {
        draw_shell_sidebar(frame, sidebar_area, sidebar, theme);
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

    // Keep transcript content off the pane edges; the wrap width used to build
    // the message lines (`transcript_wrap_width`) accounts for this inset.
    let area = transcript_padded(area);

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
    _theme: &Theme,
) {
    if area.is_empty() {
        return;
    }

    let (style, bg) = match warning.severity {
        PromptWarningSeverity::Warning => (
            TextStyle::default().fg(Color::Black).bold(),
            Color::DarkYellow,
        ),
        PromptWarningSeverity::Critical => (
            TextStyle::default().fg(Color::White).bold(),
            Color::Red,
        ),
    };
    frame.fill_rect(area, ' ', style.bg(bg));
    let text = format!(" ⚠  {}", warning.text);
    frame.write_str(area.x, area.y, &text, style.bg(bg), area.width);
}

/// Minimum total terminal width required to activate the shell-level sidebar.
///
/// 120 cols = 77-col main area + 1 gutter + SIDEBAR_WIDTH (42) + 0 pad.
/// Below this threshold all content occupies the full terminal width.
pub const MIN_SIDEBAR_WIDTH: u16 = 120;

/// Column width of the sidebar panel (excluding the 1-column left gutter).
pub const SIDEBAR_WIDTH: u16 = 42;

/// Returns the effective main-column width used by [`render_shell`].
///
/// When `has_sidebar` is `true` and `terminal_width` meets the
/// [`MIN_SIDEBAR_WIDTH`] threshold, the sidebar column (and the 1-column
/// left gutter) are subtracted exactly as the renderer does.  Pass this
/// result as the layout width to cursor-position helpers so they stay in
/// sync with the renderer.
///
/// # Examples
///
/// ```
/// use wonder_of_u_tui::{shell_main_area_width, MIN_SIDEBAR_WIDTH, SIDEBAR_WIDTH};
///
/// // Narrow terminal: no deduction regardless of sidebar flag.
/// assert_eq!(shell_main_area_width(80, true), 80);
///
/// // Wide terminal with sidebar: subtracts sidebar + gutter.
/// assert_eq!(shell_main_area_width(MIN_SIDEBAR_WIDTH, true), MIN_SIDEBAR_WIDTH - SIDEBAR_WIDTH - 1);
///
/// // Wide terminal without sidebar: no deduction.
/// assert_eq!(shell_main_area_width(MIN_SIDEBAR_WIDTH, false), MIN_SIDEBAR_WIDTH);
/// ```
pub fn shell_main_area_width(terminal_width: u16, has_sidebar: bool) -> u16 {
    if has_sidebar && terminal_width >= MIN_SIDEBAR_WIDTH {
        terminal_width.saturating_sub(SIDEBAR_WIDTH + 1)
    } else {
        terminal_width
    }
}

/// Horizontal padding (in cells) between the main-pane edges and transcript
/// content, so message text never touches the terminal border or the sidebar
/// gutter.
pub const TRANSCRIPT_HPAD: u16 = 1;

/// Width available for wrapping transcript message lines inside a main pane of
/// `main_area_width` cells — the pane width minus [`TRANSCRIPT_HPAD`] on each
/// side. Use this wherever message lines are wrapped or counted so the wrap
/// width always matches the renderer's padded transcript area.
///
/// # Examples
///
/// ```
/// use wonder_of_u_tui::{transcript_wrap_width, TRANSCRIPT_HPAD};
///
/// assert_eq!(transcript_wrap_width(80), 80 - 2 * TRANSCRIPT_HPAD);
/// // Degenerate panes never collapse to zero.
/// assert_eq!(transcript_wrap_width(1), 1);
/// ```
pub fn transcript_wrap_width(main_area_width: u16) -> u16 {
    main_area_width
        .saturating_sub(2 * TRANSCRIPT_HPAD)
        .max(1)
}

/// Insets `area` by [`TRANSCRIPT_HPAD`] on the left and right, leaving the
/// vertical extent untouched.
fn transcript_padded(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(TRANSCRIPT_HPAD),
        area.y,
        area.width.saturating_sub(2 * TRANSCRIPT_HPAD).max(1),
        area.height,
    )
}

/// Draws the right-side sidebar showing keybinding hints and session metadata.
///
/// Matches opencode's solid-panel sidebar: no border, just a flat fill in
/// `theme.panel`.  The inner area is split into three fixed vertical bands:
///
/// - **Top** — the Session title ([`sidebar_title_lines`]), pinned, never scrolls.
/// - **Middle** — every other section ([`sidebar_body_lines`]), windowed by
///   `sidebar.scroll_offset` with an overflow indicator when content exceeds
///   the visible height.
/// - **Bottom** — a single-row branded footer (`● {footer_brand}`), pinned.
///
/// Inner padding follows opencode: 1 row top/bottom, 2 columns left/right.
fn draw_shell_sidebar(frame: &mut FrameBuffer, area: Rect, sidebar: &SidebarView, theme: &Theme) {
    if area.is_empty() {
        return;
    }

    // Solid panel fill — no border, matching opencode's flat sidebar surface.
    frame.fill_rect(area, ' ', theme.panel);

    // Opencode-style inner padding: 1 row top/bottom, 2 cols left/right.
    let inner = sidebar_inner_area(area);
    if inner.is_empty() {
        return;
    }

    const FOOTER_HEIGHT: u16 = 1;

    let title_lines = with_panel_bg(sidebar_title_lines(sidebar, theme), theme);
    let title_height = u16::try_from(title_lines.len())
        .unwrap_or(u16::MAX)
        .min(inner.height);

    let remaining_after_title = inner.height.saturating_sub(title_height);
    let footer_height = FOOTER_HEIGHT.min(remaining_after_title);
    let middle_height = remaining_after_title.saturating_sub(footer_height);

    let title_area = Rect::new(inner.x, inner.y, inner.width, title_height);
    let middle_area = Rect::new(
        inner.x,
        inner.y.saturating_add(title_height),
        inner.width,
        middle_height,
    );
    let footer_area = Rect::new(
        inner.x,
        inner.bottom().saturating_sub(footer_height),
        inner.width,
        footer_height,
    );

    // Top band — Session title, always pinned, never windowed.
    draw_lines(frame, title_area, &title_lines);

    // Middle band — every other section, windowed by scroll_offset.
    let body_lines = with_panel_bg(sidebar_body_lines(sidebar, theme), theme);
    let visible = usize::from(middle_area.height);
    let total = body_lines.len();
    let max_offset = total.saturating_sub(visible);
    let offset = sidebar.scroll_offset.min(max_offset);
    let slice = if offset < body_lines.len() {
        &body_lines[offset..(offset + visible).min(body_lines.len())]
    } else {
        &[]
    };
    draw_lines(frame, middle_area, slice);

    // Scroll indicator: overlay the last row of the middle band when content
    // overflows (kept inside the middle band, not the panel border — there is
    // no border in the solid-panel style).
    if total > visible && middle_area.height > 0 {
        let indicator = if offset > 0 && offset < max_offset {
            format!("▲▼ {}/{}", offset + visible, total)
        } else if offset > 0 {
            format!("▼ {}/{}", offset + visible, total)
        } else {
            format!("▲ {}/{}", offset + visible, total)
        };
        let indicator_y = middle_area.bottom().saturating_sub(1);
        frame.write_str(
            middle_area.x,
            indicator_y,
            &indicator,
            theme.footer.bg_or(theme.panel),
            middle_area.width,
        );
    }

    // Bottom band — pinned branded footer: "● {footer_brand}".
    if footer_area.height > 0 {
        draw_sidebar_footer(frame, footer_area, &sidebar.footer_brand, theme);
    }
}

/// Computes the scrollable middle-band geometry of the solid-panel sidebar,
/// mirroring the band split in [`draw_shell_sidebar`].
///
/// Returns `(visible_rows, total_body_lines)` for a sidebar panel occupying the
/// full `terminal_height`.  The controller's scroll-key and clamp logic must use
/// this so the offset range stays in lock-step with what the renderer windows —
/// only the middle band scrolls; the Session title and branded footer are pinned.
///
/// Line counts are independent of the concrete [`Theme`], so a default theme is
/// used internally.
#[must_use]
pub fn sidebar_scroll_geometry(terminal_height: u16, sidebar: &SidebarView) -> (usize, usize) {
    let theme = Theme::default();
    // inner height = panel height minus the 1-row top/bottom padding.
    let inner_h = terminal_height.saturating_sub(2);
    let title_h = u16::try_from(sidebar_title_lines(sidebar, &theme).len())
        .unwrap_or(u16::MAX)
        .min(inner_h);
    let remaining = inner_h.saturating_sub(title_h);
    let footer_h = 1u16.min(remaining);
    let middle = remaining.saturating_sub(footer_h);
    let total = sidebar_body_lines(sidebar, &theme).len();
    (usize::from(middle), total)
}

/// Insets `area` with opencode-style sidebar padding: 1 row top/bottom and
/// 2 columns left/right.
fn sidebar_inner_area(area: Rect) -> Rect {
    const HPAD: u16 = 2;
    const VPAD: u16 = 1;
    Rect::new(
        area.x.saturating_add(HPAD),
        area.y.saturating_add(VPAD),
        area.width.saturating_sub(HPAD.saturating_mul(2)),
        area.height.saturating_sub(VPAD.saturating_mul(2)),
    )
}

/// Draws the pinned bottom footer row: a green `●` dot followed by the dim
/// branded version string, e.g. `"● wonder-of-u v0.1.3"`.
fn draw_sidebar_footer(frame: &mut FrameBuffer, area: Rect, footer_brand: &str, theme: &Theme) {
    let line = StyledLine {
        text: String::new(),
        style: theme.panel,
        spans: vec![
            StyledSpan {
                text: "● ".to_owned(),
                style: TextStyle::default()
                    .fg(Color::Green)
                    .bg_or(theme.panel),
            },
            StyledSpan {
                text: footer_brand.to_owned(),
                style: theme.footer.bg_or(theme.panel),
            },
        ],
    };
    draw_styled_line(frame, area.x, area.y, &line, area.width);
}

/// Merges `theme.panel`'s background colour into every line/span style that
/// does not already carry an explicit `bg`, so sidebar text composes onto the
/// solid panel fill instead of leaving the cell's background unset.
fn with_panel_bg(lines: Vec<StyledLine>, theme: &Theme) -> Vec<StyledLine> {
    lines
        .into_iter()
        .map(|line| StyledLine {
            text: line.text,
            style: line.style.bg_or(theme.panel),
            spans: line
                .spans
                .into_iter()
                .map(|span| StyledSpan {
                    text: span.text,
                    style: span.style.bg_or(theme.panel),
                })
                .collect(),
        })
        .collect()
}

/// Builds the pinned title band: the Session section only.
///
/// Unlike [`sidebar_body_lines`], this never includes a leading blank
/// separator — it is always the first thing drawn in the panel.
fn sidebar_title_lines(sidebar: &SidebarView, theme: &Theme) -> Vec<StyledLine> {
    let dim = theme.footer;
    let accent = theme.prompt;

    let mut out: Vec<StyledLine> = Vec::new();
    push_sidebar_text_section(
        &mut out,
        "─ Session ─",
        &sidebar.session_lines,
        dim,
        accent,
        theme,
    );
    out
}

/// Builds the ordered list of styled lines for the sidebar's scrollable
/// middle band (everything except the Session title and the branded footer).
///
/// Renders sections in the OpenCode-style integration-panel order.
/// Status is promoted to position 1 (of this band) so the live turn-state
/// indicator is always visible at the top of the scroll region:
///
/// ```text
/// 1. Status       – turn state / loading verb  (promoted)
/// 2. Context      – token usage bar
/// 3. Tools        – active tool names / call counts
/// 4. MCP          – connected MCP server states
/// 5. LSP          – language-server diagnostics summary
/// 6. Todo         – in-session checklist items
/// 7. Suggestions  – proactive context-saving hints
/// 8. Providers    – model + provider
/// 9. Workspace    – git branch, cwd
/// 10. Controls    – compact keybinding reference
/// 11. Tasks       – background task count
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
fn sidebar_body_lines(sidebar: &SidebarView, theme: &Theme) -> Vec<StyledLine> {
    let dim = theme.footer;
    let accent = theme.prompt;

    let mut out: Vec<StyledLine> = Vec::new();

    // 1 – Status (promoted: turn-state is the most urgent real-time signal)
    push_sidebar_text_section(
        &mut out,
        "─ Status ─",
        &sidebar.status_lines,
        dim,
        accent,
        theme,
    );
    // 2 – Context
    push_sidebar_text_section(
        &mut out,
        "─ Context ─",
        &sidebar.context_lines,
        dim,
        accent,
        theme,
    );
    // 3 – Tools
    push_sidebar_text_section(
        &mut out,
        "─ Tools ─",
        &sidebar.tool_lines,
        dim,
        accent,
        theme,
    );
    // 4 – MCP
    push_sidebar_text_section(&mut out, "─ MCP ─", &sidebar.mcp_lines, dim, accent, theme);
    // 5 – LSP
    push_sidebar_text_section(&mut out, "─ LSP ─", &sidebar.lsp_lines, dim, accent, theme);
    // 6 – Todo
    push_sidebar_text_section(
        &mut out,
        "─ Todo ─",
        &sidebar.todo_lines,
        dim,
        accent,
        theme,
    );
    // 7 – Suggestions (span-coloured; handled by its own helper)
    push_sidebar_suggestions_section(&mut out, &sidebar.suggestions, theme);
    // 8 – Providers
    push_sidebar_text_section(
        &mut out,
        "─ Providers ─",
        &sidebar.provider_lines,
        dim,
        accent,
        theme,
    );
    // 9 – Workspace
    push_sidebar_text_section(
        &mut out,
        "─ Workspace ─",
        &sidebar.workspace_lines,
        dim,
        accent,
        theme,
    );
    // 10 – Controls
    push_sidebar_text_section(
        &mut out,
        "─ Controls ─",
        &sidebar.control_lines,
        dim,
        accent,
        theme,
    );
    // 11 – Tasks
    push_sidebar_text_section(
        &mut out,
        "─ Tasks ─",
        &sidebar.task_lines,
        dim,
        accent,
        theme,
    );

    // Plugin-injected sidebar slots
    for slot in &sidebar.slots {
        let header = format!("─ {} ─", slot.title);
        push_sidebar_text_section(&mut out, &header, &slot.lines, dim, accent, theme);
    }

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

