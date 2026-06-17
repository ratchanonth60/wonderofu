use super::TuiController;
use super::*;

impl TuiController<'_> {
    pub(in crate::tui_runtime) fn open_export_dialog(&mut self) {
        self.pending_export_dialog = Some(ExportDialogState::default());
        self.needs_render = true;
    }

    pub(in crate::tui_runtime) fn handle_export_dialog_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        let Some(state) = &self.pending_export_dialog else {
            return Ok(());
        };
        match state.mode {
            ExportDialogMode::PickOption => self.handle_export_pick_option_key(key, resolved),
            ExportDialogMode::EnterFilename => self.handle_export_enter_filename_key(key, resolved),
        }
    }

    fn handle_export_pick_option_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Up => {
                if let Some(s) = &mut self.pending_export_dialog {
                    if s.selected_index > 0 {
                        s.selected_index -= 1;
                    }
                }
                self.needs_render = true;
                Ok(())
            }
            KeyCode::Down => {
                if let Some(s) = &mut self.pending_export_dialog {
                    s.selected_index = (s.selected_index + 1).min(1);
                }
                self.needs_render = true;
                Ok(())
            }
            KeyCode::Tab => self.confirm_export_pick_option(),
            KeyCode::Esc => {
                self.pending_export_dialog = None;
                self.needs_render = true;
                Ok(())
            }
            _ => match resolved {
                Some(ResolvedKey::Edit(EditAction::InsertNewline)) => {
                    self.confirm_export_pick_option()
                }
                Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                    self.pending_export_dialog = None;
                    self.needs_render = true;
                    Ok(())
                }
                _ => {
                    self.needs_render = true;
                    Ok(())
                }
            },
        }
    }

    fn handle_export_enter_filename_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                if let Some(s) = &mut self.pending_export_dialog {
                    s.mode = ExportDialogMode::PickOption;
                }
                self.needs_render = true;
                Ok(())
            }
            _ => match resolved {
                Some(ResolvedKey::Edit(EditAction::InsertNewline)) => {
                    self.confirm_export_write_file()
                }
                Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                    self.pending_export_dialog = None;
                    self.needs_render = true;
                    Ok(())
                }
                Some(resolved) => {
                    if apply_picker_query_edit(
                        &mut self
                            .pending_export_dialog
                            .as_mut()
                            .expect("checked above")
                            .filename,
                        resolved,
                    ) {
                        self.needs_render = true;
                    }
                    Ok(())
                }
                None => Ok(()),
            },
        }
    }

    fn confirm_export_pick_option(&mut self) -> Result<()> {
        let index = self
            .pending_export_dialog
            .as_ref()
            .map(|s| s.selected_index)
            .unwrap_or(0);
        if index == 0 {
            // Clipboard
            let text = self.export_session_text();
            self.pending_export_dialog = None;
            match crate::commands::copy::write_to_clipboard(&text) {
                Ok(()) => self.push_notification(
                    "export:clipboard",
                    NotificationSeverity::Info,
                    "Exported",
                    ["Session transcript copied to clipboard."],
                    Some(SHELL_NOTIFICATION_TTL),
                    false,
                ),
                Err(e) => self.push_notification(
                    "export:clipboard-err",
                    NotificationSeverity::Error,
                    "Export failed",
                    [format!("Could not copy to clipboard: {e}")],
                    Some(SHELL_NOTIFICATION_TTL),
                    false,
                ),
            }
        } else {
            // Switch to filename entry mode
            if let Some(s) = &mut self.pending_export_dialog {
                s.mode = ExportDialogMode::EnterFilename;
            }
            self.needs_render = true;
        }
        Ok(())
    }

    fn confirm_export_write_file(&mut self) -> Result<()> {
        let filename = self
            .pending_export_dialog
            .as_ref()
            .map(|s| s.filename.text().trim().to_string())
            .unwrap_or_default();
        self.pending_export_dialog = None;
        if filename.is_empty() {
            self.needs_render = true;
            return Ok(());
        }
        let path = self.state.session.cwd.join(&filename);
        let text = self.export_session_text();
        match std::fs::write(&path, text) {
            Ok(()) => self.push_notification(
                "export:file",
                NotificationSeverity::Info,
                "Exported",
                [format!("Transcript saved to {}", path.display())],
                Some(SHELL_NOTIFICATION_TTL),
                false,
            ),
            Err(e) => self.push_notification(
                "export:file-err",
                NotificationSeverity::Error,
                "Export failed",
                [format!("Could not write {}: {e}", path.display())],
                Some(SHELL_NOTIFICATION_TTL),
                false,
            ),
        }
        Ok(())
    }

    fn export_session_text(&self) -> String {
        let title = if self.state.session.title.is_empty() {
            format!("Session {}", self.state.session.id)
        } else {
            self.state.session.title.clone()
        };
        build_export_text(&title, &self.state.messages)
    }
}

pub(super) fn build_export_text(
    title: &str,
    messages: &[wonder_of_u_core::MessageEnvelope],
) -> String {
    let mut lines = vec![format!("# {title}"), String::new()];
    for msg in messages {
        match &msg.payload {
            wonder_of_u_core::MessagePayload::UserText { content } => {
                lines.push(format!("**User**: {content}"));
                lines.push(String::new());
            }
            wonder_of_u_core::MessagePayload::AssistantText { content } => {
                lines.push(format!("**Assistant**: {content}"));
                lines.push(String::new());
            }
            _ => {}
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use wonder_of_u_core::{MessageEnvelope, MessagePayload, SessionId, ToolUseId};

    use super::build_export_text;

    fn session() -> SessionId {
        SessionId::new()
    }

    fn user_msg(content: &str) -> MessageEnvelope {
        MessageEnvelope::new(
            session(),
            MessagePayload::UserText {
                content: content.into(),
            },
        )
    }

    fn assistant_msg(content: &str) -> MessageEnvelope {
        MessageEnvelope::new(
            session(),
            MessagePayload::AssistantText {
                content: content.into(),
            },
        )
    }

    #[test]
    fn export_text_has_title_header() {
        let text = build_export_text("My Session", &[]);
        assert!(text.starts_with("# My Session\n"));
    }

    #[test]
    fn export_text_formats_user_and_assistant() {
        let msgs = vec![user_msg("hello"), assistant_msg("world")];
        let text = build_export_text("T", &msgs);
        assert!(text.contains("**User**: hello"));
        assert!(text.contains("**Assistant**: world"));
    }

    #[test]
    fn export_text_skips_non_text_payloads() {
        let msgs = vec![
            user_msg("question"),
            MessageEnvelope::new(
                session(),
                MessagePayload::ToolResult {
                    tool: "bash".into(),
                    use_id: ToolUseId::new(),
                    success: true,
                    content: String::new(),
                },
            ),
            assistant_msg("answer"),
        ];
        let text = build_export_text("T", &msgs);
        assert!(text.contains("**User**: question"));
        assert!(text.contains("**Assistant**: answer"));
    }
}
