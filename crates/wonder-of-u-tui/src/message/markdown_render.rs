use std::{borrow::Cow, path::Path};

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    measure::line_width,
    style::{Color, TextStyle},
};

use super::{
    MessageLineView, MessageRole, MessageSpanView,
    rich::{MarkdownCodeBlockView, MarkdownTableView, highlighted_code_lines},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    link: bool,
}

impl InlineStyle {
    fn text_style(self) -> Option<TextStyle> {
        let mut style = TextStyle::default();
        if self.code || self.link {
            style = style.fg(Color::Cyan);
        }
        if self.bold {
            style = style.bold();
        }
        if self.italic {
            style = style.italic();
        }
        if self.strike {
            style = style.dim();
        }
        if self.link {
            style = style.underlined();
        }

        (style != TextStyle::default()).then_some(style)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BlockKind {
    Paragraph,
    Heading(HeadingLevel),
    ListItem(String),
}

#[derive(Clone, Debug)]
struct ListContext {
    next: Option<u64>,
}

#[derive(Clone, Debug)]
struct LinkContext {
    is_local: bool,
}

#[derive(Clone, Debug, Default)]
struct TableState {
    in_header: bool,
    current_cell: Option<String>,
    current_row: Option<Vec<String>>,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl TableState {
    fn start_cell(&mut self) {
        self.current_cell = Some(String::new());
    }

    fn push_text(&mut self, text: &str) {
        if let Some(cell) = &mut self.current_cell {
            cell.push_str(text);
        }
    }

    fn finish_cell(&mut self) {
        let cell = self.current_cell.take().unwrap_or_default();
        self.current_row.get_or_insert_with(Vec::new).push(cell);
    }

    fn start_row(&mut self) {
        self.current_row = Some(Vec::new());
    }

    fn finish_row(&mut self) {
        let Some(row) = self.current_row.take() else {
            return;
        };
        if self.in_header {
            self.headers = row;
        } else if !row.is_empty() {
            self.rows.push(row);
        }
    }

    fn into_table(self) -> MarkdownTableView {
        MarkdownTableView {
            headers: self.headers,
            rows: self.rows,
        }
    }
}

#[derive(Clone, Debug)]
struct Writer<'a> {
    role: MessageRole,
    max_width: usize,
    cwd: Option<&'a Path>,
    role_prefix: &'static str,
    first_prefix_pending: bool,
    lines: Vec<MessageLineView>,
    inline: InlineStyle,
    inline_stack: Vec<InlineStyle>,
    links: Vec<LinkContext>,
    list_stack: Vec<ListContext>,
    pending_item_marker: Option<String>,
    current_block: Option<BlockKind>,
    current_spans: Vec<MessageSpanView>,
    quote_depth: usize,
    code_language: Option<String>,
    code_text: Option<String>,
    table: Option<TableState>,
}

impl<'a> Writer<'a> {
    fn new(
        role: MessageRole,
        max_width: usize,
        cwd: Option<&'a Path>,
        role_prefix: &'static str,
    ) -> Self {
        Self {
            role,
            max_width: max_width.max(1),
            cwd,
            role_prefix,
            first_prefix_pending: true,
            lines: Vec::new(),
            inline: InlineStyle::default(),
            inline_stack: Vec::new(),
            links: Vec::new(),
            list_stack: Vec::new(),
            pending_item_marker: None,
            current_block: None,
            current_spans: Vec::new(),
            quote_depth: 0,
            code_language: None,
            code_text: None,
            table: None,
        }
    }

    fn run(mut self, input: &str) -> Vec<MessageLineView> {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_STRIKETHROUGH);
        let parser = Parser::new_ext(input, options);

        for event in parser {
            self.handle_event(event);
        }
        self.flush_current_block();

        if self.lines.is_empty() {
            self.lines
                .push(MessageLineView::new(self.role_prefix, self.role));
        }

        self.lines
    }

    fn handle_event(&mut self, event: Event<'_>) {
        if self.code_text.is_some() {
            match event {
                Event::Text(text) => {
                    if let Some(code) = &mut self.code_text {
                        code.push_str(&text);
                    }
                }
                Event::End(TagEnd::CodeBlock) => self.flush_code_block(),
                _ => {}
            }
            return;
        }

        if self.table.is_some() && self.handle_table_event(&event) {
            return;
        }

        match event {
            Event::Start(tag) => self.handle_start(tag),
            Event::End(end) => self.handle_end(end),
            Event::Text(text) => self.push_text(&text),
            Event::Code(code) => self.push_code_span(&code),
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.push_text("\n"),
            Event::Rule => self.push_rule(),
            Event::Html(html) | Event::InlineHtml(html) => self.push_text(&html),
            Event::TaskListMarker(checked) => {
                self.push_text(if checked { "[x] " } else { "[ ] " });
            }
            Event::FootnoteReference(reference) => self.push_text(&format!("[{reference}]")),
            Event::InlineMath(math) => self.push_text(&math),
            Event::DisplayMath(math) => {
                self.flush_current_block();
                self.start_block(BlockKind::Paragraph);
                self.push_text(&math);
                self.flush_current_block();
            }
        }
    }

    fn handle_start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                let kind = self
                    .pending_item_marker
                    .take()
                    .map(BlockKind::ListItem)
                    .unwrap_or(BlockKind::Paragraph);
                self.start_block(kind);
            }
            Tag::Heading { level, .. } => self.start_block(BlockKind::Heading(level)),
            Tag::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_add(1);
            }
            Tag::CodeBlock(kind) => {
                self.flush_current_block();
                self.code_language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|token| !token.is_empty())
                        .map(ToOwned::to_owned),
                    CodeBlockKind::Indented => None,
                };
                self.code_text = Some(String::new());
            }
            Tag::List(start) => self.list_stack.push(ListContext { next: start }),
            Tag::Item => {
                let marker = if let Some(ctx) = self.list_stack.last_mut() {
                    if let Some(next) = &mut ctx.next {
                        let marker = format!("{next}.");
                        *next = next.saturating_add(1);
                        marker
                    } else {
                        "•".to_string()
                    }
                } else {
                    "•".to_string()
                };
                self.pending_item_marker = Some(marker);
            }
            Tag::Emphasis => self.push_inline(|style| style.italic = true),
            Tag::Strong => self.push_inline(|style| style.bold = true),
            Tag::Strikethrough => self.push_inline(|style| style.strike = true),
            Tag::Link { dest_url, .. } => {
                let display = local_link_display(&dest_url, self.cwd);
                if let Some(display) = display {
                    self.ensure_text_block();
                    self.current_spans.push(MessageSpanView::new(
                        display,
                        Some(TextStyle::default().fg(Color::Cyan).underlined()),
                    ));
                    self.links.push(LinkContext { is_local: true });
                } else {
                    self.push_inline(|style| style.link = true);
                    self.links.push(LinkContext { is_local: false });
                }
            }
            Tag::Table(_) => {
                self.flush_current_block();
                self.table = Some(TableState::default());
            }
            _ => {}
        }
    }

    fn handle_end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Paragraph | TagEnd::Heading(_) => self.flush_current_block(),
            TagEnd::BlockQuote(_) => {
                self.flush_current_block();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Item => {
                self.flush_current_block();
                self.pending_item_marker = None;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_inline(),
            TagEnd::Link => {
                if let Some(link) = self.links.pop() {
                    if !link.is_local {
                        self.pop_inline();
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_table_event(&mut self, event: &Event<'_>) -> bool {
        match event {
            Event::Start(Tag::TableHead) => {
                if let Some(table) = &mut self.table {
                    table.in_header = true;
                    table.start_row();
                }
                true
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(table) = &mut self.table {
                    table.finish_row();
                    table.in_header = false;
                }
                true
            }
            Event::Start(Tag::TableRow) => {
                if let Some(table) = &mut self.table {
                    table.start_row();
                }
                true
            }
            Event::End(TagEnd::TableRow) => {
                if let Some(table) = &mut self.table {
                    table.finish_row();
                }
                true
            }
            Event::Start(Tag::TableCell) => {
                if let Some(table) = &mut self.table {
                    table.start_cell();
                }
                true
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(table) = &mut self.table {
                    table.finish_cell();
                }
                true
            }
            Event::End(TagEnd::Table) => {
                if let Some(table) = self.table.take() {
                    let mut lines = table.into_table().display_lines(self.max_width, self.role);
                    self.lines.append(&mut lines);
                    self.first_prefix_pending = false;
                }
                true
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(table) = &mut self.table {
                    table.push_text(text);
                }
                true
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(table) = &mut self.table {
                    table.push_text(" ");
                }
                true
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                if let Some(table) = &mut self.table {
                    table.push_text(
                        &local_link_display(dest_url, self.cwd)
                            .unwrap_or_else(|| dest_url.to_string()),
                    );
                }
                true
            }
            _ => false,
        }
    }

    fn start_block(&mut self, kind: BlockKind) {
        self.flush_current_block();
        self.current_block = Some(kind);
        self.current_spans.clear();
    }

    fn push_inline(&mut self, update: impl FnOnce(&mut InlineStyle)) {
        self.inline_stack.push(self.inline);
        update(&mut self.inline);
    }

    fn pop_inline(&mut self) {
        if let Some(previous) = self.inline_stack.pop() {
            self.inline = previous;
        }
    }

    fn push_text(&mut self, text: &str) {
        if self.links.last().is_some_and(|link| link.is_local) {
            return;
        }
        self.ensure_text_block();
        push_text_with_url_detection(&mut self.current_spans, text, self.inline);
    }

    fn push_code_span(&mut self, text: &str) {
        self.ensure_text_block();
        let mut inline = self.inline;
        inline.code = true;
        self.current_spans
            .push(MessageSpanView::new(text, inline.text_style()));
    }

    fn ensure_text_block(&mut self) {
        if self.current_block.is_some() {
            return;
        }
        let kind = self
            .pending_item_marker
            .take()
            .map(BlockKind::ListItem)
            .unwrap_or(BlockKind::Paragraph);
        self.current_block = Some(kind);
    }

    fn push_rule(&mut self) {
        self.flush_current_block();
        let rule = "─".repeat(self.max_width.min(24));
        self.lines.push(MessageLineView::with_spans(
            self.role,
            vec![MessageSpanView::new(
                rule,
                Some(TextStyle::default().fg(Color::DarkGrey).dim()),
            )],
        ));
        self.first_prefix_pending = false;
    }

    fn flush_current_block(&mut self) {
        let Some(kind) = self.current_block.take() else {
            self.current_spans.clear();
            return;
        };
        if self.current_spans.is_empty() {
            return;
        }

        match kind {
            BlockKind::Heading(level) => {
                let style = heading_style(level);
                let spans = self
                    .current_spans
                    .iter()
                    .cloned()
                    .map(|span| merge_span_style(span, Some(style)))
                    .collect::<Vec<_>>();
                push_wrapped_spans(
                    &mut self.lines,
                    self.role,
                    Vec::new(),
                    Vec::new(),
                    spans,
                    self.max_width,
                );
            }
            BlockKind::Paragraph => {
                if self.quote_depth > 0 {
                    let quote_style = Some(TextStyle::default().fg(Color::DarkGrey));
                    push_wrapped_spans(
                        &mut self.lines,
                        self.role,
                        vec![MessageSpanView::new("│ ", quote_style)],
                        vec![MessageSpanView::new("│ ", quote_style)],
                        self.current_spans.clone(),
                        self.max_width,
                    );
                    self.first_prefix_pending = false;
                } else {
                    let (first_prefix, continuation_prefix) = self.prefixes_for_block("", "");
                    push_wrapped_spans(
                        &mut self.lines,
                        self.role,
                        first_prefix,
                        continuation_prefix,
                        self.current_spans.clone(),
                        self.max_width,
                    );
                }
            }
            BlockKind::ListItem(marker) => {
                let item_prefix = format!("{marker} ");
                let item_continuation = " ".repeat(line_width(&item_prefix));
                let (first_prefix, continuation_prefix) =
                    self.prefixes_for_block(&item_prefix, &item_continuation);
                push_wrapped_spans(
                    &mut self.lines,
                    self.role,
                    first_prefix,
                    continuation_prefix,
                    self.current_spans.clone(),
                    self.max_width,
                );
            }
        }

        self.current_spans.clear();
    }

    fn flush_code_block(&mut self) {
        let mut code = self.code_text.take().unwrap_or_default();
        if code.ends_with('\n') {
            code.pop();
            if code.ends_with('\r') {
                code.pop();
            }
        }
        let line_count = code.lines().count().max(1);
        let code = MarkdownCodeBlockView {
            language: self.code_language.take(),
            code,
            line_count,
        };
        self.lines
            .extend(highlighted_code_lines(&code, self.role, self.max_width));
        self.first_prefix_pending = false;
    }

    fn prefixes_for_block(
        &mut self,
        marker: &str,
        marker_continuation: &str,
    ) -> (Vec<MessageSpanView>, Vec<MessageSpanView>) {
        if self.first_prefix_pending && !self.role_prefix.is_empty() {
            self.first_prefix_pending = false;
            let first = format!("{}{}", self.role_prefix, marker);
            let continuation = " ".repeat(line_width(self.role_prefix)) + marker_continuation;
            (
                vec![MessageSpanView::new(first, None)],
                vec![MessageSpanView::new(continuation, None)],
            )
        } else {
            self.first_prefix_pending = false;
            (
                (!marker.is_empty())
                    .then(|| MessageSpanView::new(marker, None))
                    .into_iter()
                    .collect(),
                (!marker_continuation.is_empty())
                    .then(|| MessageSpanView::new(marker_continuation, None))
                    .into_iter()
                    .collect(),
            )
        }
    }
}

#[must_use]
pub(super) fn render_markdown_text_with_width_and_cwd(
    input: &str,
    max_width: usize,
    cwd: Option<&Path>,
    role: MessageRole,
    role_prefix: &'static str,
) -> Vec<MessageLineView> {
    Writer::new(role, max_width, cwd, role_prefix).run(input)
}

#[must_use]
pub(super) fn normalize_agent_markdown_source(input: &str, is_streaming: bool) -> String {
    let unwrapped = unwrap_markdown_fences(input);
    if is_streaming {
        hold_back_streaming_markdown(&unwrapped).into_owned()
    } else {
        unwrapped.into_owned()
    }
}

/// Returns the byte offset where an unclosed code fence starts, if any.
#[must_use]
pub(crate) fn unclosed_fence_opening_offset(source: &str) -> Option<usize> {
    unclosed_fence_opening_offset_impl(source)
}

/// Returns the byte offset where the trailing table region starts.
///
/// A trailing table region is a sequence of consecutive non-empty lines that
/// all start with `|` at the end of `source`.  Such a region is held back from
/// the stable commit boundary while streaming because we cannot know whether
/// more rows will arrive until a non-table line (or EOF) appears.
#[must_use]
pub(crate) fn trailing_table_offset(source: &str) -> Option<usize> {
    let mut table_start: Option<usize> = None;
    let mut offset = 0usize;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            table_start = None;
        } else if trimmed.starts_with('|') {
            table_start.get_or_insert(offset);
        } else {
            table_start = None;
        }
        offset += line.len();
    }
    table_start
}

fn heading_style(level: HeadingLevel) -> TextStyle {
    let style = TextStyle::default().bold();
    match level {
        HeadingLevel::H1 => style.underlined(),
        HeadingLevel::H2 => style,
        _ => style.italic(),
    }
}

fn merge_span_style(mut span: MessageSpanView, overlay: Option<TextStyle>) -> MessageSpanView {
    if let Some(overlay) = overlay {
        span.style = Some(match span.style {
            Some(base) => merge_styles(base, overlay),
            None => overlay,
        });
    }
    span
}

fn merge_styles(base: TextStyle, overlay: TextStyle) -> TextStyle {
    TextStyle {
        fg: base.fg.or(overlay.fg),
        bg: base.bg.or(overlay.bg),
        bold: base.bold || overlay.bold,
        dim: base.dim || overlay.dim,
        italic: base.italic || overlay.italic,
        underlined: base.underlined || overlay.underlined,
        reversed: base.reversed || overlay.reversed,
    }
}

fn push_text_with_url_detection(spans: &mut Vec<MessageSpanView>, text: &str, inline: InlineStyle) {
    let Some(base_style) = inline.text_style() else {
        push_plain_text_with_urls(spans, text);
        return;
    };
    spans.push(MessageSpanView::new(text, Some(base_style)));
}

fn push_plain_text_with_urls(spans: &mut Vec<MessageSpanView>, mut text: &str) {
    while let Some(start) = find_web_url_start(text) {
        if start > 0 {
            spans.push(MessageSpanView::new(&text[..start], None));
        }
        let tail = &text[start..];
        let raw_len = tail.find(char::is_whitespace).unwrap_or(tail.len());
        let (url, punctuation) = split_trailing_url_punctuation(&tail[..raw_len]);
        if !url.is_empty() {
            spans.push(MessageSpanView::new(
                url,
                Some(TextStyle::default().fg(Color::Cyan).underlined()),
            ));
        }
        if !punctuation.is_empty() {
            spans.push(MessageSpanView::new(punctuation, None));
        }
        text = &tail[raw_len..];
    }
    if !text.is_empty() {
        spans.push(MessageSpanView::new(text, None));
    }
}

fn find_web_url_start(text: &str) -> Option<usize> {
    match (text.find("https://"), text.find("http://")) {
        (Some(https), Some(http)) => Some(https.min(http)),
        (Some(https), None) => Some(https),
        (None, Some(http)) => Some(http),
        (None, None) => None,
    }
}

fn split_trailing_url_punctuation(text: &str) -> (&str, &str) {
    let trimmed = text.trim_end_matches(['.', ',', ';', ':']);
    text.split_at(trimmed.len())
}

fn push_wrapped_spans(
    output: &mut Vec<MessageLineView>,
    role: MessageRole,
    first_prefix: Vec<MessageSpanView>,
    continuation_prefix: Vec<MessageSpanView>,
    content: Vec<MessageSpanView>,
    max_width: usize,
) {
    let logical_lines = split_logical_spans(&content);
    if logical_lines.is_empty() {
        output.push(line_from_spans(role, first_prefix));
        return;
    }

    for (logical_index, logical) in logical_lines.into_iter().enumerate() {
        let first = if logical_index == 0 {
            first_prefix.clone()
        } else {
            continuation_prefix.clone()
        };
        push_wrapped_logical_spans(
            output,
            role,
            first,
            continuation_prefix.clone(),
            logical,
            max_width,
        );
    }
}

fn push_wrapped_logical_spans(
    output: &mut Vec<MessageLineView>,
    role: MessageRole,
    first_prefix: Vec<MessageSpanView>,
    continuation_prefix: Vec<MessageSpanView>,
    content: Vec<MessageSpanView>,
    max_width: usize,
) {
    let mut current_prefix = first_prefix;
    let mut current = Vec::new();
    let mut current_width = 0usize;
    let mut available = max_width
        .saturating_sub(spans_width(&current_prefix))
        .max(1);

    for span in content {
        for grapheme in span.text.graphemes(true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);
            if current_width > 0 && current_width.saturating_add(grapheme_width) > available {
                let mut line_spans = current_prefix;
                line_spans.append(&mut current);
                output.push(line_from_spans(role, line_spans));
                current_prefix = continuation_prefix.clone();
                available = max_width
                    .saturating_sub(spans_width(&current_prefix))
                    .max(1);
                current_width = 0;
            }
            current.push(MessageSpanView::new(grapheme, span.style));
            current_width = current_width.saturating_add(grapheme_width);
        }
    }

    let mut line_spans = current_prefix;
    line_spans.append(&mut current);
    output.push(line_from_spans(role, line_spans));
}

fn split_logical_spans(spans: &[MessageSpanView]) -> Vec<Vec<MessageSpanView>> {
    let mut lines = vec![Vec::new()];
    for span in spans {
        for (index, segment) in span.text.split('\n').enumerate() {
            if index > 0 {
                lines.push(Vec::new());
            }
            if !segment.is_empty() {
                if let Some(line) = lines.last_mut() {
                    line.push(MessageSpanView::new(segment, span.style));
                }
            }
        }
    }
    lines
}

fn spans_width(spans: &[MessageSpanView]) -> usize {
    spans.iter().map(|span| line_width(&span.text)).sum()
}

fn line_from_spans(role: MessageRole, spans: Vec<MessageSpanView>) -> MessageLineView {
    if spans.iter().all(|span| span.style.is_none()) {
        MessageLineView::new(
            spans
                .into_iter()
                .map(|span| span.text)
                .collect::<Vec<_>>()
                .join(""),
            role,
        )
    } else {
        MessageLineView::with_spans(role, coalesce_spans(spans))
    }
}

fn coalesce_spans(spans: Vec<MessageSpanView>) -> Vec<MessageSpanView> {
    let mut out: Vec<MessageSpanView> = Vec::new();
    for span in spans {
        if span.text.is_empty() {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if last.style == span.style {
                last.text.push_str(&span.text);
                continue;
            }
        }
        out.push(span);
    }
    out
}

fn local_link_display(dest: &str, cwd: Option<&Path>) -> Option<String> {
    if dest.starts_with("http://")
        || dest.starts_with("https://")
        || dest.starts_with("mailto:")
        || dest.starts_with('#')
    {
        return None;
    }

    let target = dest.strip_prefix("file://").unwrap_or(dest);
    let (without_hash, hash_suffix) = split_hash_location_suffix(target);
    let (path_part, colon_suffix) = split_colon_location_suffix(without_hash);
    let suffix = hash_suffix
        .or_else(|| colon_suffix.map(str::to_string))
        .unwrap_or_default();

    if path_part.is_empty() {
        return None;
    }

    let mut display = path_part.to_string();
    if let Some(cwd) = cwd {
        let path = Path::new(path_part);
        if path.is_absolute() {
            if let Ok(relative) = path.strip_prefix(cwd) {
                display = relative.display().to_string();
            }
        }
    }
    if display.starts_with("~/") {
        display = display.to_string();
    } else if let Some(home) = std::env::var_os("HOME") {
        let home = Path::new(&home);
        let path = Path::new(path_part);
        if path.is_absolute() {
            if let Ok(relative) = path.strip_prefix(home) {
                display = format!("~/{}", relative.display());
            }
        }
    }

    Some(format!("{display}{suffix}"))
}

fn split_hash_location_suffix(target: &str) -> (&str, Option<String>) {
    let Some((path, hash)) = target.split_once('#') else {
        return (target, None);
    };
    if !is_hash_location(hash) {
        return (target, None);
    }
    let suffix = format!(":{}", hash.trim_start_matches('L').replace('C', ":"));
    (path, Some(suffix))
}

fn is_hash_location(hash: &str) -> bool {
    let hash = hash.trim();
    hash.starts_with('L') && hash[1..].chars().all(|ch| ch.is_ascii_digit() || ch == 'C')
}

fn split_colon_location_suffix(target: &str) -> (&str, Option<&str>) {
    let Some((path, suffix)) = target.rsplit_once(':') else {
        return (target, None);
    };
    if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
        return (target, None);
    }

    (path, Some(&target[path.len()..]))
}

fn unwrap_markdown_fences(markdown_source: &str) -> Cow<'_, str> {
    if !markdown_source.contains("```") && !markdown_source.contains("~~~") {
        return Cow::Borrowed(markdown_source);
    }

    let mut out = String::with_capacity(markdown_source.len());
    let mut lines = markdown_source.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let Some(open) = parse_fence_open(line) else {
            out.push_str(line);
            continue;
        };

        if !open.is_markdown {
            out.push_str(line);
            for candidate in lines.by_ref() {
                out.push_str(candidate);
                if is_fence_close(candidate, open.marker, open.len) {
                    break;
                }
            }
            continue;
        }

        let opening = line;
        let mut body = String::new();
        let mut closing = None;
        for candidate in lines.by_ref() {
            if is_fence_close(candidate, open.marker, open.len) {
                closing = Some(candidate);
                break;
            }
            body.push_str(candidate);
        }

        if closing.is_some() && markdown_contains_table(&body) {
            out.push_str(&body);
        } else {
            out.push_str(opening);
            out.push_str(&body);
            if let Some(closing) = closing {
                out.push_str(closing);
            }
        }
    }

    Cow::Owned(out)
}

#[derive(Clone, Copy, Debug)]
struct FenceOpen {
    marker: char,
    len: usize,
    is_markdown: bool,
}

fn parse_fence_open(line: &str) -> Option<FenceOpen> {
    let stripped = strip_fence_indent(line)?;
    let marker = stripped.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let len = stripped.chars().take_while(|ch| *ch == marker).count();
    if len < 3 {
        return None;
    }
    let info = stripped[len..].trim();
    let info = info.split_whitespace().next().unwrap_or_default();
    Some(FenceOpen {
        marker,
        len,
        is_markdown: matches!(info, "md" | "markdown"),
    })
}

fn strip_fence_indent(line: &str) -> Option<&str> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let mut byte_index = 0usize;
    let mut columns = 0usize;
    for byte in line.as_bytes() {
        match byte {
            b' ' => {
                byte_index += 1;
                columns += 1;
            }
            b'\t' => {
                byte_index += 1;
                columns += 4;
            }
            _ => break,
        }
        if columns >= 4 {
            return None;
        }
    }
    Some(&line[byte_index..])
}

fn is_fence_close(line: &str, marker: char, opening_len: usize) -> bool {
    let Some(stripped) = strip_fence_indent(line) else {
        return false;
    };
    let len = stripped.chars().take_while(|ch| *ch == marker).count();
    len >= opening_len && stripped[len..].trim().is_empty()
}

fn markdown_contains_table(content: &str) -> bool {
    let mut previous = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            previous = None;
            continue;
        }
        if let Some(previous_line) = previous {
            if is_table_header_line(previous_line) && is_table_delimiter_line(trimmed) {
                return true;
            }
        }
        previous = Some(trimmed);
    }
    false
}

fn hold_back_streaming_markdown(source: &str) -> Cow<'_, str> {
    if let Some(opening_start) = unclosed_fence_opening_offset_impl(source) {
        let mut out = String::with_capacity(source.len() + 1);
        out.push_str(&source[..opening_start]);
        out.push('\\');
        out.push_str(&source[opening_start..]);
        return Cow::Owned(out);
    }

    if has_trailing_header_only_table_impl(source) {
        let mut out = source.to_string();
        if let Some(offset) = out.rfind('|') {
            out.insert(offset, '\\');
            return Cow::Owned(out);
        }
    }

    Cow::Borrowed(source)
}

fn unclosed_fence_opening_offset_impl(source: &str) -> Option<usize> {
    let mut active: Option<(char, usize, usize)> = None;
    let mut offset = 0usize;
    for line in source.split_inclusive('\n') {
        if let Some((marker, len, _start)) = active {
            if is_fence_close(line, marker, len) {
                active = None;
            }
        } else if let Some(open) = parse_fence_open(line) {
            active = Some((open.marker, open.len, offset + leading_space_bytes(line)));
        }
        offset += line.len();
    }
    active.map(|(_, _, start)| start)
}

fn leading_space_bytes(line: &str) -> usize {
    line.as_bytes()
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count()
}

fn has_trailing_header_only_table_impl(source: &str) -> bool {
    let non_empty = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if non_empty.len() < 2 {
        return false;
    }
    let delimiter = non_empty[non_empty.len() - 1];
    let header = non_empty[non_empty.len() - 2];
    is_table_header_line(header) && is_table_delimiter_line(delimiter)
}

fn is_table_header_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 2
}

fn is_table_delimiter_line(line: &str) -> bool {
    let trimmed = line.trim().trim_start_matches('|').trim_end_matches('|');
    !trimmed.is_empty()
        && trimmed.split('|').all(|cell| {
            let cell = cell.trim();
            cell.len() >= 3
                && cell
                    .chars()
                    .all(|character| matches!(character, '-' | ':' | ' '))
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn texts(lines: &[MessageLineView]) -> Vec<String> {
        lines.iter().map(|line| line.text.clone()).collect()
    }

    #[test]
    fn ordered_list_item_stays_on_one_line() {
        let lines = render_markdown_text_with_width_and_cwd(
            "1. Tight item",
            80,
            None,
            MessageRole::Assistant,
            "◆ ",
        );

        assert_eq!(texts(&lines), vec!["◆ 1. Tight item"]);
    }

    #[test]
    fn markdown_fence_with_table_is_unwrapped() {
        let source = normalize_agent_markdown_source(
            "```markdown\n| Name | Age |\n| --- | --- |\n| Ada | 36 |\n```",
            false,
        );

        assert!(!source.contains("```markdown"));
        assert!(source.contains("| Name | Age |"));
    }

    #[test]
    fn non_markdown_fence_is_preserved() {
        let source = normalize_agent_markdown_source("```rust\nfn main() {}\n```", false);

        assert!(source.contains("```rust"));
    }

    #[test]
    fn local_links_render_relative_to_cwd() {
        let lines = render_markdown_text_with_width_and_cwd(
            "See [the file](/workspace/src/main.rs:12).",
            80,
            Some(Path::new("/workspace")),
            MessageRole::Assistant,
            "◆ ",
        );

        assert_eq!(lines[0].text, "◆ See src/main.rs:12.");
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|span| span.text == "src/main.rs:12"
                    && span.style.is_some_and(|style| style.underlined))
        );
    }

    #[test]
    fn web_urls_are_cyan_underlined() {
        let lines = render_markdown_text_with_width_and_cwd(
            "Visit https://example.com.",
            80,
            None,
            MessageRole::Assistant,
            "◆ ",
        );

        let url = lines[0]
            .spans
            .iter()
            .find(|span| span.text == "https://example.com")
            .expect("url span");
        assert_eq!(url.style.expect("style").fg, Some(Color::Cyan));
        assert!(url.style.expect("style").underlined);
    }
}
