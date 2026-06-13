use super::TuiController;
use super::*;

impl TuiController<'_> {
    pub(in crate::tui_runtime) async fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        let resolved = self.keymap.resolve(KeyBindingContext::Prompt, key);

        // ── Message cursor mode ──────────────────────────────────────────
        if self.message_cursor_index.is_some() {
            return self.handle_message_cursor_key(key, resolved);
        }

        // ── Entry combo: Ctrl+U enters message cursor mode ───────────────
        if key.is_ctrl_char('u')
            && !self.global_search_open
            && !self.has_modal_overlay()
            && self.history_search.is_none()
            && self.turn_state == TurnState::Idle
        {
            self.enter_message_actions();
            return Ok(());
        }

        if self.global_search_open {
            return self.handle_global_search_key(key, resolved);
        }
        // Fleet panel intercepts navigation keys — must be before modal overlay handler.
        if self.fleet_panel.is_some() {
            return self.handle_fleet_panel_key(key);
        }
        // Sidebar-focused: scroll keys target the sidebar panel.
        if self.sidebar_focused {
            return self.handle_sidebar_scroll_key(key);
        }
        // When ask_user is in free-text mode (no options, or "Other" selected),
        // bypass the dialog handler so the user can type in the prompt box.
        // When ask_user has options and the user hasn't picked "Other" yet,
        // route to the dialog handler for arrow-key option navigation.
        let skip_dialog_for_interaction = self.is_interaction_free_text();
        if !skip_dialog_for_interaction && self.has_modal_overlay() {
            return self.handle_dialog_key(key, resolved).await;
        }
        if self.history_search.is_some() {
            return self.handle_history_search_key(key, resolved);
        }

        // The footer advertises Shift+Tab permission-mode cycling in the prompt.
        // Handle BackTab before slash-suggestion interception so it never gets
        // mistaken for completion/selection input.
        if key.code == KeyCode::BackTab {
            return self.cycle_prompt_permission_mode_backward();
        }

        // Some terminals collapse modified Enter handling inconsistently even when
        // the keymap contains an explicit Shift+Enter binding, so keep a direct
        // multiline composition path here as a safety net.
        if key.code == KeyCode::Enter && key.modifiers.shift && self.vim.mode() == VimMode::Insert {
            self.prompt
                .apply_edit_action(EditAction::InsertLiteralNewline);
            self.turn_state = TurnState::EditingInput;
            self.state.input_mode = InputMode::Prompt;
            self.reset_history_recall();
            self.status_note = None;
            self.update_slash_suggestions();
            self.update_file_mentions();
            self.needs_render = true;
            return Ok(());
        }

        // Slash-autocomplete intercepts: Tab accepts, Up/Down navigate, Esc dismisses.
        if self.active_suggestions.is_some() {
            match key.code {
                KeyCode::Tab => {
                    self.accept_slash_suggestion();
                    return Ok(());
                }
                KeyCode::Esc => {
                    self.active_suggestions = None;
                    self.needs_render = true;
                    return Ok(());
                }
                KeyCode::Up => {
                    self.navigate_slash_suggestions(-1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.navigate_slash_suggestions(1);
                    return Ok(());
                }
                _ => {}
            }
        }

        // File-mention autocomplete intercepts: Tab accepts, Up/Down navigate, Esc dismisses.
        if self.file_mentions.is_some() {
            match key.code {
                KeyCode::Tab => {
                    self.accept_file_mention();
                    return Ok(());
                }
                KeyCode::Esc => {
                    self.file_mentions = None;
                    self.needs_render = true;
                    return Ok(());
                }
                KeyCode::Up => {
                    self.navigate_file_mentions(-1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.navigate_file_mentions(1);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Route scroll keys to the transcript when no picker/dialog overlay is active.
        // Plain Home/End fall through intentionally so they reach the keymap resolver
        // and perform prompt cursor movement.
        if !self.has_picker_overlay() && self.dialog.is_none() {
            let page = self.scroll_state.last_visible_lines.max(1);
            match (key.code, key.modifiers.control, key.modifiers.alt) {
                (KeyCode::PageUp, _, _) => {
                    self.scroll_state.scroll_by(page as i32);
                    self.needs_render = true;
                    return Ok(());
                }
                (KeyCode::PageDown, _, _) => {
                    self.scroll_state.scroll_by(-(page as i32));
                    self.needs_render = true;
                    return Ok(());
                }
                (KeyCode::Home, true, _) => {
                    self.scroll_state.scroll_to_top();
                    self.needs_render = true;
                    return Ok(());
                }
                (KeyCode::End, true, _) => {
                    self.scroll_state.scroll_to_bottom();
                    self.needs_render = true;
                    return Ok(());
                }
                // Alt+Up / Alt+Down: scroll transcript line-by-line.
                // These don't interfere with prompt editing since Alt modifies
                // the key semantics.
                (KeyCode::Up, _, true) => {
                    self.scroll_state.scroll_by(KEYBOARD_SCROLL_LINES);
                    self.needs_render = true;
                    return Ok(());
                }
                (KeyCode::Down, _, true) => {
                    self.scroll_state.scroll_by(-KEYBOARD_SCROLL_LINES);
                    self.needs_render = true;
                    return Ok(());
                }
                // Plain Up: history recall when cursor is on the first line.
                (KeyCode::Up, false, false) if self.prompt.current_line_start() == 0 => {
                    return self.recall_history_prev();
                }
                // Plain Down: step forward in recall mode.
                (KeyCode::Down, false, false) if self.history_recall_index.is_some() => {
                    return self.recall_history_next();
                }
                (KeyCode::Down, false, false) => {}
                _ => {}
            }
        }

        // Ctrl+B toggles the sidebar panel.
        if key.is_ctrl_char('b') {
            self.toggle_sidebar();
            return Ok(());
        }
        // Ctrl+Shift+B cycles sidebar push/overlay mode.
        if key.is_ctrl_char('B') {
            self.cycle_sidebar_mode();
            return Ok(());
        }
        // Ctrl+G focuses/unfocuses the sidebar for scrolling (when visible).
        if key.is_ctrl_char('g') {
            if self.sidebar_visible {
                self.sidebar_focused = !self.sidebar_focused;
                self.status_note = Some(if self.sidebar_focused {
                    "sidebar focused".into()
                } else {
                    "sidebar unfocused".into()
                });
                self.needs_render = true;
            }
            return Ok(());
        }

        // Ctrl+F toggles the fleet panel.
        if key.is_ctrl_char('f') {
            if self.fleet_panel.is_some() {
                self.close_fleet_panel();
            } else {
                self.open_fleet_panel();
            }
            return Ok(());
        }

        // Ctrl+Y or Ctrl+Shift+C enters terminal-native text selection mode.
        // While active, mouse capture is suspended so the terminal emulator
        // can handle click-drag selection and clipboard copy natively.
        // Press any key to exit selection mode and resume normal TUI input.
        //
        // Ctrl+Y is the primary binding because Ctrl+Shift+C is often
        // intercepted by the terminal for its own copy-to-clipboard action.
        if key.is_ctrl_char('y')
            || (key.code == KeyCode::Char('c') && key.modifiers.control && key.modifiers.shift)
        {
            self.enter_selection_mode();
            return Ok(());
        }

        if let Some(ResolvedKey::System(
            system @ (wonder_of_u_tui::SystemAction::OpenModelPicker
            | wonder_of_u_tui::SystemAction::ToggleThinking
            | wonder_of_u_tui::SystemAction::ToggleFastMode),
        )) = resolved
        {
            return self.handle_prompt_hotkey_system_action(system).await;
        }

        let Some(resolved) = resolved else {
            return if self.vim_enabled
                && (self.vim.mode() != VimMode::Insert || key.code == KeyCode::Esc)
            {
                self.handle_vim_key(key)
            } else {
                Ok(())
            };
        };

        if self.vim_enabled && (self.vim.mode() != VimMode::Insert || key.code == KeyCode::Esc) {
            return self.handle_vim_key(key);
        }

        match resolved {
            ResolvedKey::System(system) => self.handle_system_action(system),
            ResolvedKey::InsertChar(ch) => {
                self.prompt.insert_char(ch);
                self.turn_state = TurnState::EditingInput;
                self.state.input_mode = InputMode::Prompt;
                self.reset_history_recall();
                self.status_note = None;
                self.update_slash_suggestions();
                self.update_file_mentions();
                self.needs_render = true;
                Ok(())
            }
            ResolvedKey::Edit(EditAction::InsertNewline) => {
                // If a suggestion is selected and the user presses Enter, accept it.
                if self
                    .active_suggestions
                    .as_ref()
                    .is_some_and(|s| s.selected().is_some())
                {
                    self.accept_slash_suggestion();
                    return self.submit_prompt().await;
                }
                if self
                    .file_mentions
                    .as_ref()
                    .is_some_and(|s| s.selected().is_some())
                {
                    self.accept_file_mention();
                    return self.submit_prompt().await;
                }
                self.active_suggestions = None;
                self.file_mentions = None;
                self.submit_prompt().await
            }
            ResolvedKey::Edit(action) => {
                self.prompt.apply_edit_action(action);
                self.turn_state = TurnState::EditingInput;
                self.state.input_mode = InputMode::Prompt;
                self.reset_history_recall();
                self.status_note = None;
                self.update_slash_suggestions();
                self.update_file_mentions();
                self.needs_render = true;
                Ok(())
            }
            ResolvedKey::Vim(_) => Ok(()),
        }
    }
    pub(in crate::tui_runtime) async fn handle_prompt_hotkey_system_action(
        &mut self,
        system: wonder_of_u_tui::SystemAction,
    ) -> Result<()> {
        match system {
            wonder_of_u_tui::SystemAction::OpenModelPicker => {
                self.execute_slash_command_with("/model").await
            }
            wonder_of_u_tui::SystemAction::ToggleThinking => {
                let command = if self.state.thinking_enabled {
                    "/thinking off"
                } else {
                    "/thinking on"
                };
                self.execute_slash_command_with(command).await
            }
            wonder_of_u_tui::SystemAction::ToggleFastMode => {
                let command = if self.state.fast_mode {
                    "/fast off"
                } else {
                    "/fast on"
                };
                self.execute_slash_command_with(command).await
            }
            _ => self.handle_system_action(system),
        }
    }
    pub(in crate::tui_runtime) fn handle_vim_key(&mut self, key: KeyEvent) -> Result<()> {
        let result = self.vim.handle_key(&mut self.prompt, &self.keymap, key);
        if let Some(system) = result.system {
            return self.handle_system_action(system);
        }
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.reset_history_recall();
        self.update_slash_suggestions();
        self.update_file_mentions();
        self.status_note = Some(match self.vim.mode() {
            VimMode::Insert => "vim insert".into(),
            VimMode::Normal if self.vim.has_pending_operator() => "vim operator pending".into(),
            VimMode::Normal => "vim normal".into(),
            VimMode::Visual => "vim visual".into(),
        });
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn handle_system_action(
        &mut self,
        system: wonder_of_u_tui::SystemAction,
    ) -> Result<()> {
        match system {
            wonder_of_u_tui::SystemAction::Interrupt => {
                if self.has_active_turn() {
                    self.active_turn = ActiveTurn::None;
                    self.turn_state = TurnState::Interrupted;
                    self.state.input_mode = InputMode::Prompt;
                    self.status_note = Some("turn interrupted".into());
                    self.dismiss_dialog();
                    self.needs_render = true;
                    return Ok(());
                }
                if self.should_confirm_exit() {
                    self.dialog = Some(DialogView::confirm(
                        "Exit session?",
                        ["Use Up/Down to choose an action, Enter to confirm."],
                    ));
                    self.status_note = Some("confirm exit".into());
                    self.needs_render = true;
                    return Ok(());
                }
                self.turn_state = TurnState::Interrupted;
                self.exit_requested = true;
                self.needs_render = true;
                Ok(())
            }
            wonder_of_u_tui::SystemAction::Redraw => {
                self.status_note = Some("screen redrawn".into());
                self.needs_render = true;
                Ok(())
            }
            wonder_of_u_tui::SystemAction::HistorySearch => self.open_or_step_history_search(),
            wonder_of_u_tui::SystemAction::OpenGlobalSearch => {
                self.toggle_global_search(None);
                Ok(())
            }
            wonder_of_u_tui::SystemAction::ExpandToolOutput => {
                self.toggle_expand_tool_output();
                Ok(())
            }
            wonder_of_u_tui::SystemAction::OpenModelPicker
            | wonder_of_u_tui::SystemAction::ToggleThinking
            | wonder_of_u_tui::SystemAction::ToggleFastMode => Ok(()),
        }
    }
}
