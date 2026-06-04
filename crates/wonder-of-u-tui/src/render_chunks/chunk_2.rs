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

/// Renders the loading area: live shell output lines (if any) + spinner row.
///
/// `area` spans all reserved rows carved from the bottom of the message area.
/// The last row is always the spinner; rows above it show the most recent lines
/// of shell stdout streaming from the running tool.
pub(super) fn draw_loading_area(
    frame: &mut FrameBuffer,
    area: Rect,
    view: &ShellView,
    theme: &Theme,
) {
    if area.is_empty() {
        return;
    }

    let dim_style = {
        let mut s = theme.footer;
        s.dim = true;
        s
    };
    let spinner_style = {
        let mut s = theme.status;
        s.dim = true;
        s
    };

    // Show the last (height-1) tool progress lines above the spinner row.
    let total_rows = area.height as usize;
    if total_rows > 1 && !view.tool_progress.is_empty() {
        let line_rows = total_rows.saturating_sub(1);
        let lines = &view.tool_progress;
        let start = lines.len().saturating_sub(line_rows);
        for (i, line) in lines[start..].iter().enumerate() {
            let y = area.y + i as u16;
            frame.fill_rect(
                Rect::new(area.x, y, area.width, 1),
                ' ',
                theme.background,
            );
            // Truncate visually long lines and prefix with a guide character.
            let display = format!("  ⎿  {line}");
            frame.write_str(area.x, y, &display, dim_style, area.width);
        }
    }

    // Spinner row is always the last row.
    let spinner_y = area.y + area.height.saturating_sub(1);
    let spinner_area = Rect::new(area.x, spinner_y, area.width, 1);
    frame.fill_rect(spinner_area, ' ', theme.background);
    let text = build_loading_progress_text(view);
    frame.write_str(area.x, spinner_y, &text, spinner_style, area.width);
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

    // Use split('\n') rather than str::lines() so a trailing '\n' (inserted
    // when the user presses Shift+Enter at the end of a line) produces a
    // visible blank continuation row in the prompt box.  str::lines() silently
    // drops the trailing empty segment, which caused the prompt height to be
    // under-counted and the cursor to be clamped onto the last text row.
    text.split('\n').map(ToString::to_string).collect()
}

fn text_line_count(text: &str) -> usize {
    // split('\n') preserves a trailing newline as an extra blank row so that
    // prompt_height() and cursor-geometry helpers stay consistent with what
    // split_lines() renders.  str::lines() would drop it and under-count by 1.
    text.split('\n').count().max(1)
}

