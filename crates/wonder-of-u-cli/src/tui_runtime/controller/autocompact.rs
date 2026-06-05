use super::TuiController;
use super::*;

impl TuiController<'_> {
    pub(in crate::tui_runtime) fn mark_autocompact_failed(&mut self) {
        if self.autocompact_pending {
            self.autocompact_failures = self.autocompact_failures.saturating_add(1);
            self.autocompact_pending = false;
        }
    }
    pub(in crate::tui_runtime) fn transcript_line_count(&self, terminal_width: u16) -> usize {
        let summary_width = usize::from(terminal_width.max(1));
        let mut total = if self.state.messages.is_empty() {
            0
        } else {
            message_lines_for_width(&self.state.messages, summary_width, self.expand_tool_output)
                .len()
        };
        if is_loading_turn_state(self.turn_state) {
            total = total.saturating_add(1);
        }
        total
    }
    /// Checks whether the context is close to full and, if so, queues a
    /// `/compact` command to run at the start of the next turn.
    ///
    /// Mirrors the TypeScript `autoCompact.ts` logic:
    /// - Triggers when remaining budget < `AUTOCOMPACT_BUFFER_TOKENS` (13 k).
    /// - Circuit breaker: skips after 3 consecutive compact failures so the
    ///   session doesn't hammer the summarisation API indefinitely.
    ///
    /// # Token estimation
    ///
    /// Uses a character-count heuristic (~4 chars per token for English) rather
    /// than cumulative API-reported usage, which grows monotonically and never
    /// reflects context freed by `/compact`.
    pub(in crate::tui_runtime) fn estimated_context_tokens(&self) -> u64 {
        let mut chars = 0u64;
        for msg in &self.state.messages {
            chars += estimate_payload_chars(&msg.payload);
        }
        // Rough heuristic: ~4 characters per token for English prose.
        // Clamp to at least 1 so we never report zero while messages exist.
        (chars / 4).max(1)
    }
    /// Returns the effective context window size, falling back to the model-based
    /// default when the API hasn't reported one yet.
    pub(in crate::tui_runtime) fn effective_window_or_default(&self) -> Option<u64> {
        if let Some(w) = self.state.context_window_size.filter(|&w| w > 0) {
            return Some(w);
        }
        if let Some(model) = &self.state.model {
            let effective = effective_context_window(model) as u64;
            if effective > 0 {
                return Some(effective);
            }
        }
        None
    }
    /// - Safe to call unconditionally — it is a no-op when context is fine or
    ///   when the window size is unknown.
    pub(in crate::tui_runtime) fn maybe_autocompact(&mut self) {
        const MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES: u8 = 3;

        let Some(window_size) = self.effective_window_or_default() else {
            return;
        };
        // Circuit breaker: stop trying if we've failed too many times in a row.
        if self.autocompact_failures >= MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES {
            return;
        }
        let used = self.estimated_context_tokens();
        let buffer = AUTOCOMPACT_BUFFER_TOKENS as u64;
        if used + buffer > window_size {
            self.state.queue_command("/compact", QueuePlacement::Now);
            self.autocompact_pending = true;
            self.status_note = Some(format!(
                "context ~{used}/{window_size} tokens — auto-compacting"
            ));
        }
    }
    /// Returns `true` when the current token usage has crossed the hard blocking
    /// limit, i.e. remaining tokens < `MANUAL_COMPACT_BUFFER_TOKENS` (3 k).
    ///
    /// When this is true the session must be compacted before the next API call;
    /// sending the request would result in a context-too-large error.
    pub(in crate::tui_runtime) fn is_at_blocking_limit(&self) -> bool {
        let Some(window_size) = self.effective_window_or_default() else {
            return false;
        };
        let used = self.estimated_context_tokens();
        let buffer = MANUAL_COMPACT_BUFFER_TOKENS as u64;
        used + buffer > window_size
    }
}
