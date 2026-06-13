use super::TuiController;
use super::*;

impl TuiController<'_> {
    /// Rebuild the slash-command suggestion overlay based on the current prompt buffer.
    pub(in crate::tui_runtime) fn update_slash_suggestions(&mut self) {
        let text = self.prompt.text();
        if let Some(query) = text.strip_prefix('/') {
            let state = self
                .active_suggestions
                .get_or_insert_with(|| PromptSuggestionState::new(self.slash_suggestions.clone()));
            state.set_filter(query);
            if state.filtered().is_empty() {
                self.active_suggestions = None;
            }
        } else {
            self.active_suggestions = None;
        }
    }

    /// Scan the prompt buffer for an active `@` mention token at the cursor and
    /// update the file-mention suggestion overlay.
    pub(in crate::tui_runtime) fn update_file_mentions(&mut self) {
        let text = self.prompt.text();
        let cursor = self.prompt.cursor();
        let cursor_byte = text.chars().take(cursor).map(char::len_utf8).sum::<usize>();

        let at_pos = text[..cursor_byte].rfind('@');
        let has_whitespace_after_at = at_pos.is_some_and(|pos| {
            text[pos + '@'.len_utf8()..cursor_byte].contains(char::is_whitespace)
        });

        if let Some(at_pos) = at_pos
            && !has_whitespace_after_at
        {
            let query = &text[at_pos + '@'.len_utf8()..cursor_byte];
            if self.file_mention_cache.is_none() {
                let files = wonder_of_u_tools::list_project_files(&self.state.session.cwd, 2000);
                self.file_mention_cache = Some(
                    files
                        .into_iter()
                        .map(|path| {
                            let display = path.display().to_string();
                            PromptSuggestion::new(display.clone(), display.clone(), display)
                        })
                        .collect(),
                );
            }
            let state = self.file_mentions.get_or_insert_with(|| {
                PromptSuggestionState::new(self.file_mention_cache.clone().unwrap_or_default())
            });
            state.set_filter(query);
            if state.filtered().is_empty() {
                self.file_mentions = None;
            }
        } else {
            self.file_mentions = None;
        }
    }

    /// Move the selection cursor in the file mention list by `delta` (+1 down, -1 up).
    pub(in crate::tui_runtime) fn navigate_file_mentions(&mut self, delta: i32) {
        let Some(state) = self.file_mentions.as_mut() else {
            return;
        };
        let count = state.filtered().len();
        if count == 0 {
            return;
        }
        let current = state.selected_index as i32;
        let next = (current + delta).rem_euclid(count as i32) as usize;
        state.selected_index = next;
        self.needs_render = true;
    }

    /// Accept the currently selected file mention: delete the `@query` token
    /// at the cursor and insert `@<replacement>` in its place.
    pub(in crate::tui_runtime) fn accept_file_mention(&mut self) {
        let replacement = self
            .file_mentions
            .as_ref()
            .and_then(|s| s.selected())
            .map(|s| s.replacement.clone());
        let Some(replacement) = replacement else {
            return;
        };

        // Find the @ token range before the cursor
        let text = self.prompt.text();
        let cursor = self.prompt.cursor();
        let cursor_byte = text.chars().take(cursor).map(char::len_utf8).sum::<usize>();
        if let Some(at_pos) = text[..cursor_byte].rfind('@') {
            // Delete from @ to cursor, then insert replacement with '@' prefix
            let at_char_index = text[..at_pos].chars().count();
            self.prompt.delete_range(at_char_index, cursor);
            self.prompt
                .insert_text_at(at_char_index, &format!("@{replacement}"));
            self.prompt
                .set_cursor(at_char_index + 1 + replacement.len());
        }
        self.file_mentions = None;
        self.needs_render = true;
    }
    /// Move the selection cursor in the active suggestion list by `delta` (+1 down, -1 up).
    pub(in crate::tui_runtime) fn navigate_slash_suggestions(&mut self, delta: i32) {
        let Some(state) = self.active_suggestions.as_mut() else {
            return;
        };
        let count = state.filtered().len();
        if count == 0 {
            return;
        }
        let current = state.selected_index as i32;
        let next = (current + delta).rem_euclid(count as i32) as usize;
        state.selected_index = next;
        self.needs_render = true;
    }
    /// Accept the currently selected suggestion: replace the prompt buffer with its replacement text.
    pub(in crate::tui_runtime) fn accept_slash_suggestion(&mut self) {
        let replacement = self
            .active_suggestions
            .as_ref()
            .and_then(|s| s.selected())
            .map(|s| s.replacement.clone());
        if let Some(text) = replacement {
            self.prompt = TextBuffer::from_text(&text, true);
            self.active_suggestions = None;
            self.needs_render = true;
        }
    }
    /// Step backward in prompt history (Up arrow). Saves the current prompt on
    /// first press so it can be restored if the user presses Down past index 0.
    pub(in crate::tui_runtime) fn recall_history_prev(&mut self) -> Result<()> {
        let entries = prompt_history_entries(&self.state.messages);
        if entries.is_empty() {
            return Ok(());
        }
        let next_index = match self.history_recall_index {
            None => {
                self.history_recall_saved = self.prompt.text();
                0
            }
            Some(i) => (i + 1).min(entries.len().saturating_sub(1)),
        };
        if let Some(entry) = entries.get(next_index) {
            self.history_recall_index = Some(next_index);
            self.prompt = TextBuffer::from_text(entry, true);
            self.turn_state = TurnState::EditingInput;
            self.state.input_mode = InputMode::Prompt;
            self.needs_render = true;
        }
        Ok(())
    }
    /// Step forward in prompt history (Down arrow). Restores the saved prompt
    /// when stepping past the most recent entry.
    pub(in crate::tui_runtime) fn recall_history_next(&mut self) -> Result<()> {
        let Some(index) = self.history_recall_index else {
            return Ok(());
        };
        if index == 0 {
            // Past the most recent entry — restore saved prompt.
            let saved = std::mem::take(&mut self.history_recall_saved);
            self.history_recall_index = None;
            self.prompt = TextBuffer::from_text(&saved, true);
        } else {
            let entries = prompt_history_entries(&self.state.messages);
            let next_index = index - 1;
            if let Some(entry) = entries.get(next_index) {
                self.history_recall_index = Some(next_index);
                self.prompt = TextBuffer::from_text(entry, true);
            } else {
                self.history_recall_index = None;
                let saved = std::mem::take(&mut self.history_recall_saved);
                self.prompt = TextBuffer::from_text(&saved, true);
            }
        }
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn rebuild_ephemeral_state(&mut self) {
        self.history_search = None;
        self.global_search_open = false;
        self.global_search_query.clear();
        self.global_search_results.clear();
        self.global_search_selected = 0;
        self.global_search_cursor = 0;
        self.global_search_dirty_since = None;
        self.dialog = None;
        self.clear_picker_overlays();
        self.pending_external_editor = None;
        self.pending_provider_form = None;
        self.pending_copilot_oauth = None;
        match self.state.input_mode {
            InputMode::Prompt => {
                self.turn_state = self.turn_state_for_prompt();
                self.status_note = None;
                if let Some(restored) = restored_prompt_ui_state(&self.state.messages) {
                    match restored {
                        RestoredPromptUiState::PermissionPicker(picker) => {
                            self.open_permission_picker(picker);
                        }
                        RestoredPromptUiState::MemoryPicker(picker) => {
                            self.open_memory_picker(picker);
                        }
                        RestoredPromptUiState::TagRemoval(pending) => {
                            self.open_tag_removal_confirmation(pending);
                        }
                        RestoredPromptUiState::ThemePicker(picker) => {
                            self.open_theme_picker(picker);
                        }
                        RestoredPromptUiState::ModelPicker(picker) => {
                            self.open_model_picker(picker);
                        }
                        RestoredPromptUiState::Notice {
                            dialog,
                            status_note,
                        } => {
                            self.dialog = Some(dialog);
                            self.status_note = Some(status_note);
                            self.needs_render = true;
                        }
                        RestoredPromptUiState::Status(status_note) => {
                            self.status_note = Some(status_note);
                        }
                    }
                }
            }
            InputMode::Bash => {
                self.turn_state = TurnState::ToolExecuting;
                self.status_note = Some("bash mode restored from snapshot".into());
            }
            InputMode::PermissionPending => {
                if let Some((dialog, note)) = restored_permission_dialog(
                    &self.state.messages,
                    self.state.pending_tool_approval.as_ref(),
                ) {
                    self.dialog = Some(dialog);
                    self.turn_state = TurnState::ToolPermissionPending;
                    self.status_note = Some(note);
                } else {
                    self.state.input_mode = InputMode::Prompt;
                    self.dialog = None;
                    self.turn_state = if self.state.messages.is_empty() {
                        TurnState::Idle
                    } else {
                        TurnState::Completed
                    };
                    self.status_note = Some("stale permission state cleared".into());
                }
            }
            InputMode::TaskNotification => {
                if let Some(task) = latest_terminal_task(&self.state.background_tasks) {
                    self.dialog = Some(DialogView::notice(
                        "Task update",
                        task_notification_lines(task),
                    ));
                    self.turn_state = TurnState::Completed;
                    self.status_note = Some(format!(
                        "task {}: {}",
                        task_status_label(task.status),
                        task.description
                    ));
                } else {
                    self.state.input_mode = InputMode::Prompt;
                    self.dialog = None;
                    self.turn_state = if self.state.messages.is_empty() {
                        TurnState::Idle
                    } else {
                        TurnState::Completed
                    };
                    self.status_note = Some("stale task notification cleared".into());
                }
            }
        }
    }
}
