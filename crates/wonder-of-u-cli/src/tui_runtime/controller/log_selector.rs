use super::TuiController;
use super::*;

impl TuiController<'_> {
    pub(in crate::tui_runtime) fn open_log_selector(&mut self) {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            self.status_note = Some("no storage directory available".into());
            self.needs_render = true;
            return;
        };

        let store = TranscriptStore::new(storage_dir);
        let sessions = match store.list_metadata() {
            Ok(sessions) => sessions,
            Err(e) => {
                self.status_note = Some(format!("failed to list sessions: {e}"));
                self.needs_render = true;
                return;
            }
        };

        if sessions.is_empty() {
            self.status_note = Some("no past sessions found".into());
            self.needs_render = true;
            return;
        }

        let entries: Vec<LogSelectorEntry> = sessions
            .into_iter()
            .map(|m| LogSelectorEntry {
                session_id: m.session_id,
                title: m.title,
                message_count: m.message_count,
                updated_at: m.updated_at,
                tags: m.tags,
            })
            .collect();

        if entries.is_empty() {
            self.status_note = Some("no past sessions found".into());
            self.needs_render = true;
            return;
        }

        self.clear_picker_overlays();
        self.pending_log_selector = Some(LogSelectorState {
            entries,
            selected_index: 0,
        });
        self.status_note = Some("select a session to resume".into());
        self.needs_render = true;
    }

    pub(in crate::tui_runtime) fn handle_log_selector_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        let Some(state) = &mut self.pending_log_selector else {
            return Ok(());
        };
        let max = state.entries.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if state.selected_index > 0 {
                    state.selected_index -= 1;
                }
                self.needs_render = true;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.selected_index < max {
                    state.selected_index += 1;
                }
                self.needs_render = true;
            }
            KeyCode::Esc => {
                self.pending_log_selector = None;
                self.status_note = Some("session resume cancelled".into());
                self.needs_render = true;
            }
            KeyCode::Tab | KeyCode::Enter => {
                self.confirm_log_selector()?;
            }
            _ => {
                if matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline))) {
                    self.confirm_log_selector()?;
                } else {
                    self.status_note =
                        Some("↑↓/j/k navigate  Enter select  Esc cancel".into());
                    self.needs_render = true;
                }
            }
        }
        Ok(())
    }

    pub(in crate::tui_runtime) fn confirm_log_selector(&mut self) -> Result<()> {
        let Some(state) = self.pending_log_selector.take() else {
            return Ok(());
        };
        let Some(entry) = state.entries.get(state.selected_index) else {
            return Ok(());
        };

        let session_id = entry.session_id;

        if session_id == self.state.session.id {
            self.status_note = Some("already in this session".into());
            self.needs_render = true;
            return Ok(());
        }

        let Some(storage_dir) = self.storage_dir.as_deref() else {
            self.status_note = Some("no storage directory available".into());
            self.needs_render = true;
            return Ok(());
        };

        let store = TranscriptStore::new(storage_dir);
        let restored = store.restore_session(session_id)?;

        self.persistence.transcript_message_count = restored.transcript.messages.len();
        self.persistence.transcript_warning_count = restored.transcript.warnings.len();
        self.persistence.persisted = true;
        self.state = restored.state;
        let _ = std::env::set_current_dir(&self.state.session.cwd);
        self.state.session.entrypoint = Some("tui".into());
        self.state.session.app_version = Some(env!("CARGO_PKG_VERSION").into());
        self.rebuild_ephemeral_state();
        self.notify_transcript_changed();
        self.scroll_state.scroll_to_bottom();
        self.status_note = Some(format!("resumed session {}", &self.state.session.title));
        self.needs_render = true;
        Ok(())
    }
}
