//! Ephemeral transcript scroll state for the TUI controller.
//!
//! [`TranscriptScrollState`] tracks how far the user has scrolled up in the
//! message transcript.  Nothing here is persisted to storage; the struct lives
//! only for the lifetime of a single TUI session.

/// Ephemeral scroll position for the TUI transcript viewport.
///
/// `offset_from_bottom == 0` means the view is pinned to the live tail
/// ("follow-tail" mode).  Any positive value means the user has scrolled up by
/// that many rendered transcript lines.
///
/// # Follow-tail semantics
///
/// When `offset_from_bottom` is `0` and new messages arrive, the viewport
/// automatically shows the newest content (no action required from the caller
/// beyond calling [`on_messages_changed`](TranscriptScrollState::on_messages_changed)).
/// When the user has scrolled up, new messages do **not** pull the viewport
/// down; the existing visual position relative to the bottom is preserved and
/// clamped if the transcript grows or shrinks.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct TranscriptScrollState {
    /// Lines above the bottom of the transcript that sit at the top of the
    /// current viewport.  Zero → follow-tail (live mode).
    pub(super) offset_from_bottom: usize,
    /// Cached visible height of the transcript area (in terminal rows) from
    /// the last [`on_resize`](TranscriptScrollState::on_resize) call.
    pub(super) last_visible_lines: usize,
    /// Cached total rendered message line count from the last
    /// [`on_messages_changed`](TranscriptScrollState::on_messages_changed) or
    /// [`on_resize`](TranscriptScrollState::on_resize) call.
    pub(super) last_total_lines: usize,
}

impl TranscriptScrollState {
    /// Creates a new scroll state in follow-tail mode with no cached dimensions.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when the viewport is pinned to the live tail.
    ///
    /// This is the default mode; it is restored by calling
    /// [`scroll_to_bottom`](TranscriptScrollState::scroll_to_bottom).
    #[allow(dead_code)]  // only used in tests, not from production call sites
    #[must_use]
    pub(super) fn is_following_tail(&self) -> bool {
        self.offset_from_bottom == 0
    }

    /// Maximum valid offset before all content is older than visible.
    ///
    /// Returns `0` when the transcript fits entirely within the visible
    /// viewport (no scrolling is possible).
    #[must_use]
    fn max_offset(&self) -> usize {
        self.last_total_lines.saturating_sub(self.last_visible_lines)
    }

    /// Clamps `offset_from_bottom` to `[0, max_offset()]`.
    fn clamp(&mut self) {
        self.offset_from_bottom = self.offset_from_bottom.min(self.max_offset());
    }

    /// Updates cached dimensions after a terminal resize and re-clamps the offset.
    ///
    /// `visible_lines` is the height of the transcript viewport in terminal
    /// rows (i.e. `ShellLayout::messages.height` after the resize).
    /// `total_lines` is the current rendered transcript line count.
    ///
    /// Both follow-tail mode and scrolled-up mode remain logically consistent
    /// after this call: the viewport is clamped to the new bounds.
    pub(super) fn on_resize(&mut self, visible_lines: usize, total_lines: usize) {
        self.last_visible_lines = visible_lines;
        self.last_total_lines = total_lines;
        self.clamp();
    }

    /// Updates the cached total line count when messages are added or removed.
    ///
    /// * **Follow-tail mode** (`offset == 0`): stays at `0`; the newest
    ///   content will be shown automatically by the renderer.
    /// * **Scrolled-up mode** (`offset > 0`): the visual distance from the
    ///   bottom is preserved; the offset is clamped if the transcript shrank.
    pub(super) fn on_messages_changed(&mut self, total_lines: usize) {
        self.last_total_lines = total_lines;
        // Clamping is a no-op in follow-tail mode (offset=0 ≤ max_offset).
        // In scrolled-up mode it prevents the offset from overshooting when
        // the transcript is truncated or lines are removed.
        self.clamp();
    }

    /// Scrolls by `delta` rendered lines.
    ///
    /// Positive `delta` moves toward older content (up); negative `delta`
    /// moves toward newer content (down).  The result is always clamped to
    /// `[0, max_offset()]`.
    pub(super) fn scroll_by(&mut self, delta: i32) {
        if delta > 0 {
            self.offset_from_bottom = self
                .offset_from_bottom
                .saturating_add(delta as usize)
                .min(self.max_offset());
        } else if delta < 0 {
            let step = (-delta) as usize;
            self.offset_from_bottom = self.offset_from_bottom.saturating_sub(step);
        }
    }

    /// Jumps to the oldest available transcript content (maximum scroll-up).
    pub(super) fn scroll_to_top(&mut self) {
        self.offset_from_bottom = self.max_offset();
    }

    /// Returns to follow-tail (live) mode.
    pub(super) fn scroll_to_bottom(&mut self) {
        self.offset_from_bottom = 0;
    }

    /// Returns the index of the first rendered line that should appear at the
    /// top of the visible viewport.
    ///
    /// Callers use this to slice a `Vec<MessageLineView>` before passing it to
    /// the renderer.  Returns `None` when the transcript is empty or the
    /// viewport has no height.
    ///
    /// Used by the transcript windowing renderer added in a later task.
    // Called by the windowed render helper added in a later task.
    #[allow(dead_code)]
    #[must_use]
    pub(super) fn first_visible_line(&self) -> Option<usize> {
        if self.last_total_lines == 0 || self.last_visible_lines == 0 {
            return None;
        }
        // The last visible line is always `last_total_lines - 1 - offset`.
        // The first visible line sits `last_visible_lines - 1` rows above that.
        let clamped = self.offset_from_bottom.min(self.max_offset());
        Some(
            self.last_total_lines
                .saturating_sub(self.last_visible_lines)
                .saturating_sub(clamped),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── construction ─────────────────────────────────────────────────────────

    #[test]
    fn new_state_is_following_tail() {
        let state = TranscriptScrollState::new();
        assert_eq!(state.offset_from_bottom, 0);
        assert!(state.is_following_tail());
    }

    #[test]
    fn default_matches_new() {
        assert_eq!(TranscriptScrollState::default(), TranscriptScrollState::new());
    }

    // ── max_offset ───────────────────────────────────────────────────────────

    #[test]
    fn max_offset_is_zero_when_content_fits_in_viewport() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(30, 20); // viewport larger than content
        // Cannot scroll: all 20 lines fit in 30 visible rows.
        assert_eq!(state.max_offset(), 0);
    }

    #[test]
    fn max_offset_equals_overflow_lines() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100); // 100 lines, 20 visible
        assert_eq!(state.max_offset(), 80);
    }

    // ── on_resize ────────────────────────────────────────────────────────────

    #[test]
    fn on_resize_updates_cached_dimensions() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(15, 50);
        assert_eq!(state.last_visible_lines, 15);
        assert_eq!(state.last_total_lines, 50);
    }

    #[test]
    fn on_resize_clamps_offset_when_viewport_grows() {
        let mut state = TranscriptScrollState::new();
        // 100 lines, 20 visible → max_offset = 80; scroll up to 50.
        state.on_resize(20, 100);
        state.scroll_by(50);
        assert_eq!(state.offset_from_bottom, 50);

        // Viewport grows to 90 → max_offset becomes 10; offset clamped.
        state.on_resize(90, 100);
        assert_eq!(state.offset_from_bottom, 10);
    }

    #[test]
    fn on_resize_in_follow_tail_mode_stays_following_tail() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 50);
        assert!(state.is_following_tail());

        state.on_resize(25, 60);
        assert!(state.is_following_tail());
    }

    // ── on_messages_changed ──────────────────────────────────────────────────

    #[test]
    fn on_messages_changed_stays_at_tail_when_following() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 50);

        // New messages arrive while following tail.
        state.on_messages_changed(60);
        assert!(state.is_following_tail(), "should stay in follow-tail mode");
        assert_eq!(state.last_total_lines, 60);
    }

    #[test]
    fn on_messages_changed_preserves_viewport_when_scrolled_up() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100);
        state.scroll_by(30); // scrolled up 30 lines from bottom

        // 10 new messages arrive (total grows from 100 → 110).
        state.on_messages_changed(110);

        // Offset is preserved: still 30 lines above the bottom.
        assert_eq!(state.offset_from_bottom, 30);
        assert_eq!(state.last_total_lines, 110);
    }

    #[test]
    fn on_messages_changed_clamps_when_transcript_shrinks() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100);
        state.scroll_by(70); // offset = 70, max_offset = 80 → ok

        // Transcript truncated to 30 lines → max_offset = 10.
        state.on_messages_changed(30);
        assert_eq!(state.offset_from_bottom, 10);
    }

    #[test]
    fn on_messages_changed_keeps_tail_when_content_shrinks_below_viewport() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100);
        // Not scrolled up.
        state.on_messages_changed(5); // content now fits in viewport
        assert!(state.is_following_tail());
        assert_eq!(state.max_offset(), 0);
    }

    // ── scroll_by ────────────────────────────────────────────────────────────

    #[test]
    fn scroll_by_positive_scrolls_toward_older_content() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100); // max_offset = 80
        state.scroll_by(15);
        assert_eq!(state.offset_from_bottom, 15);
        assert!(!state.is_following_tail());
    }

    #[test]
    fn scroll_by_clamps_at_max_offset() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100); // max_offset = 80
        state.scroll_by(200); // would overshoot
        assert_eq!(state.offset_from_bottom, 80);
    }

    #[test]
    fn scroll_by_negative_scrolls_toward_newer_content() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100);
        state.scroll_by(40);
        state.scroll_by(-15);
        assert_eq!(state.offset_from_bottom, 25);
    }

    #[test]
    fn scroll_by_negative_clamps_at_zero() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100);
        state.scroll_by(10);
        state.scroll_by(-200); // would underflow
        assert_eq!(state.offset_from_bottom, 0);
        assert!(state.is_following_tail());
    }

    #[test]
    fn scroll_by_zero_is_a_no_op() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100);
        state.scroll_by(20);
        let before = state.clone();
        state.scroll_by(0);
        assert_eq!(state, before);
    }

    // ── scroll_to_top / scroll_to_bottom ─────────────────────────────────────

    #[test]
    fn scroll_to_top_jumps_to_oldest_available_content() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100); // max_offset = 80
        state.scroll_to_top();
        assert_eq!(state.offset_from_bottom, 80);
    }

    #[test]
    fn scroll_to_top_is_no_op_when_content_fits_in_viewport() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(30, 10); // content fits; max_offset = 0
        state.scroll_to_top();
        assert_eq!(state.offset_from_bottom, 0);
        assert!(state.is_following_tail());
    }

    #[test]
    fn scroll_to_bottom_restores_follow_tail_mode() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100);
        state.scroll_by(50);
        assert!(!state.is_following_tail());

        state.scroll_to_bottom();
        assert_eq!(state.offset_from_bottom, 0);
        assert!(state.is_following_tail());
    }

    // ── first_visible_line ───────────────────────────────────────────────────

    #[test]
    fn first_visible_line_at_tail_is_last_page_start() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100);
        // At tail (offset=0), first visible line = 100 - 20 - 0 = 80.
        assert_eq!(state.first_visible_line(), Some(80));
    }

    #[test]
    fn first_visible_line_when_scrolled_up() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100);
        state.scroll_by(10);
        // first = 100 - 20 - 10 = 70.
        assert_eq!(state.first_visible_line(), Some(70));
    }

    #[test]
    fn first_visible_line_is_zero_when_at_top() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 100);
        state.scroll_to_top();
        // offset = 80; first = 100 - 20 - 80 = 0.
        assert_eq!(state.first_visible_line(), Some(0));
    }

    #[test]
    fn first_visible_line_is_zero_when_content_fits_in_viewport() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(30, 10); // all 10 lines fit; max_offset = 0
        // first = 10 - 30 (saturates to 0) - 0 = 0.
        assert_eq!(state.first_visible_line(), Some(0));
    }

    #[test]
    fn first_visible_line_returns_none_for_empty_transcript() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 0); // no messages
        assert_eq!(state.first_visible_line(), None);
    }

    #[test]
    fn first_visible_line_returns_none_when_viewport_is_zero() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(0, 100); // degenerate terminal height
        assert_eq!(state.first_visible_line(), None);
    }

    // ── combined scenario: resize while scrolled up ───────────────────────────

    #[test]
    fn viewport_preserved_across_resize_while_scrolled_up() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 200); // 200 lines, 20 visible, max_offset=180
        state.scroll_by(50);
        assert_eq!(state.first_visible_line(), Some(130)); // 200 - 20 - 50

        // Terminal shrinks to 15 rows; max_offset grows to 185.
        state.on_resize(15, 200);
        // offset stays at 50 (still valid under new max=185).
        assert_eq!(state.offset_from_bottom, 50);
        assert_eq!(state.first_visible_line(), Some(135)); // 200 - 15 - 50
    }

    // ── combined scenario: new messages while scrolled up ────────────────────

    #[test]
    fn new_messages_do_not_move_viewport_when_scrolled_up() {
        let mut state = TranscriptScrollState::new();
        state.on_resize(20, 50);
        state.scroll_by(20); // read position: 20 lines from bottom

        // 10 new messages arrive.
        state.on_messages_changed(60);

        // User still sees 20 lines above the current tail.
        assert_eq!(state.offset_from_bottom, 20);
        assert_eq!(state.first_visible_line(), Some(20)); // 60 - 20 - 20
    }
}
