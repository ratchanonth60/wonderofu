fn parse_git_diff_numstat(text: &str) -> Vec<DiffFileEntry> {
    let mut files = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let added_str = parts.next().unwrap_or("");
        let removed_str = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("").to_string();
        if path.is_empty() {
            continue;
        }
        let is_binary = added_str == "-" || removed_str == "-";
        let lines_added = if is_binary {
            0
        } else {
            added_str.parse().unwrap_or(0)
        };
        let lines_removed = if is_binary {
            0
        } else {
            removed_str.parse().unwrap_or(0)
        };
        files.push(DiffFileEntry {
            path,
            lines_added,
            lines_removed,
            is_binary,
        });
    }
    files
}

use super::TuiController;
use super::*;

impl TuiController<'_> {
    pub(in crate::tui_runtime) async fn handle_dialog_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        if self.diff_dialog.is_some() {
            return self.handle_diff_dialog_key(key, resolved);
        }
        if matches!(
            resolved,
            Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Redraw))
        ) {
            self.needs_render = true;
            return Ok(());
        }
        if self.pending_permission_picker.is_some() {
            return self.handle_permission_picker_key(key, resolved);
        }
        if self.pending_memory_picker.is_some() {
            return self.handle_memory_picker_key(key, resolved);
        }
        if self.pending_message_selector.is_some() {
            return self.handle_message_selector_key(key, resolved);
        }
        if self.pending_tag_removal.is_some() {
            return self.handle_tag_removal_key(key, resolved);
        }
        if self.pending_theme_picker.is_some() {
            return self.handle_theme_picker_key(key, resolved);
        }
        if self.pending_model_picker.is_some() {
            return self.handle_model_picker_key(key, resolved);
        }
        if self.pending_setup_overlay.is_some() {
            return self.handle_setup_overlay_key(key, resolved).await;
        }
        if self.pending_provider_form.is_some() {
            return self.handle_provider_form_key(key, resolved);
        }
        if self.pending_copilot_oauth.is_some() {
            return self.handle_copilot_oauth_dialog_key(key, resolved);
        }
        match self.dialog.as_ref().map(DialogView::kind) {
            Some(wonder_of_u_tui::DialogKind::Permission) => {
                self.handle_permission_dialog_key(key, resolved).await
            }
            Some(wonder_of_u_tui::DialogKind::Interaction) => {
                self.handle_interaction_dialog_key(key, resolved).await
            }
            Some(wonder_of_u_tui::DialogKind::Confirm)
                if matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline))) =>
            {
                let is_confirm = self.dialog.as_ref().is_none_or(|d| d.selected_action == 0);
                if is_confirm {
                    self.dialog = None;
                    self.turn_state = TurnState::Interrupted;
                    self.exit_requested = true;
                } else {
                    self.dismiss_dialog();
                }
                self.needs_render = true;
                Ok(())
            }
            Some(wonder_of_u_tui::DialogKind::Confirm) => match key.code {
                KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                    if let Some(dialog) = &mut self.dialog {
                        dialog.selected_action = dialog.selected_action.saturating_add(1);
                        if dialog.selected_action >= dialog.actions.len() {
                            dialog.selected_action = 0;
                        }
                        self.needs_render = true;
                    }
                    Ok(())
                }
                _ => {
                    self.status_note = Some("exit cancelled".into());
                    self.dismiss_dialog();
                    Ok(())
                }
            },
            Some(wonder_of_u_tui::DialogKind::Notice) => {
                self.dismiss_notice_dialog();
                Ok(())
            }
            None => Ok(()),
        }
    }
    pub(in crate::tui_runtime) fn handle_permission_picker_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Tab => self.complete_permission_picker(),
            KeyCode::Up => {
                self.step_permission_picker(-1);
                Ok(())
            }
            KeyCode::Down => {
                self.step_permission_picker(1);
                Ok(())
            }
            KeyCode::Esc => self.cancel_permission_picker(),
            _ => match resolved {
                Some(ResolvedKey::Edit(EditAction::InsertNewline)) => {
                    self.complete_permission_picker()
                }
                Some(resolved) if self.edit_permission_picker_query(resolved) => Ok(()),
                Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                    self.cancel_permission_picker()
                }
                _ => {
                    self.status_note = Some(picker_status_note("permission mode"));
                    self.needs_render = true;
                    Ok(())
                }
            },
        }
    }
    pub(in crate::tui_runtime) fn handle_model_picker_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Tab => self.complete_model_picker(),
            KeyCode::Up => {
                self.step_model_picker(-1);
                Ok(())
            }
            KeyCode::Down => {
                self.step_model_picker(1);
                Ok(())
            }
            KeyCode::Esc => self.cancel_model_picker(),
            _ => match resolved {
                Some(ResolvedKey::Edit(EditAction::InsertNewline)) => self.complete_model_picker(),
                Some(resolved) if self.edit_model_picker_query(resolved) => Ok(()),
                Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                    self.cancel_model_picker()
                }
                _ => {
                    self.status_note = Some(picker_status_note("model picker"));
                    self.needs_render = true;
                    Ok(())
                }
            },
        }
    }
    pub(in crate::tui_runtime) fn handle_memory_picker_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Tab => self.complete_memory_picker(),
            KeyCode::Up => {
                self.step_memory_picker(-1);
                Ok(())
            }
            KeyCode::Down => {
                self.step_memory_picker(1);
                Ok(())
            }
            KeyCode::Esc => self.cancel_memory_picker(),
            _ => match resolved {
                Some(ResolvedKey::Edit(EditAction::InsertNewline)) => self.complete_memory_picker(),
                Some(resolved) if self.edit_memory_picker_query(resolved) => Ok(()),
                Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                    self.cancel_memory_picker()
                }
                _ => {
                    self.status_note = Some(picker_status_note("memory"));
                    self.needs_render = true;
                    Ok(())
                }
            },
        }
    }
    pub(in crate::tui_runtime) fn handle_theme_picker_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Tab => self.complete_theme_picker(),
            KeyCode::Up => {
                self.step_theme_picker(-1);
                Ok(())
            }
            KeyCode::Down => {
                self.step_theme_picker(1);
                Ok(())
            }
            KeyCode::Esc => self.cancel_theme_picker(),
            _ => match resolved {
                Some(ResolvedKey::Edit(EditAction::InsertNewline)) => self.complete_theme_picker(),
                Some(resolved) if self.edit_theme_picker_query(resolved) => Ok(()),
                Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                    self.cancel_theme_picker()
                }
                _ => {
                    self.status_note = Some(picker_status_note("theme picker"));
                    self.needs_render = true;
                    Ok(())
                }
            },
        }
    }
    pub(in crate::tui_runtime) async fn handle_permission_dialog_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        // Up / Left: move selection to previous action.
        if matches!(key.code, KeyCode::Up | KeyCode::Left) {
            if let Some(dialog) = &mut self.dialog {
                dialog.focus_prev();
                self.needs_render = true;
            }
            return Ok(());
        }

        // Down / Right / Tab: move selection to next action.
        if matches!(key.code, KeyCode::Down | KeyCode::Right | KeyCode::Tab) {
            if let Some(dialog) = &mut self.dialog {
                dialog.focus_next();
                self.needs_render = true;
            }
            return Ok(());
        }

        // Enter / Space: confirm the currently focused action.
        if matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline)))
            || key.code == KeyCode::Char(' ')
        {
            let approved = self.dialog.as_ref().is_none_or(|d| d.selected_is_primary());
            return self.resolve_pending_tool_approval(approved).await;
        }

        // Legacy y/n shortcuts still work.
        if key.code == KeyCode::Esc
            || matches!(
                resolved,
                Some(ResolvedKey::InsertChar('n'))
                    | Some(ResolvedKey::InsertChar('N'))
                    | Some(ResolvedKey::System(
                        wonder_of_u_tui::SystemAction::Interrupt
                    ))
            )
        {
            return self.resolve_pending_tool_approval(false).await;
        }
        if matches!(
            resolved,
            Some(ResolvedKey::InsertChar('y')) | Some(ResolvedKey::InsertChar('Y'))
        ) {
            return self.resolve_pending_tool_approval(true).await;
        }
        self.needs_render = true;
        Ok(())
    }
    /// Key handler for `DialogKind::Interaction` (ask_user with options).
    ///
    /// Up/Down navigate the option list.  Enter confirms the selected option.
    /// When the last action ("Other") is selected, Enter switches to free-text
    /// mode — the dialog body is updated and the prompt box becomes active.
    pub(in crate::tui_runtime) async fn handle_interaction_dialog_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        // Up: move to previous option.
        if matches!(key.code, KeyCode::Up | KeyCode::Left) {
            if let Some(dialog) = &mut self.dialog {
                dialog.focus_prev();
                self.needs_render = true;
            }
            return Ok(());
        }
        // Down / Tab: move to next option.
        if matches!(key.code, KeyCode::Down | KeyCode::Right | KeyCode::Tab) {
            if let Some(dialog) = &mut self.dialog {
                dialog.focus_next();
                self.needs_render = true;
            }
            return Ok(());
        }
        // Enter / Space: confirm selection.
        if matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline)))
            || key.code == KeyCode::Char(' ')
        {
            let selected_label = self
                .dialog
                .as_ref()
                .and_then(|d| d.actions.get(d.selected_action))
                .map(|a| a.label.clone())
                .unwrap_or_default();

            if selected_label == "Other (type your answer)" {
                // Switch to free-text mode: update dialog hint + enable prompt.
                self.interaction_other_mode = true;
                if let Some(dialog) = &mut self.dialog {
                    dialog.actions.clear();
                    dialog.body.retain(|l| !l.is_empty());
                    dialog.body.push(String::new());
                    dialog
                        .body
                        .push("Type your answer in the prompt box below and press Enter ↵".into());
                    dialog.selected_action = 0;
                }
                self.state.input_mode = InputMode::Prompt;
                self.status_note = Some("type answer ↵ to send".into());
                self.needs_render = true;
                return Ok(());
            }

            // Deliver selected option label as the answer.
            let answer = selected_label;
            self.interaction_pending_answer = Some(answer);
            return self.resolve_pending_tool_approval(true).await;
        }
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn handle_diff_dialog_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        let Some(ref mut state) = self.diff_dialog else {
            return Ok(());
        };

        if matches!(
            resolved,
            Some(ResolvedKey::System(
                wonder_of_u_tui::SystemAction::Interrupt
            ))
        ) {
            self.diff_dialog = None;
            self.dialog = None;
            self.status_note = None;
            self.needs_render = true;
            return Ok(());
        }

        if state.detail_mode {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                state.detail_mode = false;
                state.detail_lines.clear();
                self.needs_render = true;
                return Ok(());
            }
            if matches!(key.code, KeyCode::Up | KeyCode::Down) {
                self.needs_render = true;
                return Ok(());
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if state.selected_index > 0 {
                    state.selected_index -= 1;
                } else if !state.files.is_empty() {
                    state.selected_index = state.files.len() - 1;
                }
                self.needs_render = true;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !state.files.is_empty() {
                    state.selected_index = (state.selected_index + 1) % state.files.len();
                }
                self.needs_render = true;
            }
            KeyCode::Enter => {
                self.open_diff_detail();
                self.needs_render = true;
            }
            KeyCode::Esc => {
                self.diff_dialog = None;
                self.dialog = None;
                self.status_note = None;
                self.needs_render = true;
            }
            _ => {
                self.status_note = Some("↑↓/j/k navigate  ↵ view detail  Esc close".into());
                self.needs_render = true;
            }
        }
        Ok(())
    }
    pub(in crate::tui_runtime) fn open_diff_detail(&mut self) {
        let Some(ref state) = self.diff_dialog else {
            return;
        };
        if state.files.is_empty() {
            return;
        }
        let idx = state.selected_index.min(state.files.len() - 1);
        let file = &state.files[idx];
        if file.is_binary {
            let mut new_state = state.clone();
            new_state.detail_mode = true;
            new_state.detail_title = file.path.clone();
            new_state.detail_lines = vec!["Binary file - diff not shown.".into()];
            self.diff_dialog = Some(new_state);
            return;
        }
        let cwd = self.state.session.cwd.clone();
        let path = file.path.clone();
        let output = std::process::Command::new("git")
            .args(["--no-pager", "diff", "--", &path])
            .current_dir(&cwd)
            .output();
        let detail_lines = match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).into_owned();
                if text.is_empty() {
                    vec!["No diff output.".into()]
                } else {
                    text.lines().map(String::from).collect()
                }
            }
            _ => vec!["Failed to get diff for this file.".into()],
        };
        let mut new_state = state.clone();
        new_state.detail_mode = true;
        new_state.detail_title = file.path.clone();
        new_state.detail_lines = detail_lines;
        self.diff_dialog = Some(new_state);
    }
    pub(in crate::tui_runtime) fn build_diff_dialog_view(
        &self,
        state: &DiffDialogState,
    ) -> DialogView {
        if state.detail_mode {
            let mut body = state.detail_lines.clone();
            if body.is_empty() {
                body.push("No diff available.".into());
            }
            let title = format!("Diff: {}", state.detail_title);
            DialogView {
                title,
                body,
                actions: vec![DialogActionView::new("Back", true)],
                selected_action: 0,
            }
        } else {
            let max_path_len = state
                .files
                .iter()
                .map(|f| f.path.len())
                .max()
                .unwrap_or(0)
                .min(50);
            let body: Vec<String> = state
                .files
                .iter()
                .enumerate()
                .map(|(i, file)| {
                    let prefix = if i == state.selected_index {
                        "▶"
                    } else {
                        " "
                    };
                    let stats = if file.is_binary {
                        "Binary".to_string()
                    } else {
                        format!("+{} -{}", file.lines_added, file.lines_removed)
                    };
                    format!(
                        "{prefix} {: <path_width$} {}",
                        file.path,
                        stats,
                        path_width = max_path_len
                    )
                })
                .collect();
            DialogView {
                title: "Git Diff".into(),
                body,
                actions: vec![
                    DialogActionView::new("View Detail", true),
                    DialogActionView::new("Close", false),
                ],
                selected_action: 0,
            }
        }
    }
    pub(in crate::tui_runtime) fn open_diff_dialog(&mut self) -> Result<()> {
        let cwd = self.state.session.cwd.clone();
        let numstat_output = std::process::Command::new("git")
            .args(["--no-pager", "diff", "--numstat", "--find-renames"])
            .current_dir(&cwd)
            .output();
        let output = match numstat_output {
            Ok(out) if out.status.success() => out,
            _ => {
                self.status_note = Some("no git diff available".into());
                return Ok(());
            }
        };
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        if text.trim().is_empty() {
            self.status_note = Some("working tree clean".into());
            return Ok(());
        }
        let files = parse_git_diff_numstat(&text);
        if files.is_empty() {
            self.status_note = Some("no changed files".into());
            return Ok(());
        }
        self.diff_dialog = Some(DiffDialogState {
            files,
            selected_index: 0,
            detail_mode: false,
            detail_title: String::new(),
            detail_lines: Vec::new(),
        });
        self.status_note = None;
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn open_permission_picker(
        &mut self,
        mut picker: PermissionPickerState,
    ) {
        if picker.options.is_empty() {
            self.status_note = Some("no permission modes available".into());
            self.dialog = None;
            self.pending_permission_picker = None;
            self.needs_render = true;
            return;
        }
        if picker.selected_index >= picker.options.len() {
            picker.selected_index = 0;
        }
        self.clear_picker_overlays();
        self.pending_permission_picker = Some(picker);
        self.status_note = None;
        self.refresh_permission_picker_dialog();
    }
    pub(in crate::tui_runtime) fn cycle_prompt_permission_mode_backward(&mut self) -> Result<()> {
        let mode = previous_prompt_permission_mode(self.state.permission_mode);
        let output = format!(
            "permission_mode={}\nstatus=permission mode updated\nplan_mode_active={}",
            permission_mode_output_label(mode),
            matches!(mode, PermissionMode::Plan),
        );
        self.apply_command_output_hints(Some(&output));
        self.status_note = Some(format!(
            "permission mode {}",
            permission_mode_status_label(mode)
        ));
        self.needs_render = true;
        self.persist_state_snapshot()
    }
    pub(in crate::tui_runtime) fn refresh_permission_picker_dialog(&mut self) {
        let Some(picker) = &self.pending_permission_picker else {
            return;
        };
        let filtered = filtered_picker_indices(&picker.query, &picker.options, |option| {
            format!("{} {}", option.label, option.description)
        });
        let mut body = picker_dialog_header(
            "Search",
            &picker.query,
            filtered.len(),
            picker.options.len(),
        );
        if filtered.is_empty() {
            body.push("No matching permission modes.".into());
        } else {
            body.extend(filtered.into_iter().map(|index| {
                let option = &picker.options[index];
                let cursor = if index == picker.selected_index {
                    ">"
                } else {
                    " "
                };
                let current = if option.selected { " [current]" } else { "" };
                format!(
                    "{cursor} {}{} - {}",
                    option.label, current, option.description
                )
            }));
        }
        self.dialog = Some(DialogView {
            title: "Permission mode".into(),
            body,
            actions: vec![
                wonder_of_u_tui::DialogActionView::new("Select", true),
                wonder_of_u_tui::DialogActionView::new("Cancel", false),
            ],
            selected_action: 0,
        });
        self.status_note = Some(picker_status_note("permission mode"));
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn step_permission_picker(&mut self, delta: isize) {
        let Some(picker) = &mut self.pending_permission_picker else {
            return;
        };
        picker.selected_index = step_picker_selection(
            picker.selected_index,
            delta,
            &filtered_picker_indices(&picker.query, &picker.options, |option| {
                format!("{} {}", option.label, option.description)
            }),
        );
        self.refresh_permission_picker_dialog();
    }
    pub(in crate::tui_runtime) fn edit_permission_picker_query(
        &mut self,
        resolved: ResolvedKey,
    ) -> bool {
        let Some(picker) = &mut self.pending_permission_picker else {
            return false;
        };
        if !apply_picker_query_edit(&mut picker.query, resolved) {
            return false;
        }
        sync_picker_selection(
            &mut picker.selected_index,
            &filtered_picker_indices(&picker.query, &picker.options, |option| {
                format!("{} {}", option.label, option.description)
            }),
        );
        self.refresh_permission_picker_dialog();
        true
    }
    pub(in crate::tui_runtime) fn complete_permission_picker(&mut self) -> Result<()> {
        let Some(picker) = self.pending_permission_picker.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        let filtered = filtered_picker_indices(&picker.query, &picker.options, |option| {
            format!("{} {}", option.label, option.description)
        });
        let Some(selection_index) = selected_picker_index(picker.selected_index, &filtered) else {
            self.pending_permission_picker = Some(picker);
            self.refresh_permission_picker_dialog();
            self.status_note = Some("permission mode: no matching option to select".into());
            return Ok(());
        };
        let Some(selection) = picker.options.get(selection_index).cloned() else {
            self.pending_permission_picker = Some(picker);
            self.refresh_permission_picker_dialog();
            self.status_note = Some("permission mode: no matching option to select".into());
            return Ok(());
        };
        self.dialog = None;
        let output = format!(
            "permission_mode={}\nstatus=permission mode updated\nplan_mode_active={}",
            permission_mode_output_label(selection.mode),
            matches!(selection.mode, PermissionMode::Plan),
        );
        self.apply_command_output_hints(Some(&output));
        self.record_command_message(&picker.original_input, Some(&output))?;
        self.status_note = Some(format!(
            "permission mode {}",
            selection.label.to_ascii_lowercase()
        ));
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn cancel_permission_picker(&mut self) -> Result<()> {
        let Some(picker) = self.pending_permission_picker.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        self.dialog = None;
        self.record_command_message(
            &picker.original_input,
            Some("status=permission picker cancelled"),
        )?;
        self.status_note = Some("permission picker cancelled".into());
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn open_memory_picker(&mut self, mut picker: MemoryPickerState) {
        if picker.options.is_empty() {
            self.status_note = Some("no memory files available".into());
            self.dialog = None;
            self.pending_memory_picker = None;
            self.needs_render = true;
            return;
        }
        if picker.selected_index >= picker.options.len() {
            picker.selected_index = 0;
        }
        self.clear_picker_overlays();
        self.pending_memory_picker = Some(picker);
        self.status_note = None;
        self.refresh_memory_picker_dialog();
    }
    pub(in crate::tui_runtime) fn refresh_memory_picker_dialog(&mut self) {
        let Some(picker) = &self.pending_memory_picker else {
            return;
        };
        let filtered = filtered_picker_indices(&picker.query, &picker.options, |option| {
            format!(
                "{} {} {}",
                option.label,
                option.description,
                option.path.display()
            )
        });
        let mut body = picker_dialog_header(
            "Search",
            &picker.query,
            filtered.len(),
            picker.options.len(),
        );
        if filtered.is_empty() {
            body.push("No matching memory files.".into());
        } else {
            body.extend(filtered.into_iter().map(|index| {
                let option = &picker.options[index];
                let cursor = if index == picker.selected_index {
                    ">"
                } else {
                    " "
                };
                let current = if option.selected { " [default]" } else { "" };
                format!(
                    "{cursor} {}{} - {}",
                    option.label, current, option.description
                )
            }));
        }
        self.dialog = Some(DialogView {
            title: "Memory".into(),
            body,
            actions: vec![
                wonder_of_u_tui::DialogActionView::new("Select", true),
                wonder_of_u_tui::DialogActionView::new("Cancel", false),
            ],
            selected_action: 0,
        });
        self.status_note = Some(picker_status_note("memory"));
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn step_memory_picker(&mut self, delta: isize) {
        let Some(picker) = &mut self.pending_memory_picker else {
            return;
        };
        picker.selected_index = step_picker_selection(
            picker.selected_index,
            delta,
            &filtered_picker_indices(&picker.query, &picker.options, |option| {
                format!(
                    "{} {} {}",
                    option.label,
                    option.description,
                    option.path.display()
                )
            }),
        );
        self.refresh_memory_picker_dialog();
    }
    pub(in crate::tui_runtime) fn edit_memory_picker_query(
        &mut self,
        resolved: ResolvedKey,
    ) -> bool {
        let Some(picker) = &mut self.pending_memory_picker else {
            return false;
        };
        if !apply_picker_query_edit(&mut picker.query, resolved) {
            return false;
        }
        sync_picker_selection(
            &mut picker.selected_index,
            &filtered_picker_indices(&picker.query, &picker.options, |option| {
                format!(
                    "{} {} {}",
                    option.label,
                    option.description,
                    option.path.display()
                )
            }),
        );
        self.refresh_memory_picker_dialog();
        true
    }
    pub(in crate::tui_runtime) fn complete_memory_picker(&mut self) -> Result<()> {
        let Some(picker) = self.pending_memory_picker.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        let filtered = filtered_picker_indices(&picker.query, &picker.options, |option| {
            format!(
                "{} {} {}",
                option.label,
                option.description,
                option.path.display()
            )
        });
        let Some(selection_index) = selected_picker_index(picker.selected_index, &filtered) else {
            self.pending_memory_picker = Some(picker);
            self.refresh_memory_picker_dialog();
            self.status_note = Some("memory: no matching option to select".into());
            return Ok(());
        };
        let Some(selection) = picker.options.get(selection_index).cloned() else {
            self.pending_memory_picker = Some(picker);
            self.refresh_memory_picker_dialog();
            self.status_note = Some("memory: no matching option to select".into());
            return Ok(());
        };
        self.dialog = None;
        let output = crate::commands::project::memory_open_output(
            &self.command_context(),
            self.storage_dir.as_deref(),
            selection.target,
        )?;
        self.apply_command_output_hints(Some(&output));
        self.record_command_message(&picker.original_input, Some(&output))?;
        self.status_note = if self.pending_external_editor.is_some() {
            Some("opening file in editor".into())
        } else {
            Some(format!("opened {}", selection.label.to_ascii_lowercase()))
        };
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn cancel_memory_picker(&mut self) -> Result<()> {
        let Some(picker) = self.pending_memory_picker.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        self.dialog = None;
        self.record_command_message(
            &picker.original_input,
            Some("status=memory picker cancelled"),
        )?;
        self.status_note = Some("memory picker cancelled".into());
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn open_tag_removal_confirmation(
        &mut self,
        pending: TagRemovalState,
    ) {
        self.clear_picker_overlays();
        self.pending_tag_removal = Some(pending.clone());
        self.dialog = Some(DialogView::confirm(
            "Remove tag?",
            vec![
                format!("Current tag: #{}", pending.tag),
                "Press Enter to remove it from the current session.".to_string(),
                "Press any other key to keep it.".to_string(),
            ],
        ));
        self.status_note = Some("confirm tag removal".into());
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn handle_tag_removal_key(
        &mut self,
        _key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        if matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline))) {
            return self.complete_tag_removal();
        }
        self.cancel_tag_removal()
    }
    pub(in crate::tui_runtime) fn complete_tag_removal(&mut self) -> Result<()> {
        let Some(pending) = self.pending_tag_removal.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        self.dialog = None;
        self.state.set_session_tags(Vec::new());
        let output = format!("session_tags=\nstatus=removed tag #{}", pending.tag);
        self.record_command_message(&pending.original_input, Some(&output))?;
        self.status_note = Some(format!("removed #{}", pending.tag));
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn cancel_tag_removal(&mut self) -> Result<()> {
        let Some(pending) = self.pending_tag_removal.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        self.dialog = None;
        self.record_command_message(
            &pending.original_input,
            Some(&format!("status=kept tag #{}", pending.tag)),
        )?;
        self.status_note = Some(format!("kept #{}", pending.tag));
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn open_theme_picker(&mut self, mut picker: ThemePickerState) {
        if picker.options.is_empty() {
            self.status_note = Some("no themes available".into());
            self.dialog = None;
            self.pending_theme_picker = None;
            self.needs_render = true;
            return;
        }
        if picker.selected_index >= picker.options.len() {
            picker.selected_index = 0;
        }
        self.clear_picker_overlays();
        self.pending_theme_picker = Some(picker);
        self.status_note = None;
        self.refresh_theme_picker_dialog();
    }
    pub(in crate::tui_runtime) fn refresh_theme_picker_dialog(&mut self) {
        let Some(picker) = &self.pending_theme_picker else {
            return;
        };
        let filtered = filtered_picker_indices(&picker.query, &picker.options, |option| {
            format!("{} {} {}", option.theme, option.label, option.description)
        });
        let mut body = picker_dialog_header(
            "Search",
            &picker.query,
            filtered.len(),
            picker.options.len(),
        );
        if filtered.is_empty() {
            body.push("No matching themes.".into());
        } else {
            body.extend(filtered.into_iter().map(|index| {
                let option = &picker.options[index];
                let cursor = if index == picker.selected_index {
                    ">"
                } else {
                    " "
                };
                let current = if option.selected { " [current]" } else { "" };
                format!(
                    "{cursor} {}{} - {}",
                    option.label, current, option.description
                )
            }));
        }
        self.dialog = Some(DialogView {
            title: "Theme picker".into(),
            body,
            actions: vec![
                wonder_of_u_tui::DialogActionView::new("Select", true),
                wonder_of_u_tui::DialogActionView::new("Cancel", false),
            ],
            selected_action: 0,
        });
        self.status_note = Some(picker_status_note("theme picker"));
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn step_theme_picker(&mut self, delta: isize) {
        let Some(picker) = &mut self.pending_theme_picker else {
            return;
        };
        picker.selected_index = step_picker_selection(
            picker.selected_index,
            delta,
            &filtered_picker_indices(&picker.query, &picker.options, |option| {
                format!("{} {} {}", option.theme, option.label, option.description)
            }),
        );
        self.refresh_theme_picker_dialog();
    }
    pub(in crate::tui_runtime) fn edit_theme_picker_query(
        &mut self,
        resolved: ResolvedKey,
    ) -> bool {
        let Some(picker) = &mut self.pending_theme_picker else {
            return false;
        };
        if !apply_picker_query_edit(&mut picker.query, resolved) {
            return false;
        }
        sync_picker_selection(
            &mut picker.selected_index,
            &filtered_picker_indices(&picker.query, &picker.options, |option| {
                format!("{} {} {}", option.theme, option.label, option.description)
            }),
        );
        self.refresh_theme_picker_dialog();
        true
    }
    pub(in crate::tui_runtime) fn complete_theme_picker(&mut self) -> Result<()> {
        let Some(picker) = self.pending_theme_picker.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        let filtered = filtered_picker_indices(&picker.query, &picker.options, |option| {
            format!("{} {} {}", option.theme, option.label, option.description)
        });
        let Some(selection_index) = selected_picker_index(picker.selected_index, &filtered) else {
            self.pending_theme_picker = Some(picker);
            self.refresh_theme_picker_dialog();
            self.status_note = Some("theme picker: no matching option to select".into());
            return Ok(());
        };
        let Some(selection) = picker.options.get(selection_index).cloned() else {
            self.pending_theme_picker = Some(picker);
            self.refresh_theme_picker_dialog();
            self.status_note = Some("theme picker: no matching option to select".into());
            return Ok(());
        };
        self.dialog = None;
        let output = format!("theme={}\nstatus=theme updated", selection.theme);
        self.apply_command_output_hints(Some(&output));
        self.record_command_message(&picker.original_input, Some(&output))?;
        self.status_note = Some(format!("theme {}", selection.label.to_ascii_lowercase()));
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn cancel_theme_picker(&mut self) -> Result<()> {
        let Some(picker) = self.pending_theme_picker.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        self.dialog = None;
        self.record_command_message(
            &picker.original_input,
            Some("status=theme picker cancelled"),
        )?;
        self.status_note = Some("theme picker cancelled".into());
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn open_model_picker(&mut self, mut picker: ModelPickerState) {
        if picker.options.is_empty() {
            self.status_note = Some("no models available".into());
            self.dialog = None;
            self.pending_model_picker = None;
            self.needs_render = true;
            return;
        }
        if picker.selected_index >= picker.options.len() {
            picker.selected_index = 0;
        }
        self.clear_picker_overlays();
        self.pending_model_picker = Some(picker);
        self.status_note = None;
        self.refresh_model_picker_dialog();
    }
    pub(in crate::tui_runtime) fn refresh_model_picker_dialog(&mut self) {
        let Some(picker) = &self.pending_model_picker else {
            return;
        };
        let filtered =
            filtered_picker_indices(&picker.query, &picker.options, |option| option.search_key());
        let mut body = picker_dialog_header(
            "Search",
            &picker.query,
            filtered.len(),
            picker.options.len(),
        );
        if filtered.is_empty() {
            body.push("No matching models.".into());
        } else {
            body.extend(filtered.into_iter().map(|index| {
                let option = &picker.options[index];
                let cursor = if index == picker.selected_index {
                    ">"
                } else {
                    " "
                };
                let mut badges = vec![option.auth.clone()];
                if option.default {
                    badges.push("default".into());
                }
                if option.selected {
                    badges.push("current".into());
                }
                format!(
                    "{cursor} {} / {} [{}]",
                    option.provider_display,
                    option.model_display,
                    badges.join(", ")
                )
            }));
        }
        self.dialog = Some(DialogView {
            title: "Model picker".into(),
            body,
            actions: vec![
                wonder_of_u_tui::DialogActionView::new("Select", true),
                wonder_of_u_tui::DialogActionView::new("Cancel", false),
            ],
            selected_action: 0,
        });
        self.status_note = Some(picker_status_note("model picker"));
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn step_model_picker(&mut self, delta: isize) {
        let Some(picker) = &mut self.pending_model_picker else {
            return;
        };
        picker.selected_index = step_picker_selection(
            picker.selected_index,
            delta,
            &filtered_picker_indices(&picker.query, &picker.options, |option| option.search_key()),
        );
        self.refresh_model_picker_dialog();
    }
    pub(in crate::tui_runtime) fn edit_model_picker_query(
        &mut self,
        resolved: ResolvedKey,
    ) -> bool {
        let Some(picker) = &mut self.pending_model_picker else {
            return false;
        };
        if !apply_picker_query_edit(&mut picker.query, resolved) {
            return false;
        }
        sync_picker_selection(
            &mut picker.selected_index,
            &filtered_picker_indices(&picker.query, &picker.options, |option| option.search_key()),
        );
        self.refresh_model_picker_dialog();
        true
    }
    pub(in crate::tui_runtime) fn complete_model_picker(&mut self) -> Result<()> {
        let Some(picker) = self.pending_model_picker.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        let filtered =
            filtered_picker_indices(&picker.query, &picker.options, |option| option.search_key());
        let Some(selection_index) = selected_picker_index(picker.selected_index, &filtered) else {
            self.pending_model_picker = Some(picker);
            self.refresh_model_picker_dialog();
            self.status_note = Some("model picker: no matching option to select".into());
            return Ok(());
        };
        let Some(selection) = picker.options.get(selection_index).cloned() else {
            self.pending_model_picker = Some(picker);
            self.refresh_model_picker_dialog();
            self.status_note = Some("model picker: no matching option to select".into());
            return Ok(());
        };
        self.dialog = None;
        let output = crate::commands::auth::set_model_selection(
            self.storage_dir.as_deref(),
            Some(selection.provider.clone()),
            Some(selection.model.clone()),
            None,
        )?;
        self.apply_command_output_hints(Some(&output));
        self.record_command_message(&picker.original_input, Some(&output))?;
        let _ = self.refresh_runtime_state()?;
        self.status_note = Some(format!(
            "selected {} / {}",
            selection.provider_display, selection.model_display
        ));
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn cancel_model_picker(&mut self) -> Result<()> {
        let Some(picker) = self.pending_model_picker.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        self.dialog = None;
        self.record_command_message(
            &picker.original_input,
            Some("status=model picker cancelled"),
        )?;
        self.status_note = Some("model picker cancelled".into());
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn open_setup_overlay(&mut self, mut overlay: SetupOverlayState) {
        if overlay.items.is_empty() {
            self.status_note = Some("setup menu is empty".into());
            self.needs_render = true;
            return;
        }
        overlay.clamp_selection();
        self.clear_picker_overlays();
        self.pending_setup_overlay = Some(overlay);
        self.status_note = Some(picker_status_note("setup"));
        self.dialog = None;
        self.needs_render = true;
    }
    /// Moves the highlighted row by `delta` (+1 down, -1 up).
    pub(in crate::tui_runtime) fn step_setup_overlay(&mut self, delta: isize) {
        let Some(overlay) = &mut self.pending_setup_overlay else {
            return;
        };
        if overlay.items.is_empty() {
            return;
        }
        let count = overlay.items.len();
        let current = overlay.selected_index as isize;
        overlay.selected_index = (current + delta).rem_euclid(count as isize) as usize;
        self.needs_render = true;
    }
    /// Confirms the currently highlighted setup item and dispatches its action.
    pub(in crate::tui_runtime) async fn complete_setup_overlay(&mut self) -> Result<()> {
        let Some(overlay) = self.pending_setup_overlay.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        self.dialog = None;
        let Some(item) = overlay.items.get(overlay.selected_index).cloned() else {
            return Ok(());
        };
        match item.action {
            SetupItemAction::Dispatch(command) => {
                self.execute_slash_command_with(&command).await?;
            }
            SetupItemAction::Placeholder(message) => {
                // Show a notice dialog while the full form is deferred.
                let body: Vec<String> = message.lines().map(str::to_string).collect();
                self.dialog = Some(DialogView::notice(item.label.clone(), body.clone()));
                self.status_note = Some(format!("setup: {}", item.label.to_ascii_lowercase()));
                self.push_notification(
                    format!("setup-placeholder:{}", item.id),
                    NotificationSeverity::Info,
                    item.label,
                    body,
                    Some(SHELL_NOTIFICATION_TTL),
                    false,
                );
                self.needs_render = true;
            }
            SetupItemAction::Deferred(message) => {
                // Show a notice dialog explaining that this cloud/remote feature is
                // intentionally out-of-scope for the local-first TUI.
                let body: Vec<String> = message.lines().map(str::to_string).collect();
                self.dialog = Some(DialogView::notice(item.label.clone(), body.clone()));
                self.status_note = Some(format!(
                    "setup: {} (deferred)",
                    item.label.to_ascii_lowercase()
                ));
                self.push_notification(
                    format!("setup-deferred:{}", item.id),
                    NotificationSeverity::Info,
                    item.label,
                    body,
                    Some(SHELL_NOTIFICATION_TTL),
                    false,
                );
                self.needs_render = true;
            }
            SetupItemAction::ProviderForm(kind) => {
                self.open_provider_form(kind);
            }
            SetupItemAction::CopilotOAuth => {
                self.open_copilot_oauth_flow();
            }
        }
        Ok(())
    }
    /// Cancels the setup overlay, records a status note, and marks this session
    /// so that the autostart logic does not re-open the overlay.
    pub(in crate::tui_runtime) fn cancel_setup_overlay(&mut self) -> Result<()> {
        let Some(overlay) = self.pending_setup_overlay.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        // Prevent maybe_auto_open_setup from re-opening for the rest of this session.
        self.setup_cancelled_this_session = true;
        self.dialog = None;
        self.record_command_message(&overlay.original_input, Some("status=setup cancelled"))?;
        self.status_note = Some("setup cancelled".into());
        self.needs_render = true;
        Ok(())
    }
    /// Handles keyboard input while the setup overlay is active.
    pub(in crate::tui_runtime) async fn handle_setup_overlay_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Tab | KeyCode::Enter => self.complete_setup_overlay().await,
            KeyCode::Up => {
                self.step_setup_overlay(-1);
                Ok(())
            }
            KeyCode::Down => {
                self.step_setup_overlay(1);
                Ok(())
            }
            KeyCode::Esc => self.cancel_setup_overlay(),
            _ => match resolved {
                Some(ResolvedKey::Edit(EditAction::InsertNewline)) => {
                    self.complete_setup_overlay().await
                }
                Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                    self.cancel_setup_overlay()
                }
                _ => {
                    self.status_note = Some(picker_status_note("setup"));
                    self.needs_render = true;
                    Ok(())
                }
            },
        }
    }
    /// Opens `/setup` automatically when the provider is not yet configured,
    /// unless the user already dismissed it during this session.
    ///
    /// Called once at the end of [`TuiController::new`]. After the first
    /// successful provider configuration the method becomes a no-op.
    pub(in crate::tui_runtime) async fn maybe_auto_open_setup(&mut self) -> Result<()> {
        if self.setup_cancelled_this_session {
            return Ok(());
        }
        if self.state.provider_readiness() == ProviderReadiness::Ready {
            return Ok(());
        }
        // Provider is not ready and the user has not cancelled yet - run the
        // slash command so the normal output-parsing path opens the overlay.
        Box::pin(self.execute_slash_command_with("/setup")).await
    }
    /// Opens a two-stage provider form (API-key or API-base) from the setup hub.
    ///
    /// Providers are pre-filtered: API-key forms only list providers that
    /// require a key (`AuthMaterialKind::ApiKey`); API-base forms list all.
    pub(in crate::tui_runtime) fn open_provider_form(&mut self, kind: ProviderFormKind) {
        let options: Vec<ProviderFormOption> = ProviderResolver::builtin()
            .registry()
            .providers()
            .filter(|p| match kind {
                ProviderFormKind::ApiKey => p.auth_kind == AuthMaterialKind::ApiKey,
                ProviderFormKind::ApiBase => true,
            })
            .map(|p| ProviderFormOption {
                provider_id: p.id.clone(),
                display_name: p.display_name.clone(),
            })
            .collect();
        // The setup overlay is replaced by the provider form.
        self.pending_setup_overlay = None;
        self.pending_provider_form = Some(ProviderFormState::new(kind, options));
        self.needs_render = true;
    }
    /// Handles keyboard input while the provider form is active.
    pub(in crate::tui_runtime) fn handle_provider_form_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        let stage = match &self.pending_provider_form {
            Some(f) => f.stage.clone(),
            None => return Ok(()),
        };
        match stage {
            ProviderFormStage::PickProvider => match key.code {
                KeyCode::Up => {
                    let f = self.pending_provider_form.as_mut().unwrap();
                    if !f.options.is_empty() {
                        f.selected_index =
                            (f.selected_index + f.options.len() - 1) % f.options.len();
                    }
                    self.needs_render = true;
                    Ok(())
                }
                KeyCode::Down => {
                    let f = self.pending_provider_form.as_mut().unwrap();
                    if !f.options.is_empty() {
                        f.selected_index = (f.selected_index + 1) % f.options.len();
                    }
                    self.needs_render = true;
                    Ok(())
                }
                KeyCode::Esc => self.cancel_provider_form(),
                _ => {
                    let advance =
                        matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline)))
                            || matches!(key.code, KeyCode::Tab | KeyCode::Enter);
                    if advance {
                        let f = self.pending_provider_form.as_mut().unwrap();
                        if f.options.is_empty() {
                            self.status_note = Some("no providers available for this form".into());
                        } else {
                            f.stage = ProviderFormStage::EnterValue;
                        }
                    } else {
                        self.status_note = Some(provider_form_status_note(&stage));
                    }
                    self.needs_render = true;
                    Ok(())
                }
            },
            ProviderFormStage::EnterValue => {
                if key.code == KeyCode::Esc {
                    // Go back to provider selection; clear the staged input.
                    let f = self.pending_provider_form.as_mut().unwrap();
                    f.stage = ProviderFormStage::PickProvider;
                    f.input = TextBuffer::new(false);
                    self.needs_render = true;
                    return Ok(());
                }
                if matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline)))
                    || key.code == KeyCode::Enter
                {
                    return self.complete_provider_form();
                }
                if let Some(res) = resolved {
                    let f = self.pending_provider_form.as_mut().unwrap();
                    apply_picker_query_edit(&mut f.input, res);
                }
                self.needs_render = true;
                Ok(())
            }
        }
    }
    /// Commits the staged value and closes the form.
    ///
    /// For API keys the value is written to [`CredentialStore`] and the status
    /// note names the provider but **never** includes the key value itself.
    /// For API-base URLs the value is written to [`SettingsStore`].
    pub(in crate::tui_runtime) fn complete_provider_form(&mut self) -> Result<()> {
        let Some(form) = self.pending_provider_form.take() else {
            return Ok(());
        };
        let Some(opt) = form.options.get(form.selected_index).cloned() else {
            self.status_note = Some("provider form: no provider selected".into());
            self.needs_render = true;
            return Ok(());
        };
        let provider_id = opt.provider_id;
        let provider_display = opt.display_name;
        let value = form.input.text().to_string();
        let Some(dir) = self.storage_dir.as_deref() else {
            self.status_note = Some("provider form: no storage dir configured".into());
            self.needs_render = true;
            return Ok(());
        };
        match form.kind {
            ProviderFormKind::ApiKey => {
                CredentialStore::new(dir).set_api_key(&provider_id, value)?;
                // Status note names the provider but NEVER includes the key value.
                self.status_note = Some(format!("API key saved for {provider_display}"));
            }
            ProviderFormKind::ApiBase => {
                let mut settings = SettingsStore::new(dir).read()?;
                settings
                    .providers
                    .entry(provider_id.clone())
                    .or_insert_with(Default::default)
                    .api_base = Some(value);
                SettingsStore::new(dir).write(&settings)?;
                self.status_note = Some(format!("API base saved for {provider_display}"));
            }
        }
        self.needs_render = true;
        Ok(())
    }
    /// Cancels the provider form and records a status note.
    pub(in crate::tui_runtime) fn cancel_provider_form(&mut self) -> Result<()> {
        self.pending_provider_form = None;
        self.status_note = Some("provider form cancelled".into());
        self.needs_render = true;
        Ok(())
    }
    ///
    /// Requests a device code synchronously (a single fast HTTP call), then
    /// shows a confirmation dialog with the verification URL and user code.
    /// The browser is **never** opened and polling does **not** begin until
    /// the user explicitly presses Enter.
    pub(in crate::tui_runtime) fn open_copilot_oauth_flow(&mut self) {
        // Clear the setup overlay so the dialog renders instead of the picker.
        self.pending_setup_overlay = None;
        self.needs_render = true;

        let device_code = match request_copilot_device_code() {
            Ok(code) => code,
            Err(err) => {
                self.dialog = Some(DialogView::notice(
                    "Copilot Login Failed",
                    [format!("Could not start Copilot login: {err}")],
                ));
                self.status_note = Some("copilot oauth: device code request failed".into());
                self.needs_render = true;
                return;
            }
        };

        // Build a confirm-style dialog showing the URL and user code.  The raw
        // `device_code` secret is kept only in `pending_copilot_oauth`, never in
        // the dialog body or any logged/displayed text.
        let dialog = DialogView {
            title: "GitHub Copilot Login".into(),
            body: vec![
                "Authorize Wonder-of-U to use GitHub Copilot.".into(),
                String::new(),
                format!("1. Visit:      {}", device_code.verification_uri),
                format!("2. Enter code: {}", device_code.user_code),
                String::new(),
                "Press Enter to open the browser and wait for authorization.".into(),
                "Press Esc to cancel.".into(),
            ],
            actions: vec![
                DialogActionView::new("Open Browser", true),
                DialogActionView::new("Cancel", false),
            ],
            selected_action: 0,
        };
        self.dialog = Some(dialog);
        self.pending_copilot_oauth =
            Some(CopilotOAuthFlowState::AwaitingConfirmation { device_code });
        self.status_note = Some("copilot oauth: press Enter to open browser".into());
        self.needs_render = true;
    }
    /// Handles key events while the Copilot OAuth dialog is visible.
    ///
    /// - `AwaitingConfirmation`: Enter / `y` opens the browser and starts
    ///   polling; Esc cancels.
    /// - `Polling`: Esc cancels (dropping the background thread result).
    pub(in crate::tui_runtime) fn handle_copilot_oauth_dialog_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        let Some(state) = &self.pending_copilot_oauth else {
            self.dismiss_dialog();
            return Ok(());
        };
        match state {
            CopilotOAuthFlowState::AwaitingConfirmation { .. } => {
                let confirmed =
                    matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline)))
                        || matches!(resolved, Some(ResolvedKey::InsertChar('y')))
                        || matches!(resolved, Some(ResolvedKey::InsertChar('Y')));
                let cancelled = key.code == KeyCode::Esc
                    || matches!(
                        resolved,
                        Some(ResolvedKey::System(
                            wonder_of_u_tui::SystemAction::Interrupt
                        ))
                    );
                if confirmed {
                    // Start polling; browser opens inside this call.
                    self.start_copilot_oauth_polling();
                } else if cancelled {
                    self.pending_copilot_oauth = None;
                    self.dialog = None;
                    self.status_note = Some("copilot oauth cancelled".into());
                    self.needs_render = true;
                } else {
                    self.status_note = Some("press Enter to open browser, Esc to cancel".into());
                    self.needs_render = true;
                }
                Ok(())
            }
            CopilotOAuthFlowState::Polling { user_code, .. } => {
                let user_code = user_code.clone();
                let cancelled = key.code == KeyCode::Esc
                    || matches!(
                        resolved,
                        Some(ResolvedKey::System(
                            wonder_of_u_tui::SystemAction::Interrupt
                        ))
                    );
                if cancelled {
                    // Drop the flow; the background thread result is discarded.
                    self.pending_copilot_oauth = None;
                    self.dialog = None;
                    self.status_note = Some("copilot oauth polling cancelled".into());
                    self.needs_render = true;
                } else {
                    // Remind the user of the code without blocking or advancing.
                    self.status_note = Some(format!(
                        "copilot oauth: waiting for authorization (code: {user_code})"
                    ));
                    self.needs_render = true;
                }
                Ok(())
            }
        }
    }
    /// Transitions from `AwaitingConfirmation` to `Polling`.
    ///
    /// Opens the browser (best-effort; failure is non-fatal) and spawns a
    /// background thread that calls [`poll_copilot_access_token`].  The main
    /// thread checks the channel on each [`UiEvent::Tick`] via
    /// [`Self::tick_copilot_oauth_poll`].
    pub(in crate::tui_runtime) fn start_copilot_oauth_polling(&mut self) {
        let Some(CopilotOAuthFlowState::AwaitingConfirmation { device_code }) =
            self.pending_copilot_oauth.take()
        else {
            return;
        };

        // Open browser; failure is non-fatal — the user can navigate manually.
        open_browser_url(&device_code.verification_uri);

        let user_code = device_code.user_code.clone();
        let dc_secret = device_code.device_code.clone();
        let interval = device_code.interval;
        let timeout = device_code.expires_in.max(1);

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = poll_copilot_access_token(
                &dc_secret,
                interval,
                std::time::Duration::from_secs(timeout),
            );
            // A send error means the receiver was dropped (user cancelled); ignore it.
            let _ = tx.send(result);
        });

        // Replace the confirmation dialog with a "polling" notice.
        self.dialog = Some(DialogView::notice(
            "GitHub Copilot Login",
            [
                "Waiting for authorization in the browser\u{2026}".to_string(),
                format!("Code: {user_code}  (still valid)"),
                String::new(),
                "Approve the request in your browser, then return here.".to_string(),
                "Press Esc to cancel.".to_string(),
            ],
        ));
        self.pending_copilot_oauth = Some(CopilotOAuthFlowState::Polling {
            user_code,
            result_rx: rx,
        });
        self.status_note = Some("copilot oauth: waiting for browser authorization\u{2026}".into());
        self.needs_render = true;
    }
    /// Checks the polling channel on each [`UiEvent::Tick`].
    ///
    /// When the background thread sends a result the OAuth token is stored via
    /// [`CredentialStore`] and the dialog is dismissed.  The raw token value is
    /// **never** included in the status note, dialog body, notifications, or
    /// any logged output.
    pub(in crate::tui_runtime) fn tick_copilot_oauth_poll(&mut self) -> Result<()> {
        // Borrow `pending_copilot_oauth` immutably to peek at the channel.
        let poll_result = if let Some(CopilotOAuthFlowState::Polling { result_rx, .. }) =
            &self.pending_copilot_oauth
        {
            match result_rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(WonderError::validation(
                    "copilot oauth polling thread disconnected",
                ))),
                // Still waiting — nothing to do this tick.
                Err(mpsc::TryRecvError::Empty) => None,
            }
        } else {
            None
        };
        // The immutable borrow ends here; we can now mutate freely.

        let Some(result) = poll_result else {
            return Ok(());
        };

        // Clear the flow state and dismiss the dialog.
        self.pending_copilot_oauth = None;
        self.dialog = None;
        self.needs_render = true;

        match result {
            Ok(token) => {
                if let Some(dir) = self.storage_dir.as_deref() {
                    CredentialStore::new(dir).set_oauth_token(
                        "copilot",
                        token.access_token,
                        token.refresh_token,
                        token.expires_at,
                    )?;
                }
                // Confirm success without ever echoing the token value.
                self.status_note = Some("GitHub Copilot authorized successfully".into());
            }
            Err(err) => {
                self.status_note = Some(format!("Copilot OAuth failed: {err}"));
            }
        }
        Ok(())
    }
    pub(in crate::tui_runtime) fn dismiss_dialog(&mut self) {
        self.dialog = None;
        self.clear_picker_overlays();
        self.pending_provider_form = None;
        self.pending_copilot_oauth = None;
        if matches!(
            self.state.input_mode,
            InputMode::PermissionPending | InputMode::TaskNotification
        ) {
            if matches!(self.state.input_mode, InputMode::PermissionPending) {
                self.state.pending_tool_approval = None;
            }
            self.state.input_mode = InputMode::Prompt;
        }
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn show_cost_threshold_dialog(&mut self) {
        self.cost_threshold_dialog_shown_session = true;
        self.dialog = Some(DialogView::notice(
            "You've spent $5 on the API this session.",
            vec![
                "Learn more about monitoring your spending:".to_string(),
                "https://code.claude.com/docs/en/costs".to_string(),
            ],
        ));
        self.needs_render = true;
    }
    pub(in crate::tui_runtime) fn dismiss_notice_dialog(&mut self) {
        let status = self
            .dialog
            .as_ref()
            .map(|dialog| overlay_closed_status(&dialog.title));
        self.dismiss_dialog();
        if let Some(status) = status {
            self.status_note = Some(status);
        }
        if self.cost_threshold_dialog_shown_session && self.dialog.is_none() {
            if let Some(storage_dir) = self.storage_dir.as_deref() {
                if let Ok(mut settings) = SettingsStore::new(storage_dir).read() {
                    settings.has_acknowledged_cost_threshold = true;
                    let _ = SettingsStore::new(storage_dir).write(&settings);
                }
            }
        }
    }
    pub(in crate::tui_runtime) fn dismiss_task_notice(&mut self) {
        self.task_notice_ttl = None;
        self.dismiss_dialog();
        self.needs_render = true;
    }
}
