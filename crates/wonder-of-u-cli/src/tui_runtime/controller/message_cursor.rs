use super::TuiController;
use super::*;

impl TuiController<'_> {
    pub(in crate::tui_runtime) fn enter_message_actions(&mut self) {
        let idx = self
            .state
            .messages
            .iter()
            .rposition(|m| matches!(&m.payload, MessagePayload::UserText { .. }));
        self.message_cursor_index = idx;
        self.message_cursor_expanded = false;
        self.status_note = Some(
            "[c] copy · [Enter] expand · [\u{2191}\u{2193}] navigate · [Esc] exit · [s] rewind"
                .into(),
        );
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn handle_message_cursor_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        let max = self.state.messages.len().saturating_sub(1);
        let Some(idx) = self.message_cursor_index else {
            return Ok(());
        };

        match key.code {
            KeyCode::Esc => {
                self.exit_message_actions();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.message_cursor_index = Some(idx.saturating_sub(1));
                self.message_cursor_expanded = false;
                self.needs_render = true;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let new = (idx + 1).min(max);
                self.message_cursor_index = Some(new);
                self.message_cursor_expanded = false;
                self.needs_render = true;
            }
            KeyCode::Enter => {
                if let Some(msg) = self.state.messages.get(idx) {
                    let is_tool_group = matches!(
                        &msg.payload,
                        MessagePayload::AssistantToolUse { .. } | MessagePayload::ToolResult { .. }
                    );
                    if is_tool_group {
                        self.message_cursor_expanded = !self.message_cursor_expanded;
                        self.needs_render = true;
                    } else if matches!(&msg.payload, MessagePayload::UserText { .. }) {
                        // Show message selector for rewinding from this message
                        let user_idx = self
                            .state
                            .messages
                            .iter()
                            .enumerate()
                            .filter(|(_, m)| matches!(&m.payload, MessagePayload::UserText { .. }))
                            .map(|(i, _)| i)
                            .collect::<Vec<_>>();
                        if let Some(pos) = user_idx.iter().position(|&i| i == idx) {
                            let entry_idx = pos.min(user_idx.len().saturating_sub(1));
                            self.confirm_rewind_at_index(user_idx[entry_idx]);
                        }
                    }
                }
            }
            KeyCode::Char('c') => {
                self.copy_message_at_cursor(idx);
            }
            KeyCode::Char('s') => {
                self.show_message_selector();
            }
            _ => {
                if matches!(
                    resolved,
                    Some(ResolvedKey::System(
                        wonder_of_u_tui::SystemAction::Interrupt
                    ))
                ) {
                    self.exit_message_actions();
                }
            }
        }
        Ok(())
    }
    pub(in crate::tui_runtime) fn copy_message_at_cursor(&self, idx: usize) {
        let Some(msg) = self.state.messages.get(idx) else {
            return;
        };
        let text = match &msg.payload {
            MessagePayload::UserText { content } => content.as_str(),
            MessagePayload::AssistantText { content } => content.as_str(),
            MessagePayload::ToolResult { content, .. } => content.as_str(),
            MessagePayload::AssistantThinking { content, .. } => content.as_str(),
            MessagePayload::System { content } => content.as_str(),
            _ => "",
        };
        if !text.is_empty() {
            let _ = crate::commands::copy::write_to_clipboard(text);
        }
    }
    pub(in crate::tui_runtime) fn confirm_rewind_at_index(&mut self, msg_index: usize) {
        let before = self.state.messages.len();
        if msg_index < before {
            self.state.messages.truncate(msg_index);
            let removed = before.saturating_sub(self.state.messages.len());
            self.status_note = Some(format!("rewound {removed} messages"));
            if let Some(storage_dir) = self.storage_dir.as_deref() {
                let store = TranscriptStore::new(storage_dir);
                let transcript_path = store.paths().transcript_path(self.state.session.id);
                if let Ok(file) = std::fs::File::create(&transcript_path) {
                    let mut writer = std::io::BufWriter::new(file);
                    for msg in &self.state.messages {
                        let _ = serde_json::to_writer(&mut writer, msg);
                        let _ = std::io::Write::write_all(&mut writer, b"\n");
                    }
                    let _ = std::io::Write::flush(&mut writer);
                    let _ = writer.get_ref().sync_data();
                }
                if let Ok(mut metadata) = store.read_metadata(self.state.session.id) {
                    metadata.message_count = self.state.messages.len();
                    metadata.updated_at = time::OffsetDateTime::now_utc();
                    let _ = store.write_metadata(&metadata);
                }
                let _ = std::fs::remove_file(store.paths().snapshot_path(self.state.session.id));
                let _ = self.persist_state_snapshot();
            }
        }
        self.message_cursor_index = None;
        self.message_cursor_expanded = false;
        self.status_note = None;
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn exit_message_actions(&mut self) {
        self.message_cursor_index = None;
        self.message_cursor_expanded = false;
        self.status_note = None;
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn show_message_selector(&mut self) {
        let user_entries: Vec<(usize, String)> = self
            .state
            .messages
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                if let MessagePayload::UserText { content } = &m.payload {
                    let preview = truncate_chars(content, 80);
                    Some((i, preview))
                } else {
                    None
                }
            })
            .collect();

        if user_entries.is_empty() {
            self.status_note = Some("no user messages to rewind to".into());
            return;
        }

        self.message_selector_user_indices = user_entries.iter().map(|(i, _)| *i).collect();
        let entries = user_entries
            .into_iter()
            .enumerate()
            .map(|(idx, (_mi, preview))| MessageSelectorEntry {
                preview,
                message_index: idx,
            })
            .collect();

        self.pending_message_selector = Some(MessageSelectorState {
            selected_index: 0,
            entries,
        });
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn handle_message_selector_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        let Some(state) = &mut self.pending_message_selector else {
            return Ok(());
        };
        let max = state.entries.len().saturating_sub(1);
        match key.code {
            KeyCode::Up => {
                state.selected_index = state.selected_index.saturating_sub(1);
                self.needs_render = true;
            }
            KeyCode::Down => {
                if state.selected_index < max {
                    state.selected_index = state.selected_index.saturating_add(1);
                }
                self.needs_render = true;
            }
            KeyCode::Esc => {
                self.pending_message_selector = None;
                self.message_selector_user_indices.clear();
                self.status_note = None;
                self.needs_render = true;
            }
            KeyCode::Tab | KeyCode::Enter => {
                self.confirm_message_selector()?;
            }
            _ => {
                if matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline))) {
                    self.confirm_message_selector()?;
                } else {
                    self.status_note =
                        Some("\u{2191}\u{2193} navigate  Tab/Enter select  Esc cancel".into());
                    self.needs_render = true;
                }
            }
        }
        Ok(())
    }
    pub(in crate::tui_runtime) fn confirm_message_selector(&mut self) -> Result<()> {
        let Some(state) = self.pending_message_selector.take() else {
            return Ok(());
        };
        let Some(entry) = state.entries.get(state.selected_index) else {
            self.message_selector_user_indices.clear();
            return Ok(());
        };
        let Some(&msg_index) = self.message_selector_user_indices.get(entry.message_index) else {
            self.message_selector_user_indices.clear();
            return Ok(());
        };
        self.message_selector_user_indices.clear();
        let cutoff = msg_index;
        let before = self.state.messages.len();
        self.state.messages.truncate(cutoff);
        let removed = before.saturating_sub(self.state.messages.len());
        self.status_note = Some(format!("rewound {removed} messages"));
        if let Some(storage_dir) = self.storage_dir.as_deref() {
            let store = TranscriptStore::new(storage_dir);
            let _id = self.state.session.id;
            // Persist the truncated transcript.
            let transcript_path = store.paths().transcript_path(self.state.session.id);
            let file = std::fs::File::create(&transcript_path)?;
            let mut writer = std::io::BufWriter::new(file);
            for msg in &self.state.messages {
                serde_json::to_writer(&mut writer, msg)?;
                std::io::Write::write_all(&mut writer, b"\n")?;
            }
            std::io::Write::flush(&mut writer)?;
            writer.get_ref().sync_data()?;
            // Update metadata.
            if let Ok(mut metadata) = store.read_metadata(self.state.session.id) {
                metadata.message_count = self.state.messages.len();
                metadata.updated_at = time::OffsetDateTime::now_utc();
                let _ = store.write_metadata(&metadata);
            }
            // Invalidate snapshot so it gets rebuilt.
            let _ = std::fs::remove_file(store.paths().snapshot_path(self.state.session.id));
            // Re-persist the state snapshot.
            let _ = self.persist_state_snapshot();
        }
        self.exit_message_actions();
        self.needs_render = true;
        Ok(())
    }
}
