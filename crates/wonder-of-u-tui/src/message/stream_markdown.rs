//! Stable streaming markdown rendering.
//!
//! This module ports the codex newline-gated streaming model:
//!
//! - [`StreamMarkdownCollector`] accumulates raw assistant text and commits only
//!   complete, stable lines (up to the last newline that is not inside an
//!   unclosed code fence or trailing table region).
//! - [`StreamRender`] keeps frozen rendered lines for committed source and a
//!   pending queue of newly-committed lines waiting to be revealed.  Only the
//!   mutable tail is re-rendered each frame; frozen lines are parsed exactly
//!   once, eliminating the flicker caused by repeatedly re-parsing the entire
//!   growing message.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use super::{MessageLineView, MessageRole, markdown_render};

/// Number of pending lines revealed per tick.
///
/// Tuned to feel responsive without dumping large blocks of text at once.
pub const REVEAL_LINES_PER_TICK: usize = 2;

/// Newline-gated markdown source collector.
///
/// Deltas are pushed into a single buffer.  [`commit_complete_source`] returns
/// the substring that has become stable since the last commit, i.e. text before
/// the first of:
///
/// - the last newline,
/// - the start of an unclosed code fence,
/// - the start of a trailing table region.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StreamMarkdownCollector {
    buffer: String,
    committed_len: usize,
}

impl StreamMarkdownCollector {
    /// Creates an empty collector.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a raw streaming delta to the buffer.
    pub fn push_delta(&mut self, delta: &str) {
        self.buffer.push_str(delta);
    }

    /// If new stable source is available, advances the committed cursor and
    /// returns the newly committed substring.
    pub fn commit_complete_source(&mut self) -> Option<&str> {
        let boundary = self.stable_boundary();
        if boundary <= self.committed_len {
            return None;
        }
        let start = self.committed_len;
        self.committed_len = boundary;
        Some(&self.buffer[start..boundary])
    }

    /// Flushes any remaining uncommitted source at the end of the stream.
    ///
    /// This should be called once when streaming completes so the final tail
    /// (which may not end with a newline) is rendered through the committed
    /// path.
    pub fn finalize_and_drain_source(&mut self) -> Option<&str> {
        if self.committed_len >= self.buffer.len() {
            return None;
        }
        let start = self.committed_len;
        self.committed_len = self.buffer.len();
        Some(&self.buffer[start..])
    }

    /// Returns the full buffered source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.buffer
    }

    /// Returns the stable, already-committed portion of the source.
    #[must_use]
    pub fn committed_source(&self) -> &str {
        &self.buffer[..self.committed_len]
    }

    /// Returns the mutable tail that has not yet been committed.
    #[must_use]
    pub fn tail_source(&self) -> &str {
        &self.buffer[self.committed_len..]
    }

    fn stable_boundary(&self) -> usize {
        let last_newline = self.buffer.rfind('\n').map(|index| index + 1).unwrap_or(0);
        let fence_boundary =
            markdown_render::unclosed_fence_opening_offset(&self.buffer[..last_newline]);
        let table_boundary = markdown_render::trailing_table_offset(&self.buffer[..last_newline]);

        let mut boundary = last_newline;
        if let Some(fence) = fence_boundary {
            boundary = boundary.min(fence);
        }
        if let Some(table) = table_boundary {
            boundary = boundary.min(table);
        }
        boundary
    }
}

/// Frozen rendered lines + pending queue for stable streaming display.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamRender {
    /// Lines that have been committed, rendered, and revealed.  These are
    /// immutable unless the terminal is resized.
    frozen: Vec<MessageLineView>,
    /// Lines that have been committed and rendered but are waiting to be
    /// revealed at a steady rate.
    pending: VecDeque<MessageLineView>,
    /// Width used for `frozen`.  Kept so `lines()` can detect caller bugs, but
    /// the controller explicitly calls `reflow()` on resize.
    width: u16,
    role: MessageRole,
    role_prefix: &'static str,
    cwd: Option<PathBuf>,
}

impl StreamRender {
    /// Creates a new renderer for a streaming assistant message.
    #[must_use]
    pub fn new(width: u16, role: MessageRole, cwd: Option<&Path>) -> Self {
        Self {
            frozen: Vec::new(),
            pending: VecDeque::new(),
            width,
            role,
            role_prefix: role_prefix(role),
            cwd: cwd.map(Path::to_path_buf),
        }
    }

    /// Renders a newly committed source chunk and appends the resulting lines
    /// to the pending queue.
    pub fn enqueue_committed(&mut self, src: &str, width: u16) {
        if src.is_empty() {
            return;
        }
        self.width = width;
        // The first visible chunk carries the role prefix; every subsequent
        // chunk continues the current block and uses an empty first prefix.
        let first_prefix = if self.frozen.is_empty() && self.pending.is_empty() {
            self.role_prefix
        } else {
            ""
        };
        let normalized = markdown_render::normalize_agent_markdown_source(src, false);
        let lines = markdown_render::render_markdown_text_with_width_and_cwd(
            &normalized,
            usize::from(width),
            self.cwd.as_deref(),
            self.role,
            first_prefix,
        );
        self.pending.extend(lines);
    }

    /// Moves up to `max` lines from the pending queue into the frozen set.
    ///
    /// Returns `true` when at least one line moved.
    pub fn reveal(&mut self, max: usize) -> bool {
        let mut changed = false;
        for _ in 0..max {
            let Some(line) = self.pending.pop_front() else {
                break;
            };
            self.frozen.push(line);
            changed = true;
        }
        changed
    }

    /// Returns the currently visible lines: frozen lines plus the rendered
    /// mutable tail.
    #[must_use]
    pub fn lines(&self, tail_source: &str, width: u16) -> Vec<MessageLineView> {
        let mut result = self.frozen.clone();

        if tail_source.is_empty() {
            if result.is_empty() {
                result.push(MessageLineView::new(self.role_prefix, self.role));
            }
            return result;
        }

        let first_prefix = if result.is_empty() {
            self.role_prefix
        } else {
            ""
        };
        let normalized = markdown_render::normalize_agent_markdown_source(tail_source, true);
        let mut tail_lines = markdown_render::render_markdown_text_with_width_and_cwd(
            &normalized,
            usize::from(width),
            self.cwd.as_deref(),
            self.role,
            first_prefix,
        );
        result.append(&mut tail_lines);
        result
    }

    /// Re-wraps the committed source after a terminal resize.
    ///
    /// All committed lines are re-rendered and moved directly to `frozen` so
    /// they remain visible; future commits continue into the pending queue.
    pub fn reflow(&mut self, committed_source: &str, width: u16) {
        self.width = width;
        self.frozen.clear();
        self.pending.clear();
        if committed_source.is_empty() {
            return;
        }
        let normalized = markdown_render::normalize_agent_markdown_source(committed_source, false);
        self.frozen = markdown_render::render_markdown_text_with_width_and_cwd(
            &normalized,
            usize::from(width),
            self.cwd.as_deref(),
            self.role,
            self.role_prefix,
        );
    }

    /// Returns the number of lines currently frozen.
    #[must_use]
    pub fn frozen_len(&self) -> usize {
        self.frozen.len()
    }

    /// Returns the number of lines waiting to be revealed.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

const fn role_prefix(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "▶ ",
        MessageRole::Assistant => "◆ ",
        MessageRole::System => "● ",
        MessageRole::Tool => "● ",
        MessageRole::Progress => "● ",
        MessageRole::Error => "● ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[MessageLineView]) -> Vec<String> {
        lines.iter().map(|line| line.text.clone()).collect()
    }

    #[test]
    fn collector_holds_back_until_newline() {
        let mut collector = StreamMarkdownCollector::new();
        collector.push_delta("hello");
        assert!(collector.commit_complete_source().is_none());
        assert_eq!(collector.tail_source(), "hello");

        collector.push_delta(" world\n");
        assert_eq!(collector.commit_complete_source(), Some("hello world\n"));
        assert!(collector.commit_complete_source().is_none());
    }

    #[test]
    fn collector_commits_multiple_complete_lines() {
        let mut collector = StreamMarkdownCollector::new();
        collector.push_delta("one\ntwo\nthree");
        assert_eq!(collector.commit_complete_source(), Some("one\ntwo\n"));
        assert_eq!(collector.tail_source(), "three");
    }

    #[test]
    fn collector_holds_back_unclosed_fence() {
        let mut collector = StreamMarkdownCollector::new();
        collector.push_delta("before\n```rust\ncode\n");
        // Everything before the fence opening is stable.
        assert_eq!(collector.commit_complete_source(), Some("before\n"));
        // Content inside the open fence is held back.
        assert!(collector.commit_complete_source().is_none());
        collector.push_delta("```\nafter\n");
        assert_eq!(
            collector.commit_complete_source(),
            Some("```rust\ncode\n```\nafter\n")
        );
    }

    #[test]
    fn collector_holds_back_trailing_table() {
        let mut collector = StreamMarkdownCollector::new();
        collector.push_delta("intro\n| Name | Age |\n");
        // Everything before the trailing table is stable.
        assert_eq!(collector.commit_complete_source(), Some("intro\n"));
        // The trailing table region is held back until it closes.
        assert!(collector.commit_complete_source().is_none());
        collector.push_delta("|------|-----|\n");
        // Still inside the trailing table region.
        assert!(collector.commit_complete_source().is_none());
        collector.push_delta("| Ada  | 36  |\n");
        // A trailing table with data rows is still held back — more rows may
        // arrive.  It commits only once a non-table line (or EOF) closes it.
        assert!(collector.commit_complete_source().is_none());
        collector.push_delta("after\n");
        assert_eq!(
            collector.commit_complete_source(),
            Some("| Name | Age |\n|------|-----|\n| Ada  | 36  |\nafter\n")
        );
    }

    #[test]
    fn collector_finalize_drains_tail() {
        let mut collector = StreamMarkdownCollector::new();
        collector.push_delta("hello");
        assert_eq!(collector.finalize_and_drain_source(), Some("hello"));
        assert!(collector.finalize_and_drain_source().is_none());
    }

    #[test]
    fn render_reveals_at_steady_rate() {
        let mut render = StreamRender::new(80, MessageRole::Assistant, None);
        // Blank-line-separated paragraphs render as three distinct lines.
        render.enqueue_committed("line one\n\nline two\n\nline three\n\n", 80);
        assert_eq!(render.pending_len(), 3);
        assert!(render.reveal(2));
        assert_eq!(render.frozen_len(), 2);
        assert_eq!(render.pending_len(), 1);
        assert!(!render.reveal(0));
        assert!(render.reveal(usize::MAX));
        assert_eq!(render.pending_len(), 0);
        assert_eq!(render.frozen_len(), 3);
    }

    #[test]
    fn render_first_chunk_gets_role_prefix() {
        let mut render = StreamRender::new(80, MessageRole::Assistant, None);
        render.enqueue_committed("hello\n", 80);
        render.reveal(usize::MAX);
        let lines = render.lines("", 80);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "◆ hello");
    }

    #[test]
    fn render_tail_continues_without_prefix_when_frozen_exists() {
        let mut render = StreamRender::new(80, MessageRole::Assistant, None);
        render.enqueue_committed("frozen line\n", 80);
        render.reveal(usize::MAX);
        let lines = render.lines("tail line", 80);
        assert_eq!(texts(&lines), vec!["◆ frozen line", "tail line"]);
    }

    #[test]
    fn render_tail_gets_prefix_when_nothing_frozen() {
        let render = StreamRender::new(80, MessageRole::Assistant, None);
        let lines = render.lines("tail line", 80);
        assert_eq!(texts(&lines), vec!["◆ tail line"]);
    }

    #[test]
    fn render_applies_bold_after_line_closes() {
        let mut render = StreamRender::new(80, MessageRole::Assistant, None);
        render.enqueue_committed("this is **bold**\n", 80);
        render.reveal(usize::MAX);
        let line = &render.lines("", 80)[0];
        assert_eq!(line.text, "◆ this is bold");
        assert!(
            line.spans
                .iter()
                .any(|span| span.text == "bold" && span.style.is_some_and(|s| s.bold)),
            "bold span should be marked bold: {:?}",
            line.spans
        );
    }

    #[test]
    fn render_fence_renders_as_block_once_closed() {
        let mut render = StreamRender::new(80, MessageRole::Assistant, None);
        // Tail contains an unclosed fence; it should render as escaped literal.
        let tail_lines = render.lines("```rust\nfn main() {}", 80);
        assert!(
            tail_lines.iter().any(|line| line.text.contains("```rust")),
            "unclosed fence should appear literally in tail: {:?}",
            tail_lines
        );

        render.enqueue_committed("```rust\nfn main() {}\n```\n", 80);
        render.reveal(usize::MAX);
        let frozen = render.lines("", 80);
        assert!(
            !frozen.iter().any(|line| line.text.contains("```rust")),
            "closed fence should render as code block, not literal: {:?}",
            frozen
        );
    }

    #[test]
    fn render_reflow_rewraps_committed_source() {
        let mut render = StreamRender::new(20, MessageRole::Assistant, None);
        render.enqueue_committed("a very long line that wraps\n", 20);
        render.reveal(usize::MAX);
        let wrapped = render.frozen_len();
        assert!(wrapped >= 2);

        let mut collector = StreamMarkdownCollector::new();
        collector.push_delta("a very long line that wraps\n");
        collector.commit_complete_source();
        render.reflow(collector.committed_source(), 80);
        assert_eq!(render.frozen_len(), 1);
    }

    #[test]
    fn render_empty_tail_shows_prefix_placeholder() {
        let render = StreamRender::new(80, MessageRole::Assistant, None);
        let lines = render.lines("", 80);
        assert_eq!(texts(&lines), vec!["◆ "]);
    }

    #[test]
    fn render_final_output_matches_non_streaming_render() {
        // Simulate streaming a message that contains a code fence and bold text.
        let source = "intro\n\n```rust\nfn main() {}\n```\n\nthis is **bold**\n";
        let mut collector = StreamMarkdownCollector::new();
        let mut render = StreamRender::new(80, MessageRole::Assistant, None);

        // Feed the source as multiple deltas and commit/reveal.
        collector.push_delta("intro\n\n```rust\nfn main()");
        while let Some(src) = collector.commit_complete_source() {
            render.enqueue_committed(src, 80);
        }
        render.reveal(usize::MAX);
        collector.push_delta(" {}\n```\n\nthis is **bold**\n");
        while let Some(src) = collector.commit_complete_source() {
            render.enqueue_committed(src, 80);
        }
        render.reveal(usize::MAX);

        let final_lines = render.lines("", 80);

        // Non-streaming render of the same source for comparison.
        let expected = markdown_render::render_markdown_text_with_width_and_cwd(
            &markdown_render::normalize_agent_markdown_source(source, false),
            80,
            None,
            MessageRole::Assistant,
            "◆ ",
        );

        assert_eq!(
            texts(&final_lines),
            texts(&expected),
            "streaming final output must match non-streaming render"
        );
    }
}
