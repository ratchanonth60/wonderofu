use super::TuiController;
use super::*;

impl TuiController<'_> {
    /// Returns the transcript messages [`Rect`] derived from the last known
    /// terminal dimensions.
    ///
    /// Returns the transcript messages rect for layout and hit-testing.
    ///
    /// Used by tests to verify layout geometry.
    pub(in crate::tui_runtime) fn transcript_messages_rect(&self) -> Rect {
        let (width, height) = self.last_terminal_size;
        if width == 0 || height == 0 {
            return Rect::new(0, 0, 0, 0);
        }
        // Use the same helper as on_terminal_resize so the hit-test rect for
        // mouse wheel events is always consistent with the rendered layout.
        let uncapped = controller_prompt_height(&self.prompt.text());
        let cap = (height / 3).max(6);
        let warning_height = u16::from(context_warning_visible(
            self.estimated_context_tokens(),
            self.state.context_window_size,
        ));
        let prompt_height = uncapped
            .saturating_add(warning_height)
            .min(cap.saturating_add(warning_height));
        ShellLayout::split(
            Rect::new(
                0,
                0,
                shell_main_area_width(width, self.sidebar_visible),
                height,
            ),
            prompt_height,
        )
        .messages
    }
    /// Handles a mouse event from the terminal, scrolling the transcript on
    /// vertical wheel events when the pointer is over the transcript area and
    /// no overlay is active.
    ///
    /// `ScrollLeft` and `ScrollRight` events are silently ignored, as are all
    /// button and move events.
    pub(in crate::tui_runtime) fn handle_mouse_event(&mut self, event: UiEvent) {
        let Some(mouse) = event.normalized_mouse() else {
            return;
        };

        // Left-click in the transcript area enters native selection mode so
        // the terminal emulator handles drag-select and clipboard copy.
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && self.active_overlay() == ActiveOverlay::None
        {
            let rect = self.transcript_messages_rect();
            if rect.contains(mouse.column, mouse.row) {
                self.enter_selection_mode();
            }
            return;
        }

        // Map vertical wheel to signed line deltas.
        // `ScrollUp`   → positive → offset_from_bottom grows  → older content.
        // `ScrollDown` → negative → offset_from_bottom shrinks → newer content.
        let delta: i32 = match mouse.kind {
            MouseEventKind::ScrollUp => MOUSE_SCROLL_LINES,
            MouseEventKind::ScrollDown => -(MOUSE_SCROLL_LINES),
            // Horizontal wheel and all button/move events are not handled here.
            _ => return,
        };

        // Do not scroll while any modal overlay (dialog, picker, history
        // search) is shown; those overlays own their own navigation.
        if self.active_overlay() != ActiveOverlay::None {
            return;
        }

        // Apply scroll regardless of pointer position — the transcript is the
        // only scrollable surface so restricting to the transcript rect just
        // confuses users scrolling while the cursor is in the prompt area.
        self.scroll_state.scroll_by(delta);
        self.needs_render = true;
    }
}
