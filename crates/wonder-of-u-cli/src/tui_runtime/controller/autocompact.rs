use super::TuiController;
use super::*;

/// Minutes of idle gap (since the last assistant message) after which the
/// provider prompt cache is guaranteed cold, so clearing old tool results
/// costs nothing extra. Mirrors `timeBasedMCConfig.ts` (`gapThresholdMinutes`).
const TIME_BASED_MC_GAP_MINUTES: i64 = 60;

/// How many most-recent compactable tool results to keep when the time-based
/// microcompact fires. Mirrors `timeBasedMCConfig.ts` (`keepRecent`).
const TIME_BASED_MC_KEEP_RECENT: usize = 5;

/// Replacement content for cleared tool results.
/// Mirrors `TIME_BASED_MC_CLEARED_MESSAGE` in `microCompact.ts`.
pub(in crate::tui_runtime) const TIME_BASED_MC_CLEARED_MESSAGE: &str =
    "[Old tool result content cleared]";

/// Tools whose results may be content-cleared by microcompact — outputs that
/// can be re-derived (re-read, re-run, re-fetch). Mirrors `COMPACTABLE_TOOLS`
/// in `microCompact.ts`.
const COMPACTABLE_TOOLS: &[&str] = &[
    "bash",
    "shell",
    "powershell",
    "file_read",
    "file_write",
    "file_edit",
    "glob",
    "grep",
    "web_fetch",
    "web_search",
];

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
    /// Records the token usage reported by the latest provider response so
    /// context estimation can anchor on real API numbers.
    ///
    /// Call after the assistant output for that response has been appended to
    /// `state.messages` — messages at indices `>= anchor` are assumed to be
    /// *not* covered by the recorded usage and are estimated separately.
    pub(in crate::tui_runtime) fn note_context_usage(&mut self, usage: TokenUsage) {
        if usage.is_zero() {
            // Some providers omit usage; keep the previous anchor (or the
            // full-transcript estimate fallback) rather than recording zero.
            return;
        }
        self.last_context_usage = Some(usage.total_tokens());
        self.context_usage_anchor = self.state.messages.len();
    }
    /// Forgets the recorded usage anchor. Call whenever `state.messages` is
    /// rewritten wholesale (`/clear`, `/compact`) so stale usage from the
    /// pre-rewrite context can't leak into estimates.
    pub(in crate::tui_runtime) fn reset_context_usage_tracking(&mut self) {
        self.last_context_usage = None;
        self.context_usage_anchor = 0;
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
    /// Hybrid, mirroring `tokenCountWithEstimation` in `tokens.ts`: anchor on
    /// the token usage reported by the latest API response (input + cache
    /// creation + cache read + output), then add a character-count estimate
    /// (~4 chars per token) only for messages appended after that response.
    /// Falls back to a full-transcript character estimate when no usage has
    /// been recorded yet (fresh or resumed session) or the anchor is stale.
    pub(in crate::tui_runtime) fn estimated_context_tokens(&self) -> u64 {
        let anchored_usage = self
            .last_context_usage
            .filter(|_| self.context_usage_anchor <= self.state.messages.len());
        let tail_start = if anchored_usage.is_some() {
            self.context_usage_anchor
        } else {
            0
        };
        let mut chars = 0u64;
        for msg in &self.state.messages[tail_start..] {
            chars += estimate_payload_chars(&msg.payload);
        }
        // Rough heuristic: ~4 characters per token for English prose.
        // Clamp to at least 1 so we never report zero while messages exist.
        (anchored_usage.unwrap_or(0) + chars / 4).max(1)
    }
    /// Returns the effective context window size — the raw window minus the
    /// reserve for compaction summary output — falling back to the model-based
    /// default when the API hasn't reported a window yet.
    ///
    /// Mirrors `getEffectiveContextWindowSize` in `autoCompact.ts`.
    pub(in crate::tui_runtime) fn effective_window_or_default(&self) -> Option<u64> {
        if let Some(w) = self.state.context_window_size.filter(|&w| w > 0) {
            return Some(w.saturating_sub(COMPACT_MAX_OUTPUT_TOKENS as u64).max(1));
        }
        if let Some(model) = &self.state.model {
            let effective = effective_context_window(model) as u64;
            if effective > 0 {
                return Some(effective);
            }
        }
        None
    }
    /// Returns the autocompact trigger threshold: tokens above this queue a
    /// `/compact`. When autocompact is disabled the threshold is the full
    /// effective window (only the warning/blocking checks remain meaningful).
    pub(in crate::tui_runtime) fn autocompact_threshold(&self) -> Option<u64> {
        let window = self.effective_window_or_default()?;
        if self.state.auto_compact_enabled {
            Some(window.saturating_sub(AUTOCOMPACT_BUFFER_TOKENS as u64))
        } else {
            Some(window)
        }
    }
    /// Returns `true` when usage has crossed the warning threshold
    /// (`threshold - WARNING_THRESHOLD_BUFFER_TOKENS`, i.e. within 20 k tokens
    /// of compaction). Mirrors `calculateTokenWarningState` in `autoCompact.ts`.
    pub(in crate::tui_runtime) fn context_warning_active(&self) -> bool {
        let Some(threshold) = self.autocompact_threshold() else {
            return false;
        };
        let warning = threshold.saturating_sub(WARNING_THRESHOLD_BUFFER_TOKENS as u64);
        self.estimated_context_tokens() >= warning
    }
    /// - Safe to call unconditionally — it is a no-op when context is fine or
    ///   when the window size is unknown.
    pub(in crate::tui_runtime) fn maybe_autocompact(&mut self) {
        const MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES: u8 = 3;

        if !self.state.auto_compact_enabled {
            return;
        }
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
    ///
    /// `WONDER_OF_U_BLOCKING_LIMIT_OVERRIDE` replaces the computed limit with an
    /// absolute token count (mirrors `CLAUDE_CODE_BLOCKING_LIMIT_OVERRIDE`).
    pub(in crate::tui_runtime) fn is_at_blocking_limit(&self) -> bool {
        let Some(window_size) = self.effective_window_or_default() else {
            return false;
        };
        let blocking_limit = std::env::var("WONDER_OF_U_BLOCKING_LIMIT_OVERRIDE")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&v| v > 0)
            .unwrap_or_else(|| window_size.saturating_sub(MANUAL_COMPACT_BUFFER_TOKENS as u64));
        self.estimated_context_tokens() > blocking_limit
    }
    /// Time-based microcompact: when the session has been idle long enough
    /// that the provider prompt cache is cold (gap > 60 min since the last
    /// assistant message), clear the content of old compactable tool results
    /// — the full prefix gets rewritten on the next request anyway, so the
    /// clearing is free. Keeps the most recent
    /// [`TIME_BASED_MC_KEEP_RECENT`] results intact.
    ///
    /// Mirrors `maybeTimeBasedMicrocompact` in `microCompact.ts`.
    pub(in crate::tui_runtime) fn maybe_time_based_microcompact(&mut self) {
        let Some(last_assistant_at) = self
            .state
            .messages
            .iter()
            .rev()
            .find(|msg| {
                matches!(
                    msg.payload,
                    MessagePayload::AssistantText { .. }
                        | MessagePayload::AssistantThinking { .. }
                        | MessagePayload::AssistantToolUse { .. }
                )
            })
            .map(|msg| msg.timestamp)
        else {
            return;
        };
        let gap = time::OffsetDateTime::now_utc() - last_assistant_at;
        if gap < time::Duration::minutes(TIME_BASED_MC_GAP_MINUTES) {
            return;
        }

        // Compactable tool_use ids in encounter order; keep the newest N.
        let compactable: Vec<ToolUseId> = self
            .state
            .messages
            .iter()
            .filter_map(|msg| match &msg.payload {
                MessagePayload::AssistantToolUse { tool, use_id, .. }
                    if COMPACTABLE_TOOLS.contains(&tool.as_str()) =>
                {
                    Some(*use_id)
                }
                _ => None,
            })
            .collect();
        // Floor at 1 like the TS source: clearing everything would leave the
        // model with zero working context.
        let keep_recent = TIME_BASED_MC_KEEP_RECENT.max(1);
        if compactable.len() <= keep_recent {
            return;
        }
        let keep = &compactable[compactable.len() - keep_recent..];

        let mut freed_chars = 0u64;
        let mut cleared = 0usize;
        for msg in &mut self.state.messages {
            if let MessagePayload::ToolResult {
                use_id, content, ..
            } = &mut msg.payload
            {
                if compactable.contains(use_id)
                    && !keep.contains(use_id)
                    && content != TIME_BASED_MC_CLEARED_MESSAGE
                {
                    freed_chars += content.len() as u64;
                    *content = TIME_BASED_MC_CLEARED_MESSAGE.to_string();
                    cleared += 1;
                }
            }
        }
        if cleared == 0 {
            return;
        }

        // The anchored usage still reflects the pre-clear content (the API
        // counted it), so subtract the rough estimate of what was freed —
        // mirrors the `snipTokensFreed` adjustment in `shouldAutoCompact`.
        let freed_tokens = freed_chars / 4;
        if let Some(usage) = &mut self.last_context_usage {
            *usage = usage.saturating_sub(freed_tokens);
        }
        self.status_note = Some(format!(
            "cleared {cleared} old tool results (~{freed_tokens} tokens) after idle gap"
        ));
        self.needs_render = true;
    }
}
