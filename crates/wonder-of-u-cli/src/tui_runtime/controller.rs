use super::*;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) enum ActiveOverlay {
    HistorySearch,
    Picker,
    ConfirmDialog,
    NoticeDialog,
    None,
}

pub(super) struct TuiController<'a> {
    pub(super) registry: &'a CommandRegistry,
    pub(super) storage_dir: Option<PathBuf>,
    pub(super) state: AppState,
    pub(super) persistence: SessionPersistenceState,
    pub(super) prompt: TextBuffer,
    pub(super) keymap: KeyBindingResolver,
    pub(super) vim: VimState,
    pub(super) history_search: Option<HistorySearchState>,
    pub(super) turn_state: TurnState,
    pub(super) needs_render: bool,
    pub(super) exit_requested: bool,
    pub(super) status_note: Option<String>,
    pub(super) dialog: Option<DialogView>,
    pub(super) pending_tag_removal: Option<TagRemovalState>,
    pub(super) pending_theme_picker: Option<ThemePickerState>,
    pub(super) pending_model_picker: Option<ModelPickerState>,
    pub(super) pending_permission_picker: Option<PermissionPickerState>,
    pub(super) pending_memory_picker: Option<MemoryPickerState>,
    pub(super) pending_external_editor: Option<ExternalEditorRequest>,
    /// Countdown ticks until the task notification dialog is auto-dismissed.
    pub(super) task_notice_ttl: Option<u8>,
    pub(super) notifications: NotificationQueue,
    /// All slash-command suggestions, built once at startup.
    pub(super) slash_suggestions: Vec<PromptSuggestion>,
    /// Live filtered state when the user is typing a `/` command.
    pub(super) active_suggestions: Option<PromptSuggestionState>,
    /// Ephemeral transcript scroll position; never persisted to `AppState`.
    pub(super) scroll_state: TranscriptScrollState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModelPickerOption {
    pub(super) provider: String,
    pub(super) provider_display: String,
    pub(super) model: String,
    pub(super) model_display: String,
    pub(super) default: bool,
    pub(super) selected: bool,
    pub(super) auth: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModelPickerState {
    pub(super) original_input: String,
    pub(super) options: Vec<ModelPickerOption>,
    pub(super) selected_index: usize,
    pub(super) query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ThemePickerOption {
    pub(super) theme: String,
    pub(super) label: String,
    pub(super) description: String,
    pub(super) selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ThemePickerState {
    pub(super) original_input: String,
    pub(super) options: Vec<ThemePickerOption>,
    pub(super) selected_index: usize,
    pub(super) query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TagRemovalState {
    pub(super) original_input: String,
    pub(super) tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PermissionPickerOption {
    pub(super) mode: PermissionMode,
    pub(super) label: String,
    pub(super) description: String,
    pub(super) selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PermissionPickerState {
    pub(super) original_input: String,
    pub(super) options: Vec<PermissionPickerOption>,
    pub(super) selected_index: usize,
    pub(super) query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MemoryPickerOption {
    pub(super) target: crate::commands::project::MemoryTarget,
    pub(super) label: String,
    pub(super) description: String,
    pub(super) path: PathBuf,
    pub(super) selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MemoryPickerState {
    pub(super) original_input: String,
    pub(super) options: Vec<MemoryPickerOption>,
    pub(super) selected_index: usize,
    pub(super) query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HistorySearchState {
    pub(super) query: TextBuffer,
    pub(super) matches: Vec<usize>,
    pub(super) cursor: usize,
    pub(super) saved_buffer: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExternalEditorRequest {
    pub(super) cwd: PathBuf,
    pub(super) path: PathBuf,
}

#[derive(Clone)]
pub(super) struct LocalToolCall {
    pub(super) provider_call: ProviderToolCall,
    pub(super) use_id: ToolUseId,
}

pub(super) enum ToolExecutionOutcome {
    Completed(ProviderToolResultMessage),
    Paused { reason: String },
}

pub(super) enum RestoredPromptUiState {
    PermissionPicker(PermissionPickerState),
    MemoryPicker(MemoryPickerState),
    TagRemoval(TagRemovalState),
    ThemePicker(ThemePickerState),
    ModelPicker(ModelPickerState),
    Notice {
        dialog: DialogView,
        status_note: String,
    },
    Status(String),
}

impl<'a> TuiController<'a> {
    pub(super) fn new(
        context: CommandContext,
        registry: &'a CommandRegistry,
        storage_dir: Option<&Path>,
        options: TuiLaunchOptions,
    ) -> Result<Self> {
        let (state, persistence) = load_or_create_state(
            &context,
            storage_dir,
            options.session_id.as_deref(),
            &default_session_title(&context.cwd),
            "tui",
        )?;
        let mut controller = Self {
            registry,
            storage_dir: storage_dir.map(Path::to_path_buf),
            state,
            persistence,
            prompt: TextBuffer::new(false),
            keymap: crate::commands::workflow::load_keybinding_resolver(storage_dir)?,
            vim: VimState::default(),
            history_search: None,
            turn_state: TurnState::Idle,
            needs_render: true,
            exit_requested: false,
            status_note: None,
            dialog: None,
            pending_tag_removal: None,
            pending_theme_picker: None,
            pending_model_picker: None,
            pending_permission_picker: None,
            pending_memory_picker: None,
            pending_external_editor: None,
            task_notice_ttl: None,
            notifications: NotificationQueue::new(),
            slash_suggestions: build_slash_suggestions(registry),
            active_suggestions: None,
            scroll_state: TranscriptScrollState::new(),
        };
        controller.hydrate_initial_settings()?;
        controller.refresh_runtime_state()?;
        controller.rebuild_ephemeral_state();
        controller.persist_state_snapshot()?;
        Ok(controller)
    }

    pub(super) fn hydrate_initial_settings(&mut self) -> Result<()> {
        if !self.state.messages.is_empty() {
            return Ok(());
        }
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        let settings = SettingsStore::new(storage_dir).read()?;
        self.state.effort_level = settings.effort_level;
        self.state.fast_mode = settings.fast_mode;
        Ok(())
    }

    pub(super) fn handle_event<F>(&mut self, event: UiEvent, mut before_blocking: F) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        match event {
            UiEvent::Key(key) => self.handle_key_event(key, &mut before_blocking),
            UiEvent::Paste(text) => {
                if !text.is_empty() {
                    if self.history_search.is_some() {
                        self.edit_history_search_query_text(&text);
                    } else {
                        self.prompt.insert_text(&text);
                        self.turn_state = TurnState::EditingInput;
                        self.state.input_mode = InputMode::Prompt;
                        self.reset_history_recall();
                        self.status_note = None;
                        self.needs_render = true;
                    }
                }
                Ok(())
            }
            UiEvent::Tick => {
                match self.task_notice_ttl {
                    Some(0) => {
                        self.dismiss_task_notice();
                    }
                    Some(n) => {
                        self.task_notice_ttl = Some(n - 1);
                    }
                    None => {}
                }
                self.needs_render |= self.notifications.tick();
                if self.refresh_runtime_state()? {
                    self.needs_render = true;
                }
                Ok(())
            }
            UiEvent::FocusGained => {
                self.needs_render |= self.notifications.set_window_focused(true);
                self.needs_render = true;
                Ok(())
            }
            UiEvent::FocusLost => {
                self.needs_render |= self.notifications.set_window_focused(false);
                self.needs_render = true;
                Ok(())
            }
            UiEvent::Resize { width, height } => {
                self.needs_render = true;
                self.on_terminal_resize(width, height);
                Ok(())
            }
            UiEvent::Mouse(_) => {
                self.needs_render = true;
                Ok(())
            }
        }
    }

    pub(super) fn handle_key_event<F>(
        &mut self,
        key: KeyEvent,
        before_blocking: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let resolved = self.keymap.resolve(KeyBindingContext::Prompt, key);
        if self.dialog.is_some() {
            return self.handle_dialog_key(key, resolved, before_blocking);
        }
        if self.history_search.is_some() {
            return self.handle_history_search_key(key, resolved);
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

        let Some(resolved) = resolved else {
            return if self.vim.mode() == VimMode::Normal || key.code == KeyCode::Esc {
                self.handle_vim_key(key)
            } else {
                Ok(())
            };
        };
        if self.vim.mode() == VimMode::Normal || key.code == KeyCode::Esc {
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
                    return self.submit_prompt(before_blocking);
                }
                self.active_suggestions = None;
                self.submit_prompt(before_blocking)
            }
            ResolvedKey::Edit(action) => {
                self.prompt.apply_edit_action(action);
                self.turn_state = TurnState::EditingInput;
                self.state.input_mode = InputMode::Prompt;
                self.reset_history_recall();
                self.status_note = None;
                self.update_slash_suggestions();
                self.needs_render = true;
                Ok(())
            }
            ResolvedKey::Vim(_) => Ok(()),
        }
    }

    pub(super) fn handle_vim_key(&mut self, key: KeyEvent) -> Result<()> {
        let result = self.vim.handle_key(&mut self.prompt, &self.keymap, key);
        if let Some(system) = result.system {
            return self.handle_system_action(system);
        }
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.reset_history_recall();
        self.status_note = Some(match self.vim.mode() {
            VimMode::Insert => "vim insert".into(),
            VimMode::Normal if self.vim.has_pending_operator() => "vim operator pending".into(),
            VimMode::Normal => "vim normal".into(),
        });
        self.needs_render = true;
        Ok(())
    }

    /// Rebuild the slash-command suggestion overlay based on the current prompt buffer.
    pub(super) fn update_slash_suggestions(&mut self) {
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

    /// Move the selection cursor in the active suggestion list by `delta` (+1 down, -1 up).
    pub(super) fn navigate_slash_suggestions(&mut self, delta: i32) {
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
    pub(super) fn accept_slash_suggestion(&mut self) {
        let replacement = self
            .active_suggestions
            .as_ref()
            .and_then(|s| s.selected())
            .map(|s| s.replacement.clone());
        if let Some(text) = replacement {
            self.prompt = TextBuffer::new(false);
            for ch in text.chars() {
                self.prompt.insert_char(ch);
            }
            self.active_suggestions = None;
            self.needs_render = true;
        }
    }

    pub(super) fn handle_system_action(
        &mut self,
        system: wonder_of_u_tui::SystemAction,
    ) -> Result<()> {
        match system {
            wonder_of_u_tui::SystemAction::Interrupt => {
                if self.should_confirm_exit() {
                    self.dialog = Some(DialogView::confirm(
                        "Exit session?",
                        [
                            "Press Enter to close this TUI session.",
                            "Press any other key to keep working.",
                        ],
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
        }
    }

    pub(super) fn handle_history_search_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => return self.cancel_history_search(),
            KeyCode::Up => {
                self.step_history_search(1);
                return Ok(());
            }
            KeyCode::Down => {
                self.step_history_search(-1);
                return Ok(());
            }
            _ => {}
        }

        match resolved {
            Some(ResolvedKey::Edit(EditAction::InsertNewline)) => self.accept_history_search(),
            Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::HistorySearch)) => {
                self.step_history_search(1);
                Ok(())
            }
            Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Redraw)) => {
                self.needs_render = true;
                Ok(())
            }
            Some(ResolvedKey::System(system)) => self.handle_system_action(system),
            Some(resolved) if self.edit_history_search_query(resolved) => Ok(()),
            Some(_) | None => {
                self.status_note =
                    Some(history_search_status_note(self.history_search_has_match()));
                self.needs_render = true;
                Ok(())
            }
        }
    }

    pub(super) fn handle_dialog_key<F>(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
        before_blocking: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
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
        if self.pending_tag_removal.is_some() {
            return self.handle_tag_removal_key(key, resolved);
        }
        if self.pending_theme_picker.is_some() {
            return self.handle_theme_picker_key(key, resolved);
        }
        if self.pending_model_picker.is_some() {
            return self.handle_model_picker_key(key, resolved, before_blocking);
        }
        match self.dialog.as_ref().map(DialogView::kind) {
            Some(wonder_of_u_tui::DialogKind::Permission) => {
                self.handle_permission_dialog_key(key, resolved, before_blocking)
            }
            Some(wonder_of_u_tui::DialogKind::Confirm)
                if matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline))) =>
            {
                self.dialog = None;
                self.turn_state = TurnState::Interrupted;
                self.exit_requested = true;
                self.needs_render = true;
                Ok(())
            }
            Some(wonder_of_u_tui::DialogKind::Confirm) => {
                self.status_note = Some("exit cancelled".into());
                self.dismiss_dialog();
                Ok(())
            }
            Some(wonder_of_u_tui::DialogKind::Notice) => {
                self.dismiss_notice_dialog();
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub(super) fn handle_permission_picker_key(
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

    pub(super) fn handle_model_picker_key<F>(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
        _before_blocking: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
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

    pub(super) fn handle_memory_picker_key(
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

    pub(super) fn handle_theme_picker_key(
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

    pub(super) fn handle_permission_dialog_key<F>(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
        before_blocking: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        match resolved {
            Some(ResolvedKey::Edit(EditAction::InsertNewline))
            | Some(ResolvedKey::InsertChar('y'))
            | Some(ResolvedKey::InsertChar('Y')) => {
                self.resolve_pending_tool_approval(true, before_blocking)
            }
            _ if key.code == KeyCode::Esc => {
                self.resolve_pending_tool_approval(false, before_blocking)
            }
            Some(ResolvedKey::InsertChar('n'))
            | Some(ResolvedKey::InsertChar('N'))
            | Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt)) => {
                self.resolve_pending_tool_approval(false, before_blocking)
            }
            _ => {
                self.status_note = Some("press Enter/y to allow, n/Esc to deny".into());
                self.needs_render = true;
                Ok(())
            }
        }
    }

    pub(super) fn resolve_pending_tool_approval<F>(
        &mut self,
        approved: bool,
        before_blocking: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let Some(pending) = self.state.pending_tool_approval.take() else {
            self.dismiss_dialog();
            return Ok(());
        };

        self.dialog = None;
        self.state.input_mode = InputMode::Prompt;
        self.needs_render = true;

        let runtime = ProviderRuntime::new();
        let resolved = self.resolve_prompt_execution(&runtime)?;
        self.state.set_provider_context(
            Some(resolved.provider_id().to_string()),
            Some(resolved.model().to_string()),
            resolved.auth_state(),
        );

        let registry = builtin_tool_registry()?;
        let tool_context = self.tool_context();
        let provider_tools = provider_tool_specs(&registry, &tool_context, None)
            .into_iter()
            .map(tool_spec_to_provider_tool)
            .collect::<Vec<_>>();

        let mut rounds = pending
            .rounds
            .iter()
            .map(runtime_round_from_pending)
            .collect::<Vec<_>>();
        let mut current_round = runtime_round_from_pending(&pending.current_round);
        let pending_call = local_call_from_pending(&pending.pending_call);
        let first_result = if approved {
            self.approve_pending_tool_call(
                &registry,
                &tool_context,
                &pending_call,
                &pending.reason,
                before_blocking,
            )?
        } else {
            self.deny_pending_tool_call(&pending_call, &pending.reason, before_blocking)?
        };
        current_round.results.push(first_result);

        let remaining_calls = pending
            .remaining_calls
            .iter()
            .map(local_call_from_pending)
            .collect::<Vec<_>>();
        for (index, call) in remaining_calls.iter().cloned().enumerate() {
            match self.execute_tool_call(
                &registry,
                &tool_context,
                &call.provider_call,
                call.use_id,
                before_blocking,
            )? {
                ToolExecutionOutcome::Completed(result) => current_round.results.push(result),
                ToolExecutionOutcome::Paused { reason } => {
                    self.state.pending_tool_approval = Some(PendingToolApprovalState {
                        request_prompt: pending.request_prompt.clone(),
                        rounds: rounds.iter().map(pending_round_from_runtime).collect(),
                        current_round: pending_round_from_runtime(&current_round),
                        pending_call: pending_local_call_from_runtime(&call),
                        remaining_calls: remaining_calls
                            .iter()
                            .skip(index + 1)
                            .map(pending_local_call_from_runtime)
                            .collect(),
                        reason,
                    });
                    self.persist_state_snapshot()?;
                    return Ok(());
                }
            }
        }

        rounds.push(current_round);
        self.continue_tool_loop_from_rounds(
            &pending.request_prompt,
            &runtime,
            &resolved,
            &provider_tools,
            rounds,
            before_blocking,
        )
    }

    pub(super) fn approve_pending_tool_call<F>(
        &mut self,
        registry: &wonder_of_u_core::ToolRegistry,
        context: &ToolContext,
        call: &LocalToolCall,
        reason: &str,
        before_blocking: &mut F,
    ) -> Result<ProviderToolResultMessage>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let permission_message = append_contextual_message(
            &mut self.state,
            MessagePayload::Permission {
                tool: call.provider_call.tool_name.clone(),
                decision: "allow".into(),
                reason: format!("approved in tui: {reason}"),
            },
        )?;
        let query = ToolQuery::from(context);
        if let Some(tool) = registry.resolve_enabled(&call.provider_call.tool_name, &query) {
            self.turn_state = TurnState::ToolExecuting;
            self.state.input_mode = InputMode::Bash;
            self.status_note = Some(format!("running tool {}", call.provider_call.tool_name));
            self.needs_render = true;
            before_blocking(self)?;
            let result = match block_on(tool.execute(
                context.clone(),
                call.use_id,
                call.provider_call.arguments.clone(),
            )) {
                Ok(result) => result,
                Err(error) => {
                    ToolResult::failure(call.use_id, format!("tool execution failed: {error}"))
                }
            };
            self.finalize_tool_result(
                vec![permission_message],
                &call.provider_call,
                result,
                before_blocking,
            )
        } else {
            self.finalize_tool_result(
                vec![permission_message],
                &call.provider_call,
                ToolResult::failure(
                    call.use_id,
                    format!(
                        "tool `{}` is not available in this session",
                        call.provider_call.tool_name
                    ),
                ),
                before_blocking,
            )
        }
    }

    pub(super) fn deny_pending_tool_call<F>(
        &mut self,
        call: &LocalToolCall,
        reason: &str,
        before_blocking: &mut F,
    ) -> Result<ProviderToolResultMessage>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let permission_message = append_contextual_message(
            &mut self.state,
            MessagePayload::Permission {
                tool: call.provider_call.tool_name.clone(),
                decision: "deny".into(),
                reason: format!("denied in tui: {reason}"),
            },
        )?;
        self.turn_state = TurnState::Completed;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some(format!("denied tool {}", call.provider_call.tool_name));
        self.finalize_tool_result(
            vec![permission_message],
            &call.provider_call,
            ToolResult::failure(
                call.use_id,
                format!("tool execution denied by user: {reason}"),
            ),
            before_blocking,
        )
    }

    pub(super) fn continue_tool_loop_from_rounds<F>(
        &mut self,
        request_prompt: &str,
        runtime: &ProviderRuntime,
        resolved: &wonder_of_u_agent::ResolvedProviderExecution,
        provider_tools: &[ProviderToolSpec],
        mut rounds: Vec<ToolConversationRound>,
        before_blocking: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let registry = builtin_tool_registry()?;
        let tool_context = self.tool_context();

        for iteration in rounds.len()..MAX_TOOL_LOOP_ITERATIONS {
            self.turn_state = TurnState::ModelRequestActive;
            self.state.input_mode = InputMode::Prompt;
            self.status_note = Some(format!(
                "continuing tool loop {}/{}",
                iteration + 1,
                MAX_TOOL_LOOP_ITERATIONS
            ));
            self.needs_render = true;
            before_blocking(self)?;

            let response = runtime.complete_with_tool_use(
                resolved,
                &ToolUseRequest {
                    prompt: request_prompt.to_string(),
                    system_prompt: self.state.effective_system_prompt(None),
                    max_output_tokens: None,
                    temperature: None,
                    tools: provider_tools.to_vec(),
                    rounds: rounds.clone(),
                    effort_level: self.state.effort_level.clone(),
                },
            )?;

            match response {
                ToolUseResponse::Final(response) => {
                    self.state.pending_tool_approval = None;
                    self.state.record_cost_usage(response.usage, None);
                    let assistant_message = append_contextual_message(
                        &mut self.state,
                        MessagePayload::AssistantText {
                            content: response.output_text,
                        },
                    )?;
                    self.persist_messages(&[assistant_message])?;
                    self.turn_state = TurnState::Completed;
                    self.state.input_mode = InputMode::Prompt;
                    self.status_note = Some("tool loop response recorded".into());
                    return Ok(());
                }
                ToolUseResponse::ToolCalls(batch) => {
                    self.state.record_cost_usage(batch.usage, None);
                    let local_calls = batch
                        .calls
                        .iter()
                        .map(|call| LocalToolCall {
                            provider_call: call.clone(),
                            use_id: ToolUseId::new(),
                        })
                        .collect::<Vec<_>>();

                    let mut staged_messages = Vec::new();
                    if let Some(text) = batch
                        .assistant_text
                        .as_deref()
                        .filter(|text| !text.trim().is_empty())
                    {
                        staged_messages.push(append_contextual_message(
                            &mut self.state,
                            MessagePayload::AssistantText {
                                content: text.to_string(),
                            },
                        )?);
                    }
                    for call in &local_calls {
                        staged_messages.push(append_contextual_message(
                            &mut self.state,
                            MessagePayload::AssistantToolUse {
                                tool: call.provider_call.tool_name.clone(),
                                use_id: call.use_id,
                                input: call.provider_call.arguments.clone(),
                            },
                        )?);
                    }
                    self.persist_messages(&staged_messages)?;
                    self.needs_render = true;
                    before_blocking(self)?;

                    let mut round = ToolConversationRound {
                        assistant_text: batch.assistant_text.filter(|text| !text.trim().is_empty()),
                        calls: batch.calls,
                        results: Vec::new(),
                    };
                    for (index, call) in local_calls.iter().cloned().enumerate() {
                        match self.execute_tool_call(
                            &registry,
                            &tool_context,
                            &call.provider_call,
                            call.use_id,
                            before_blocking,
                        )? {
                            ToolExecutionOutcome::Completed(result) => round.results.push(result),
                            ToolExecutionOutcome::Paused { reason } => {
                                self.state.pending_tool_approval = Some(PendingToolApprovalState {
                                    request_prompt: request_prompt.to_string(),
                                    rounds: rounds.iter().map(pending_round_from_runtime).collect(),
                                    current_round: pending_round_from_runtime(&round),
                                    pending_call: pending_local_call_from_runtime(&call),
                                    remaining_calls: local_calls
                                        .iter()
                                        .skip(index + 1)
                                        .map(pending_local_call_from_runtime)
                                        .collect(),
                                    reason,
                                });
                                self.persist_state_snapshot()?;
                                return Ok(());
                            }
                        }
                    }
                    rounds.push(round);
                }
            }
        }

        self.state.pending_tool_approval = None;
        let limit_message = append_contextual_message(
            &mut self.state,
            MessagePayload::System {
                content: format!(
                    "tool loop stopped after {MAX_TOOL_LOOP_ITERATIONS} iterations without a final assistant response"
                ),
            },
        )?;
        self.persist_messages(&[limit_message])?;
        self.turn_state = TurnState::Completed;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some("tool loop hit iteration limit".into());
        Ok(())
    }

    pub(super) fn submit_prompt<F>(&mut self, before_blocking: &mut F) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let original = self.prompt.text();
        let input = original.trim().to_string();
        if input.is_empty() {
            self.status_note = Some("prompt is empty".into());
            self.turn_state = TurnState::Completed;
            self.needs_render = true;
            return Ok(());
        }

        self.prompt = TextBuffer::new(false);
        self.reset_history_recall();
        self.status_note = None;
        self.needs_render = true;

        let result = if input.starts_with('/') {
            self.turn_state = TurnState::CommandQueued;
            before_blocking(self)?;
            self.execute_slash_command_with(&input, before_blocking)
        } else {
            self.turn_state = TurnState::ModelRequestActive;
            before_blocking(self)?;
            self.execute_prompt_submission(&input, before_blocking)
        };

        match result {
            Ok(()) => {
                if !matches!(self.turn_state, TurnState::ToolPermissionPending) {
                    self.turn_state = TurnState::Completed;
                }
                self.needs_render = true;
            }
            Err(error) => {
                self.prompt = TextBuffer::from_text(&original, false);
                self.turn_state = TurnState::Interrupted;
                self.status_note = Some(format!("error: {error}"));
                self.needs_render = true;
            }
        }

        Ok(())
    }

    pub(super) fn execute_prompt_submission<F>(
        &mut self,
        input: &str,
        on_progress: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let request_prompt = compose_conversation_prompt(&self.state.messages, input);
        let runtime = ProviderRuntime::new();
        let resolved = self.resolve_prompt_execution(&runtime)?;
        self.state.set_provider_context(
            Some(resolved.provider_id().to_string()),
            Some(resolved.model().to_string()),
            resolved.auth_state(),
        );

        if runtime.supports_tool_use_for(&resolved) {
            return self.execute_tool_loop_submission(
                input,
                &request_prompt,
                &runtime,
                &resolved,
                on_progress,
            );
        }

        let request = CompletionRequest {
            prompt: request_prompt,
            system_prompt: self.state.effective_system_prompt(None),
            max_output_tokens: None,
            temperature: None,
            effort_level: self.state.effort_level.clone(),
        };
        let streaming = runtime.supports_streaming(resolved.provider_id());

        let user_message = append_contextual_message(
            &mut self.state,
            MessagePayload::UserText {
                content: input.into(),
            },
        )?;
        let assistant_index = self.state.messages.len();
        let _assistant_placeholder = append_contextual_message(
            &mut self.state,
            MessagePayload::AssistantText {
                content: String::new(),
            },
        )?;
        self.needs_render = true;
        on_progress(self)?;

        let response = if streaming {
            runtime.complete_streaming(&resolved, &request, |delta| {
                append_streamed_text(&mut self.state, assistant_index, delta)?;
                self.turn_state = TurnState::StreamingResponse;
                self.status_note = Some(format!("streaming {} response", resolved.provider_id()));
                self.needs_render = true;
                on_progress(self)
            })?
        } else {
            let response = runtime.complete(&resolved, &request)?;
            set_assistant_message_content(&mut self.state, assistant_index, &response.output_text)?;
            response
        };

        self.state.record_cost_usage(response.usage, None);
        let assistant_message = self
            .state
            .messages
            .get(assistant_index)
            .cloned()
            .ok_or_else(|| WonderError::internal("missing streamed assistant message"))?;
        persist_messages_and_state(
            self.storage_dir.as_deref(),
            &self.state,
            &mut self.persistence,
            &[user_message, assistant_message],
        )?;
        self.status_note = Some(if streaming {
            "streamed model response recorded".into()
        } else {
            "model response recorded".into()
        });
        Ok(())
    }

    pub(super) fn execute_tool_loop_submission<F>(
        &mut self,
        input: &str,
        request_prompt: &str,
        runtime: &ProviderRuntime,
        resolved: &wonder_of_u_agent::ResolvedProviderExecution,
        on_progress: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        self.state.pending_tool_approval = None;
        let registry = builtin_tool_registry()?;
        let tool_context = self.tool_context();
        let provider_tools = provider_tool_specs(&registry, &tool_context, None)
            .into_iter()
            .map(tool_spec_to_provider_tool)
            .collect::<Vec<_>>();

        let user_message = append_contextual_message(
            &mut self.state,
            MessagePayload::UserText {
                content: input.into(),
            },
        )?;
        let mut staged_messages = vec![user_message];
        self.needs_render = true;
        on_progress(self)?;

        let mut rounds = Vec::<ToolConversationRound>::new();
        let system_prompt = self.state.effective_system_prompt(None);
        for iteration in 0..MAX_TOOL_LOOP_ITERATIONS {
            self.turn_state = TurnState::ModelRequestActive;
            self.status_note = Some(if iteration == 0 {
                format!("awaiting {} tool-aware response", resolved.provider_id())
            } else {
                format!(
                    "continuing tool loop {}/{}",
                    iteration + 1,
                    MAX_TOOL_LOOP_ITERATIONS
                )
            });
            self.needs_render = true;
            on_progress(self)?;

            let response = runtime.complete_with_tool_use(
                resolved,
                &ToolUseRequest {
                    prompt: request_prompt.to_string(),
                    system_prompt: system_prompt.clone(),
                    max_output_tokens: None,
                    temperature: None,
                    tools: provider_tools.clone(),
                    rounds: rounds.clone(),
                    effort_level: self.state.effort_level.clone(),
                },
            )?;

            match response {
                ToolUseResponse::Final(response) => {
                    self.state.pending_tool_approval = None;
                    self.state.record_cost_usage(response.usage, None);
                    let assistant_message = append_contextual_message(
                        &mut self.state,
                        MessagePayload::AssistantText {
                            content: response.output_text,
                        },
                    )?;
                    staged_messages.push(assistant_message);
                    self.persist_messages(&staged_messages)?;
                    self.status_note = Some(if rounds.is_empty() {
                        "model response recorded".into()
                    } else {
                        "tool loop response recorded".into()
                    });
                    return Ok(());
                }
                ToolUseResponse::ToolCalls(batch) => {
                    self.state.record_cost_usage(batch.usage, None);
                    let local_calls = batch
                        .calls
                        .iter()
                        .map(|call| LocalToolCall {
                            provider_call: call.clone(),
                            use_id: ToolUseId::new(),
                        })
                        .collect::<Vec<_>>();

                    if let Some(text) = batch
                        .assistant_text
                        .as_deref()
                        .filter(|text| !text.trim().is_empty())
                    {
                        staged_messages.push(append_contextual_message(
                            &mut self.state,
                            MessagePayload::AssistantText {
                                content: text.to_string(),
                            },
                        )?);
                    }
                    for call in &local_calls {
                        staged_messages.push(append_contextual_message(
                            &mut self.state,
                            MessagePayload::AssistantToolUse {
                                tool: call.provider_call.tool_name.clone(),
                                use_id: call.use_id,
                                input: call.provider_call.arguments.clone(),
                            },
                        )?);
                    }
                    self.persist_messages(&staged_messages)?;
                    staged_messages.clear();
                    self.needs_render = true;
                    on_progress(self)?;

                    let mut round = ToolConversationRound {
                        assistant_text: batch.assistant_text.filter(|text| !text.trim().is_empty()),
                        calls: batch.calls,
                        results: Vec::new(),
                    };
                    for (index, call) in local_calls.iter().cloned().enumerate() {
                        let outcome = self.execute_tool_call(
                            &registry,
                            &tool_context,
                            &call.provider_call,
                            call.use_id,
                            on_progress,
                        )?;
                        match outcome {
                            ToolExecutionOutcome::Completed(result) => round.results.push(result),
                            ToolExecutionOutcome::Paused { reason } => {
                                self.state.pending_tool_approval = Some(PendingToolApprovalState {
                                    request_prompt: request_prompt.to_string(),
                                    rounds: rounds.iter().map(pending_round_from_runtime).collect(),
                                    current_round: pending_round_from_runtime(&round),
                                    pending_call: pending_local_call_from_runtime(&call),
                                    remaining_calls: local_calls
                                        .iter()
                                        .skip(index + 1)
                                        .map(pending_local_call_from_runtime)
                                        .collect(),
                                    reason,
                                });
                                self.persist_state_snapshot()?;
                                return Ok(());
                            }
                        }
                    }
                    rounds.push(round);
                }
            }
        }

        self.state.pending_tool_approval = None;
        let limit_message = append_contextual_message(
            &mut self.state,
            MessagePayload::System {
                content: format!(
                    "tool loop stopped after {MAX_TOOL_LOOP_ITERATIONS} iterations without a final assistant response"
                ),
            },
        )?;
        self.persist_messages(&[limit_message])?;
        self.status_note = Some("tool loop hit iteration limit".into());
        Ok(())
    }

    pub(super) fn resolve_prompt_execution(
        &self,
        runtime: &ProviderRuntime,
    ) -> Result<wonder_of_u_agent::ResolvedProviderExecution> {
        if self.storage_dir.is_some() || !self.state.fast_mode {
            return runtime
                .resolve_execution(self.storage_dir.as_deref(), ProviderSelection::default());
        }

        let resolver = ProviderResolver::builtin();
        let fallback_provider = resolver.load_report(None)?.provider;
        let provider = self.state.provider.clone().or(fallback_provider);
        let selection = provider
            .as_deref()
            .and_then(|provider_id| {
                resolver
                    .registry()
                    .get(provider_id)
                    .map(|descriptor| (provider_id.to_string(), descriptor))
            })
            .and_then(|(provider_id, descriptor)| {
                descriptor
                    .preferred_fast_model()
                    .map(|model| ProviderSelection::new(Some(provider_id), Some(model.id.clone())))
            })
            .unwrap_or_default();
        runtime.resolve_execution(None, selection)
    }

    #[cfg(test)]
    pub(super) fn execute_slash_command(&mut self, input: &str) -> Result<()> {
        self.execute_slash_command_with(input, &mut |_| Ok(()))
    }

    pub(super) fn execute_slash_command_with<F>(
        &mut self,
        input: &str,
        before_blocking: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let invocation = parse_slash_command(input)
            .ok_or_else(|| WonderError::validation("invalid slash command"))?;
        let context = self.command_context();
        let query = CommandQuery::from(&context);
        let command = match self.registry.resolve_enabled(&invocation.name, &query) {
            Some(command) => command,
            None => commands::resolve_dynamic_command(
                &context.cwd,
                self.storage_dir.as_deref(),
                &invocation.name,
                &query,
            )?
            .ok_or_else(|| WonderError::not_found("command", invocation.name.clone()))?,
        };
        let output = block_on(command.execute(context, invocation.clone()))?;
        let (text, exit_requested) = command_output_text(output);
        if self.persistence.persisted {
            self.restore_current_session()?;
        }
        self.apply_inline_command_hints(input);
        self.apply_command_output_hints(text.as_deref());
        let had_queued_commands = !self.state.queued_commands.is_empty();
        let view_action = parse_view_action_hint(text.as_deref());
        if view_action.is_none()
            && self.pending_external_editor.is_none()
            && self.pending_model_picker.is_none()
            && self.pending_permission_picker.is_none()
            && self.pending_memory_picker.is_none()
            && self.pending_tag_removal.is_none()
            && self.pending_theme_picker.is_none()
        {
            self.record_command_message(input, text.as_deref())?;
        }
        let _ = self.refresh_runtime_state()?;
        self.drain_queued_commands(before_blocking)?;
        self.exit_requested |= exit_requested;
        if exit_requested {
            self.status_note = Some("exit requested".into());
        } else if let Some(view_action) = view_action {
            self.status_note = Some(match view_action {
                ViewActionHint::Clear => "conversation cleared".into(),
                ViewActionHint::Compact => "conversation compacted".into(),
            });
        } else if self.dialog.as_ref().is_some_and(|dialog| {
            dialog.title == "Context Usage"
                || dialog.title == "Activity Stats"
                || dialog.title == "Usage"
                || dialog.title == "Theme"
                || dialog.title == "Color"
                || dialog.title == "Fast"
                || dialog.title == "Brief"
                || dialog.title == "Effort"
                || dialog.title == "Feedback"
                || dialog.title == "Insights"
                || dialog.title == "Upgrade"
                || dialog.title == "Desktop"
                || dialog.title == "Mobile"
                || dialog.title == "Chrome"
                || dialog.title == "IDE Integration"
                || dialog.title == "Background Tasks"
                || dialog.title == "Release Notes"
                || dialog.title == "Version"
                || dialog.title == "Hooks"
                || dialog.title == "Keybindings"
                || dialog.title == "Privacy Settings"
                || dialog.title == "Terminal Setup"
        }) {
        } else if parse_vim_toggle_hint(text.as_deref().unwrap_or_default())
            || parse_vim_mode_hint(text.as_deref().unwrap_or_default()).is_some()
        {
            self.status_note = Some(match self.vim.mode() {
                VimMode::Insert => "vim insert".into(),
                VimMode::Normal => "vim normal".into(),
            });
        } else if parse_insights_hint(text.as_deref().unwrap_or_default()) {
            self.status_note = Some("insights queued".into());
        } else if let Some(enabled) = parse_fast_mode_hint(text.as_deref().unwrap_or_default()) {
            self.status_note = Some(format!("fast {}", if enabled { "on" } else { "off" }));
        } else if let Some(enabled) = parse_brief_mode_hint(text.as_deref().unwrap_or_default()) {
            self.status_note = Some(format!("brief {}", if enabled { "on" } else { "off" }));
        } else if let Some(effort) = parse_effort_hint(text.as_deref().unwrap_or_default()) {
            self.status_note = Some(format!("effort {effort}"));
        } else if let Some(color) = parse_session_color_hint(text.as_deref().unwrap_or_default()) {
            self.status_note = Some(format!("color {color}"));
        } else if self.pending_permission_picker.is_some()
            || self.pending_memory_picker.is_some()
            || self.pending_tag_removal.is_some()
            || self.pending_theme_picker.is_some()
            || self.pending_model_picker.is_some()
        {
        } else if self.pending_external_editor.is_some() {
            self.status_note = Some("opening file in editor".into());
        } else if !had_queued_commands {
            self.status_note = Some("slash command recorded".into());
        }
        if invocation.name == "tasks"
            && invocation.args.is_empty()
            && self.state.background_tasks.is_empty()
        {
            self.dismiss_dialog();
            self.dialog = Some(DialogView::notice(
                "Background Tasks",
                ["No background tasks running"],
            ));
            self.task_notice_ttl = Some(TASK_NOTICE_TTL);
            self.status_note = Some("no background tasks running".into());
            self.push_notification(
                "background-tasks",
                NotificationSeverity::Info,
                "Background Tasks",
                ["No background tasks running"],
                Some(SHELL_NOTIFICATION_TTL),
                true,
            );
        }
        self.needs_render = true;
        Ok(())
    }

    pub(super) fn apply_inline_command_hints(&mut self, input: &str) {
        let Ok(tokens) = shell_words::split(input.trim_start_matches('/')) else {
            return;
        };
        if tokens.first().map(String::as_str) != Some("model")
            || tokens.get(1).map(String::as_str) != Some("set")
        {
            return;
        }

        let mut provider = None;
        let mut model = None;
        let mut index = 2usize;
        while index < tokens.len() {
            match tokens[index].as_str() {
                "--provider" if index + 1 < tokens.len() => {
                    provider = Some(tokens[index + 1].clone());
                    index += 2;
                }
                "--model" if index + 1 < tokens.len() => {
                    model = Some(tokens[index + 1].clone());
                    index += 2;
                }
                selection if !selection.starts_with('-') => {
                    if let Some((selection_provider, selection_model)) = selection.split_once(':') {
                        provider.get_or_insert_with(|| selection_provider.to_string());
                        model.get_or_insert_with(|| selection_model.to_string());
                    }
                    index += 1;
                }
                _ => {
                    index += 1;
                }
            }
        }

        let Some(provider) = provider else {
            return;
        };
        let resolver = ProviderResolver::builtin();
        let Some(descriptor) = resolver.registry().get(&provider) else {
            return;
        };
        let auth = match descriptor.auth_kind {
            wonder_of_u_core::AuthMaterialKind::None => AuthState::not_required(),
            wonder_of_u_core::AuthMaterialKind::ApiKey => AuthState::missing(descriptor.auth_kind),
            wonder_of_u_core::AuthMaterialKind::OAuth => AuthState::pending(
                descriptor.auth_kind,
                None,
                "run `wonder-of-u login --provider copilot`",
            ),
        };
        self.state.set_provider_context(
            Some(provider),
            Some(model.unwrap_or_else(|| descriptor.default_model.clone())),
            auth,
        );
    }

    pub(super) fn apply_command_output_hints(&mut self, text: Option<&str>) {
        let Some(text) = text else {
            return;
        };
        if let Some(directory) = parse_additional_working_directory_hint(text) {
            self.state.add_additional_working_directory(directory);
        }
        if parse_vim_toggle_hint(text) {
            self.vim = VimState::new(match self.vim.mode() {
                VimMode::Insert => VimMode::Normal,
                VimMode::Normal => VimMode::Insert,
            });
        }
        if let Some(mode) = parse_vim_mode_hint(text) {
            self.vim = VimState::new(mode);
        }
        if let Some(picker) = parse_permission_picker_state(text) {
            self.open_permission_picker(picker);
        } else {
            self.pending_permission_picker = None;
        }
        if let Some(picker) = parse_memory_picker_state(text) {
            self.open_memory_picker(picker);
        } else {
            self.pending_memory_picker = None;
        }
        if let Some(tag) = parse_tag_remove_confirmation(text) {
            self.open_tag_removal_confirmation(TagRemovalState {
                original_input: format!("/tag {tag}"),
                tag,
            });
        } else {
            self.pending_tag_removal = None;
        }
        if let Some(picker) = parse_theme_picker_state(text) {
            self.open_theme_picker(picker);
        } else {
            self.pending_theme_picker = None;
        }
        if let Some(picker) = parse_model_picker_state(text) {
            self.open_model_picker(picker);
        } else {
            self.pending_model_picker = None;
        }
        if let Some(theme) = parse_theme_hint(text) {
            self.state
                .set_theme((theme != "default").then(|| theme.to_string()));
        }
        if let Some(color) = parse_session_color_hint(text) {
            self.state
                .set_session_color((color != "default").then(|| color.to_string()));
        }
        if let Some(enabled) = parse_fast_mode_hint(text) {
            self.state.set_fast_mode(enabled);
        }
        if let Some(enabled) = parse_brief_mode_hint(text) {
            self.state.set_brief_mode(enabled);
        }
        if let Some(effort) = parse_effort_hint(text) {
            self.state
                .set_effort_level((effort != "auto").then(|| effort.to_string()));
        }
        if let Some(tags) = parse_session_tags_hint(text) {
            self.state.set_session_tags(tags);
        }
        self.pending_external_editor = parse_external_editor_request(text, &self.state.session.cwd);
        if let Some((title, body, note)) = parse_known_notice(text) {
            self.dialog = Some(DialogView::notice(title.clone(), body.clone()));
            self.status_note = Some(note.into());
            self.push_notification(
                format!("notice:{}", title.to_ascii_lowercase().replace(' ', "-")),
                NotificationSeverity::Info,
                title,
                body,
                Some(SHELL_NOTIFICATION_TTL),
                false,
            );
            self.needs_render = true;
        } else if self.state.input_mode == InputMode::Prompt
            && !self.has_picker_overlay()
            && self
                .dialog
                .as_ref()
                .is_some_and(|dialog| dialog.kind() == wonder_of_u_tui::DialogKind::Notice)
        {
            self.dialog = None;
            self.needs_render = true;
        }
        for queued_prompt in text
            .lines()
            .filter_map(|line| line.strip_prefix("enqueue_prompt="))
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            self.state
                .queue_command(queued_prompt.to_string(), QueuePlacement::Now);
        }
        if let Some(mode) = text
            .lines()
            .find_map(|line| line.strip_prefix("permission_mode="))
            .and_then(parse_permission_mode_hint)
        {
            self.state.permission_mode = mode;
        }
        let Some(selection) = text
            .lines()
            .find_map(|line| line.strip_prefix("provider_selection="))
        else {
            return;
        };

        let (provider, model) = if selection == "none" {
            (None, None)
        } else {
            match selection.split_once(':') {
                Some((provider, model)) => (Some(provider.to_string()), Some(model.to_string())),
                None => return,
            }
        };
        let resolver = ProviderResolver::builtin();
        let auth = match text
            .lines()
            .find_map(|line| line.strip_prefix("auth_status="))
        {
            Some("not_required") => AuthState::not_required(),
            Some("missing") => provider
                .as_deref()
                .and_then(|provider_id| resolver.registry().get(provider_id))
                .map(|provider| AuthState::missing(provider.auth_kind))
                .unwrap_or_default(),
            Some("pending") => provider
                .as_deref()
                .and_then(|provider_id| resolver.registry().get(provider_id))
                .map(|provider| {
                    AuthState::pending(provider.auth_kind, None, "reported by slash command output")
                })
                .unwrap_or_default(),
            _ => self.state.auth.clone(),
        };

        self.state.set_provider_context(provider, model, auth);
    }

    pub(super) fn drain_queued_commands<F>(&mut self, before_blocking: &mut F) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let mut drained = 0usize;
        while let Some(queued) = self.state.queued_commands.pop_front() {
            drained += 1;
            if drained > 8 {
                self.status_note = Some("queued command limit reached".into());
                self.needs_render = true;
                break;
            }
            self.persist_state_snapshot()?;
            let command = queued.command.trim().to_string();
            if command.is_empty() {
                continue;
            }
            if command.starts_with('/') {
                self.execute_slash_command_with(&command, before_blocking)?;
            } else {
                self.turn_state = TurnState::ModelRequestActive;
                self.status_note = Some("processing queued prompt".into());
                self.needs_render = true;
                before_blocking(self)?;
                self.execute_prompt_submission(&command, before_blocking)?;
                self.turn_state = TurnState::Completed;
            }
        }
        Ok(())
    }

    pub(super) fn record_command_message(
        &mut self,
        input: &str,
        output: Option<&str>,
    ) -> Result<()> {
        let message = append_contextual_message(
            &mut self.state,
            MessagePayload::Command {
                input: input.into(),
                output: output.map(ToString::to_string),
            },
        )?;
        persist_messages_and_state(
            self.storage_dir.as_deref(),
            &self.state,
            &mut self.persistence,
            &[message],
        )?;
        self.notify_transcript_changed();
        Ok(())
    }

    pub(super) fn persist_messages(&mut self, messages: &[MessageEnvelope]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        persist_messages_and_state(
            self.storage_dir.as_deref(),
            &self.state,
            &mut self.persistence,
            messages,
        )?;
        // Keep scroll state consistent: follow-tail stays pinned; scrolled-up
        // mode preserves the viewport relative to the bottom of the transcript.
        self.notify_transcript_changed();
        Ok(())
    }

    pub(super) fn tool_context(&self) -> ToolContext {
        ToolContext {
            session_id: self.state.session.id,
            cwd: self.state.session.cwd.clone(),
            permission_mode: self.state.permission_mode,
            additional_working_directories: self.state.additional_working_directories.clone(),
            permission_rules: Vec::new(),
            features: self.state.features.clone(),
        }
    }

    pub(super) fn execute_tool_call<F>(
        &mut self,
        registry: &wonder_of_u_core::ToolRegistry,
        context: &ToolContext,
        call: &ProviderToolCall,
        use_id: ToolUseId,
        on_progress: &mut F,
    ) -> Result<ToolExecutionOutcome>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let query = ToolQuery::from(context);
        if let Some(tool) = registry.resolve_enabled(&call.tool_name, &query) {
            if let Err(error) = tool.validate_input(&call.arguments) {
                return self
                    .finalize_tool_result(
                        Vec::new(),
                        call,
                        ToolResult::failure(
                            use_id,
                            format!("tool input validation failed: {error}"),
                        ),
                        on_progress,
                    )
                    .map(ToolExecutionOutcome::Completed);
            }
            match tool.permission_decision(context, &call.arguments) {
                PermissionDecision::Allow { .. } => self
                    .run_tool_call(tool, context, call, use_id, on_progress)
                    .map(ToolExecutionOutcome::Completed),
                other @ PermissionDecision::Ask { .. } => {
                    let spec = tool.spec();
                    self.turn_state = TurnState::ToolPermissionPending;
                    self.state.input_mode = InputMode::PermissionPending;
                    let reason = other.reason().to_string();
                    self.dialog = Some(permission_dialog_for_tool_call(
                        &call.tool_name,
                        &call.arguments,
                        spec.read_only,
                        spec.destructive,
                        reason.clone(),
                    ));
                    let permission_message = append_contextual_message(
                        &mut self.state,
                        MessagePayload::Permission {
                            tool: call.tool_name.clone(),
                            decision: "ask".into(),
                            reason: reason.clone(),
                        },
                    )?;
                    self.persist_messages(&[permission_message])?;
                    self.status_note = Some(permission_required_status(&call.tool_name));
                    self.needs_render = true;
                    on_progress(self)?;
                    Ok(ToolExecutionOutcome::Paused { reason })
                }
                other @ PermissionDecision::Deny { .. } => {
                    let reason = other.reason().to_string();
                    self.turn_state = TurnState::ToolPermissionPending;
                    self.state.input_mode = InputMode::PermissionPending;
                    self.dialog = Some(DialogView::notice(
                        "Permission denied",
                        [
                            format!("Tool `{}` was denied.", call.tool_name),
                            reason.clone(),
                        ],
                    ));
                    let permission_message = append_contextual_message(
                        &mut self.state,
                        MessagePayload::Permission {
                            tool: call.tool_name.clone(),
                            decision: permission_decision_label(&other).into(),
                            reason: reason.clone(),
                        },
                    )?;
                    self.finalize_tool_result(
                        vec![permission_message],
                        call,
                        ToolResult::failure(use_id, format!("tool execution denied: {reason}")),
                        on_progress,
                    )
                    .map(ToolExecutionOutcome::Completed)
                }
            }
        } else {
            self.finalize_tool_result(
                Vec::new(),
                call,
                ToolResult::failure(
                    use_id,
                    format!("tool `{}` is not available in this session", call.tool_name),
                ),
                on_progress,
            )
            .map(ToolExecutionOutcome::Completed)
        }
    }

    pub(super) fn run_tool_call<F>(
        &mut self,
        tool: std::sync::Arc<dyn wonder_of_u_core::Tool>,
        context: &ToolContext,
        call: &ProviderToolCall,
        use_id: ToolUseId,
        on_progress: &mut F,
    ) -> Result<ProviderToolResultMessage>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        self.turn_state = TurnState::ToolExecuting;
        self.state.input_mode = InputMode::Bash;
        self.status_note = Some(format!("running tool {}", call.tool_name));
        self.needs_render = true;
        on_progress(self)?;
        let result = match block_on(tool.execute(context.clone(), use_id, call.arguments.clone())) {
            Ok(result) => result,
            Err(error) => ToolResult::failure(use_id, format!("tool execution failed: {error}")),
        };
        self.finalize_tool_result(Vec::new(), call, result, on_progress)
    }

    pub(super) fn finalize_tool_result<F>(
        &mut self,
        mut messages: Vec<MessageEnvelope>,
        call: &ProviderToolCall,
        result: ToolResult,
        on_progress: &mut F,
    ) -> Result<ProviderToolResultMessage>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        messages.push(append_contextual_message(
            &mut self.state,
            MessagePayload::ToolResult {
                tool: call.tool_name.clone(),
                use_id: result.use_id,
                success: result.success,
                content: result.content.clone(),
            },
        )?);
        self.persist_messages(&messages)?;
        self.needs_render = true;
        on_progress(self)?;
        Ok(ProviderToolResultMessage {
            call_id: call.call_id.clone(),
            content: render_provider_tool_result(&result),
        })
    }

    pub(super) fn restore_current_session(&mut self) -> Result<()> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        let restored = TranscriptStore::new(storage_dir).restore_session(self.state.session.id)?;
        self.persistence.transcript_message_count = restored.transcript.messages.len();
        self.persistence.transcript_warning_count = restored.transcript.warnings.len();
        self.persistence.persisted = true;
        self.state = restored.state;
        self.state.session.entrypoint = Some("tui".into());
        self.state.session.app_version = Some(env!("CARGO_PKG_VERSION").into());
        self.rebuild_ephemeral_state();
        // Messages have been fully replaced; bring scroll state in sync.
        self.notify_transcript_changed();
        Ok(())
    }

    pub(super) fn refresh_runtime_state(&mut self) -> Result<bool> {
        let resolver = ProviderResolver::builtin();
        let selection =
            ProviderSelection::new(self.state.provider.clone(), self.state.model.clone());
        let provider_report = if selection.provider.is_some() || selection.model.is_some() {
            resolver
                .load_report_for_selection(self.storage_dir.as_deref(), &selection)
                .or_else(|_| resolver.load_report(self.storage_dir.as_deref()))?
        } else {
            resolver.load_report(self.storage_dir.as_deref())?
        };
        let mut changed = false;
        if self.state.provider != provider_report.provider
            || self.state.model != provider_report.model
            || self.state.auth != provider_report.auth
        {
            self.state.set_provider_context(
                provider_report.provider,
                provider_report.model,
                provider_report.auth,
            );
            changed = true;
        }

        let previous_tasks = self.state.background_tasks.clone();
        let next_tasks = match self.storage_dir.as_deref() {
            Some(storage_dir) => TaskStore::new(storage_dir)
                .list_tasks()?
                .into_iter()
                .map(|task| (task.id, task))
                .collect::<BTreeMap<_, _>>(),
            None => BTreeMap::new(),
        };
        let effective_tasks = if next_tasks.is_empty() && !self.state.background_tasks.is_empty() {
            self.state.background_tasks.clone()
        } else {
            next_tasks
        };
        if self.state.background_tasks != effective_tasks {
            if let Some(task) = next_task_notification(&previous_tasks, &effective_tasks) {
                self.dismiss_dialog();
                self.dialog = Some(DialogView::notice(
                    "Task update",
                    task_notification_lines(task),
                ));
                self.state.input_mode = InputMode::TaskNotification;
                self.status_note = Some(format!(
                    "task {}: {}",
                    task_status_label(task.status),
                    task.description
                ));
                self.task_notice_ttl = Some(TASK_NOTICE_TTL);
                self.push_notification(
                    format!("task:{}", task.id),
                    task_notification_severity(task.status),
                    "Task update",
                    task_notification_lines(task),
                    Some(SHELL_NOTIFICATION_TTL),
                    true,
                );
            }
            self.state.background_tasks = effective_tasks;
            changed = true;
        }

        Ok(changed)
    }

    pub(super) fn persist_state_snapshot(&self) -> Result<()> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        let store = TranscriptStore::new(storage_dir);
        if self.persistence.transcript_message_count == 0 {
            store.ensure_layout()?;
            let _ = OpenOptions::new()
                .create(true)
                .append(true)
                .open(store.paths().transcript_path(self.state.session.id))?;
        }
        persist_prompt_state(&store, &self.state, &self.persistence)
    }

    pub(super) fn command_context(&self) -> CommandContext {
        CommandContext {
            session_id: self.state.session.id,
            cwd: self.state.session.cwd.clone(),
            features: self.state.features.clone(),
            authenticated: self.state.provider_readiness() == ProviderReadiness::Ready,
            interactive: true,
            permission_mode: self.state.permission_mode,
            theme: self.state.theme.clone(),
            session_color: self.state.session_color.clone(),
            effort_level: self.state.effort_level.clone(),
            brief_mode: self.state.brief_mode,
            fast_mode: self.state.fast_mode,
            session_tags: self.state.session.tags.clone(),
            additional_working_directories: self.state.additional_working_directories.clone(),
        }
    }

    pub(super) fn view(&self) -> ShellView {
        let history_entries = prompt_history_entries(&self.state.messages);
        let prompt = self
            .history_search
            .as_ref()
            .and_then(|search| current_history_search_match(search, &history_entries))
            .map_or_else(|| self.prompt.text(), ToString::to_string);
        let mut view = ShellView::from_app_state(&self.state, prompt);
        view.history_search = self
            .history_search
            .as_ref()
            .map(|search| HistorySearchView {
                query: search.query.text(),
                match_text: current_history_search_match(search, &history_entries)
                    .map(ToString::to_string),
                match_index: search.cursor,
                match_total: search.matches.len(),
            });
        let mut status = session_status_text(&self.state);
        status.push_str(" | turn=");
        status.push_str(turn_state_label(self.turn_state));
        if let Some(note) = &self.status_note {
            status.push_str(" | ");
            status.push_str(note);
        }
        view.status = status;
        view.loading = matches!(
            self.turn_state,
            TurnState::ModelRequestActive
                | TurnState::CommandQueued
                | TurnState::ToolPermissionPending
        );
        view.loading_verb = match self.turn_state {
            TurnState::ModelRequestActive => Some("thinking".to_string()),
            TurnState::CommandQueued => Some("running".to_string()),
            TurnState::ToolPermissionPending => Some("waiting".to_string()),
            _ => None,
        };
        if self.prompt.is_empty() && !matches!(self.turn_state, TurnState::ModelRequestActive) {
            view.status = "  / commands  ·  ↑ history  ·  ⌃R search".to_string();
        }

        let mut footer = session_footer_text(&self.state);
        footer.push_str(if self.persistence.persisted {
            " | storage=persisted"
        } else {
            " | storage=memory"
        });
        footer.push_str(" | ");
        footer.push_str(runtime_label(
            self.state.provider.as_deref(),
            self.state.model.as_deref(),
        ));
        footer.push_str(" | vim:");
        footer.push_str(match self.vim.mode() {
            VimMode::Insert => "insert",
            VimMode::Normal => "normal",
        });
        footer.push_str(" | theme:");
        footer.push_str(match self.state.theme.as_deref() {
            Some("midnight") => "midnight",
            Some("light") => "light",
            _ => "default",
        });
        footer.push_str(" | color:");
        footer.push_str(match self.state.session_color.as_deref() {
            Some("red") => "red",
            Some("blue") => "blue",
            Some("green") => "green",
            Some("yellow") => "yellow",
            Some("purple") => "purple",
            Some("orange") => "orange",
            Some("pink") => "pink",
            Some("cyan") => "cyan",
            _ => "default",
        });
        footer.push_str(" | effort:");
        footer.push_str(match self.state.effort_level.as_deref() {
            Some("low") => "low",
            Some("medium") => "medium",
            Some("high") => "high",
            Some("max") => "max",
            _ => "auto",
        });
        footer.push_str(" | fast:");
        footer.push_str(if self.state.fast_mode { "on" } else { "off" });
        footer.push_str(" | brief:");
        footer.push_str(if self.state.brief_mode { "on" } else { "off" });
        footer.push_str(" | ⌃C exit | ? help");
        view.footer = footer;
        let picker_list = self.current_picker_list_view();
        view.dialog = if picker_list.is_some() {
            None
        } else {
            self.dialog.clone()
        };
        view.picker_view = None;
        view.picker_list = picker_list;
        view.notifications = self.notifications.view(3);
        view.slash_suggestions = self.active_suggestions.as_ref().map(|state| {
            let filtered = state.filtered();
            let entries = filtered
                .iter()
                .enumerate()
                .map(|(i, s)| SlashSuggestionEntry {
                    display: s.display_text.clone(),
                    description: s.description.clone().unwrap_or_default(),
                    selected: i == state.selected_index.min(filtered.len().saturating_sub(1)),
                })
                .collect();
            SlashSuggestionsOverlay { entries }
        });
        view
    }

    pub(super) fn push_notification(
        &mut self,
        key: impl Into<String>,
        severity: NotificationSeverity,
        title: impl Into<String>,
        body: impl IntoIterator<Item = impl Into<String>>,
        ttl: Option<u32>,
        focus: bool,
    ) {
        let lifetime = ttl.map_or(
            NotificationLifetime::Persistent,
            NotificationLifetime::Ticks,
        );
        self.notifications.push(
            NotificationInput::new(key, title, body)
                .severity(severity)
                .lifetime(lifetime)
                .focused(focus),
        );
        self.needs_render = true;
    }

    pub(super) fn open_permission_picker(&mut self, mut picker: PermissionPickerState) {
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
        self.pending_memory_picker = None;
        self.pending_tag_removal = None;
        self.pending_theme_picker = None;
        self.pending_model_picker = None;
        self.pending_permission_picker = Some(picker);
        self.status_note = None;
        self.refresh_permission_picker_dialog();
    }

    pub(super) fn refresh_permission_picker_dialog(&mut self) {
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
        });
        self.status_note = Some(picker_status_note("permission mode"));
        self.needs_render = true;
    }

    pub(super) fn step_permission_picker(&mut self, delta: isize) {
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

    pub(super) fn edit_permission_picker_query(&mut self, resolved: ResolvedKey) -> bool {
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

    pub(super) fn complete_permission_picker(&mut self) -> Result<()> {
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

    pub(super) fn cancel_permission_picker(&mut self) -> Result<()> {
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

    pub(super) fn open_memory_picker(&mut self, mut picker: MemoryPickerState) {
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
        self.pending_permission_picker = None;
        self.pending_tag_removal = None;
        self.pending_theme_picker = None;
        self.pending_model_picker = None;
        self.pending_memory_picker = Some(picker);
        self.status_note = None;
        self.refresh_memory_picker_dialog();
    }

    pub(super) fn refresh_memory_picker_dialog(&mut self) {
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
        });
        self.status_note = Some(picker_status_note("memory"));
        self.needs_render = true;
    }

    pub(super) fn step_memory_picker(&mut self, delta: isize) {
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

    pub(super) fn edit_memory_picker_query(&mut self, resolved: ResolvedKey) -> bool {
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

    pub(super) fn complete_memory_picker(&mut self) -> Result<()> {
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

    pub(super) fn cancel_memory_picker(&mut self) -> Result<()> {
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

    pub(super) fn open_tag_removal_confirmation(&mut self, pending: TagRemovalState) {
        self.pending_permission_picker = None;
        self.pending_memory_picker = None;
        self.pending_theme_picker = None;
        self.pending_model_picker = None;
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

    pub(super) fn handle_tag_removal_key(
        &mut self,
        _key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        if matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline))) {
            return self.complete_tag_removal();
        }
        self.cancel_tag_removal()
    }

    pub(super) fn complete_tag_removal(&mut self) -> Result<()> {
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

    pub(super) fn cancel_tag_removal(&mut self) -> Result<()> {
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

    pub(super) fn open_theme_picker(&mut self, mut picker: ThemePickerState) {
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
        self.pending_permission_picker = None;
        self.pending_memory_picker = None;
        self.pending_tag_removal = None;
        self.pending_model_picker = None;
        self.pending_theme_picker = Some(picker);
        self.status_note = None;
        self.refresh_theme_picker_dialog();
    }

    pub(super) fn refresh_theme_picker_dialog(&mut self) {
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
        });
        self.status_note = Some(picker_status_note("theme picker"));
        self.needs_render = true;
    }

    pub(super) fn step_theme_picker(&mut self, delta: isize) {
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

    pub(super) fn edit_theme_picker_query(&mut self, resolved: ResolvedKey) -> bool {
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

    pub(super) fn complete_theme_picker(&mut self) -> Result<()> {
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

    pub(super) fn cancel_theme_picker(&mut self) -> Result<()> {
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

    pub(super) fn open_model_picker(&mut self, mut picker: ModelPickerState) {
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
        self.pending_permission_picker = None;
        self.pending_memory_picker = None;
        self.pending_tag_removal = None;
        self.pending_theme_picker = None;
        self.pending_model_picker = Some(picker);
        self.status_note = None;
        self.refresh_model_picker_dialog();
    }

    pub(super) fn refresh_model_picker_dialog(&mut self) {
        let Some(picker) = &self.pending_model_picker else {
            return;
        };
        let filtered = filtered_picker_indices(&picker.query, &picker.options, |option| {
            format!(
                "{} {} {} {} {}",
                option.provider,
                option.provider_display,
                option.model,
                option.model_display,
                option.auth
            )
        });
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
        });
        self.status_note = Some(picker_status_note("model picker"));
        self.needs_render = true;
    }

    pub(super) fn step_model_picker(&mut self, delta: isize) {
        let Some(picker) = &mut self.pending_model_picker else {
            return;
        };
        picker.selected_index = step_picker_selection(
            picker.selected_index,
            delta,
            &filtered_picker_indices(&picker.query, &picker.options, |option| {
                format!(
                    "{} {} {} {} {}",
                    option.provider,
                    option.provider_display,
                    option.model,
                    option.model_display,
                    option.auth
                )
            }),
        );
        self.refresh_model_picker_dialog();
    }

    pub(super) fn edit_model_picker_query(&mut self, resolved: ResolvedKey) -> bool {
        let Some(picker) = &mut self.pending_model_picker else {
            return false;
        };
        if !apply_picker_query_edit(&mut picker.query, resolved) {
            return false;
        }
        sync_picker_selection(
            &mut picker.selected_index,
            &filtered_picker_indices(&picker.query, &picker.options, |option| {
                format!(
                    "{} {} {} {} {}",
                    option.provider,
                    option.provider_display,
                    option.model,
                    option.model_display,
                    option.auth
                )
            }),
        );
        self.refresh_model_picker_dialog();
        true
    }

    pub(super) fn complete_model_picker(&mut self) -> Result<()> {
        let Some(picker) = self.pending_model_picker.take() else {
            self.dismiss_dialog();
            return Ok(());
        };
        let filtered = filtered_picker_indices(&picker.query, &picker.options, |option| {
            format!(
                "{} {} {} {} {}",
                option.provider,
                option.provider_display,
                option.model,
                option.model_display,
                option.auth
            )
        });
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

    pub(super) fn cancel_model_picker(&mut self) -> Result<()> {
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

    pub(super) fn dismiss_dialog(&mut self) {
        self.dialog = None;
        self.pending_permission_picker = None;
        self.pending_memory_picker = None;
        self.pending_tag_removal = None;
        self.pending_theme_picker = None;
        self.pending_model_picker = None;
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

    pub(super) fn dismiss_notice_dialog(&mut self) {
        let status = self
            .dialog
            .as_ref()
            .map(|dialog| overlay_closed_status(&dialog.title));
        self.dismiss_dialog();
        if let Some(status) = status {
            self.status_note = Some(status);
        }
    }

    pub(super) fn dismiss_task_notice(&mut self) {
        self.task_notice_ttl = None;
        self.dismiss_dialog();
        self.needs_render = true;
    }

    #[allow(dead_code)]
    pub(super) fn active_overlay(&self) -> ActiveOverlay {
        if self.history_search.is_some() {
            return ActiveOverlay::HistorySearch;
        }
        if self.has_picker_overlay() {
            return ActiveOverlay::Picker;
        }
        if let Some(dialog) = &self.dialog {
            return if dialog.actions.is_empty() {
                ActiveOverlay::NoticeDialog
            } else {
                ActiveOverlay::ConfirmDialog
            };
        }
        ActiveOverlay::None
    }

    pub(super) fn has_picker_overlay(&self) -> bool {
        self.pending_permission_picker.is_some()
            || self.pending_memory_picker.is_some()
            || self.pending_tag_removal.is_some()
            || self.pending_theme_picker.is_some()
            || self.pending_model_picker.is_some()
    }

    pub(super) fn current_picker_list_view(&self) -> Option<PickerListView> {
        const PICKER_HINT: &str = "↑↓ navigate  Tab/Enter select  Esc cancel";

        if let Some(picker) = &self.pending_model_picker {
            let filtered = filtered_picker_indices(&picker.query, &picker.options, |opt| {
                format!(
                    "{} {} {} {} {}",
                    opt.provider, opt.provider_display, opt.model, opt.model_display, opt.auth
                )
            });
            return Some(PickerListView {
                title: "Select Model".into(),
                query: picker.query.text(),
                entries: filtered
                    .into_iter()
                    .filter_map(|index| picker.options.get(index))
                    .map(|opt| {
                        let mut tag = opt.auth.clone();
                        if opt.default {
                            tag.push_str(", default");
                        }
                        if opt.selected {
                            tag.push_str(", current");
                        }
                        PickerListEntry {
                            label: opt.model_display.clone(),
                            description: opt.provider_display.clone(),
                            tag: Some(tag),
                            selected: opt.model == picker.options[picker.selected_index].model
                                && opt.provider == picker.options[picker.selected_index].provider,
                        }
                    })
                    .collect(),
                hint: PICKER_HINT.into(),
            });
        }
        if let Some(picker) = &self.pending_theme_picker {
            let filtered = filtered_picker_indices(&picker.query, &picker.options, |opt| {
                format!("{} {} {}", opt.theme, opt.label, opt.description)
            });
            return Some(PickerListView {
                title: "Select Theme".into(),
                query: picker.query.text(),
                entries: filtered
                    .into_iter()
                    .filter_map(|index| picker.options.get(index))
                    .map(|opt| PickerListEntry {
                        label: opt.label.clone(),
                        description: opt.description.clone(),
                        tag: opt.selected.then(|| "current".into()),
                        selected: opt.theme == picker.options[picker.selected_index].theme,
                    })
                    .collect(),
                hint: PICKER_HINT.into(),
            });
        }
        if let Some(picker) = &self.pending_permission_picker {
            let filtered = filtered_picker_indices(&picker.query, &picker.options, |opt| {
                format!("{} {}", opt.label, opt.description)
            });
            let title = self
                .state
                .pending_tool_approval
                .as_ref()
                .map(|pending| {
                    format!(
                        "Tool: {} — allow?",
                        pending.pending_call.provider_call.tool_name
                    )
                })
                .unwrap_or_else(|| "Allow Tool?".into());
            return Some(PickerListView {
                title,
                query: picker.query.text(),
                entries: filtered
                    .into_iter()
                    .filter_map(|index| picker.options.get(index))
                    .map(|opt| PickerListEntry {
                        label: opt.label.clone(),
                        description: opt.description.clone(),
                        tag: opt.selected.then(|| "current".into()),
                        selected: opt.mode == picker.options[picker.selected_index].mode,
                    })
                    .collect(),
                hint: PICKER_HINT.into(),
            });
        }
        if let Some(picker) = &self.pending_memory_picker {
            let filtered = filtered_picker_indices(&picker.query, &picker.options, |opt| {
                format!("{} {} {}", opt.label, opt.description, opt.path.display())
            });
            return Some(PickerListView {
                title: "Select Memory Target".into(),
                query: picker.query.text(),
                entries: filtered
                    .into_iter()
                    .filter_map(|index| picker.options.get(index))
                    .map(|opt| PickerListEntry {
                        label: opt.label.clone(),
                        description: format!("{} ({})", opt.description, opt.path.display()),
                        tag: opt.selected.then(|| "default".into()),
                        selected: opt.target == picker.options[picker.selected_index].target,
                    })
                    .collect(),
                hint: PICKER_HINT.into(),
            });
        }
        None
    }

    pub(super) fn should_confirm_exit(&self) -> bool {
        !self.prompt.text().trim().is_empty()
            || !self.state.messages.is_empty()
            || !self.state.background_tasks.is_empty()
    }

    pub(super) fn reset_history_recall(&mut self) {
        self.history_search = None;
    }

    pub(super) fn open_or_step_history_search(&mut self) -> Result<()> {
        if self.history_search.is_none() {
            self.history_search = Some(HistorySearchState {
                query: TextBuffer::new(false),
                matches: Vec::new(),
                cursor: 0,
                saved_buffer: self.prompt.clone(),
            });
            self.refresh_history_search();
            return Ok(());
        }

        self.step_history_search(1);
        Ok(())
    }

    pub(super) fn edit_history_search_query_text(&mut self, text: &str) {
        let Some(search) = &mut self.history_search else {
            return;
        };
        search.query.insert_text(text);
        self.refresh_history_search();
    }

    pub(super) fn edit_history_search_query(&mut self, resolved: ResolvedKey) -> bool {
        let Some(search) = &mut self.history_search else {
            return false;
        };
        if !apply_picker_query_edit(&mut search.query, resolved) {
            return false;
        }
        self.refresh_history_search();
        true
    }

    pub(super) fn refresh_history_search(&mut self) {
        let entries = prompt_history_entries(&self.state.messages);
        let Some(search) = &mut self.history_search else {
            return;
        };
        search.matches = filtered_picker_indices(&search.query, &entries, Clone::clone);
        if search.matches.is_empty() {
            search.cursor = 0;
        } else {
            search.cursor = search.cursor.min(search.matches.len().saturating_sub(1));
        }
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some(history_search_status_note(!search.matches.is_empty()));
        self.needs_render = true;
    }

    pub(super) fn step_history_search(&mut self, delta: isize) {
        let Some(search) = &mut self.history_search else {
            return;
        };
        if !search.matches.is_empty() {
            search.cursor =
                (search.cursor as isize + delta).rem_euclid(search.matches.len() as isize) as usize;
        }
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some(history_search_status_note(!search.matches.is_empty()));
        self.needs_render = true;
    }

    pub(super) fn accept_history_search(&mut self) -> Result<()> {
        let Some(search) = self.history_search.take() else {
            return Ok(());
        };
        let entries = prompt_history_entries(&self.state.messages);
        let Some(selection) = current_history_search_match(&search, &entries) else {
            self.history_search = Some(search);
            self.status_note = Some(history_search_status_note(false));
            self.needs_render = true;
            return Ok(());
        };
        self.prompt = TextBuffer::from_text(selection, false);
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some("history search accepted".into());
        self.needs_render = true;
        Ok(())
    }

    pub(super) fn cancel_history_search(&mut self) -> Result<()> {
        let Some(search) = self.history_search.take() else {
            return Ok(());
        };
        self.prompt = search.saved_buffer;
        self.turn_state = if self.prompt.text().trim().is_empty() {
            if self.state.messages.is_empty() && self.state.background_tasks.is_empty() {
                TurnState::Idle
            } else {
                TurnState::Completed
            }
        } else {
            TurnState::EditingInput
        };
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some("history search cancelled".into());
        self.needs_render = true;
        Ok(())
    }

    pub(super) fn history_search_has_match(&self) -> bool {
        self.history_search
            .as_ref()
            .is_some_and(|search| !search.matches.is_empty())
    }

    pub(super) fn rebuild_ephemeral_state(&mut self) {
        self.history_search = None;
        self.dialog = None;
        self.pending_permission_picker = None;
        self.pending_memory_picker = None;
        self.pending_tag_removal = None;
        self.pending_theme_picker = None;
        self.pending_model_picker = None;
        self.pending_external_editor = None;
        match self.state.input_mode {
            InputMode::Prompt => {
                self.turn_state = if self.prompt.text().trim().is_empty() {
                    if self.state.messages.is_empty() && self.state.background_tasks.is_empty() {
                        TurnState::Idle
                    } else {
                        TurnState::Completed
                    }
                } else {
                    TurnState::EditingInput
                };
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

    pub(super) fn prompt_cursor(&self, width: u16, height: u16) -> (u16, u16) {
        if let Some(search) = &self.history_search {
            let view = HistorySearchView {
                query: search.query.text(),
                match_text: self
                    .view()
                    .history_search
                    .and_then(|history_search| history_search.match_text),
                match_index: search.cursor,
                match_total: search.matches.len(),
            };
            return history_search_cursor_position(width, height, &view, search.query.cursor());
        }
        prompt_cursor_position(width, height, &self.prompt.text(), self.prompt.cursor())
    }

    pub(super) fn needs_render(&self) -> bool {
        self.needs_render
    }

    pub(super) fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    pub(super) fn take_external_editor_request(&mut self) -> Option<ExternalEditorRequest> {
        self.pending_external_editor.take()
    }

    pub(super) fn finish_external_editor_request(
        &mut self,
        request: &ExternalEditorRequest,
        result: Result<()>,
    ) {
        match result {
            Ok(()) => {
                self.status_note = Some(format!("edited {}", request.path.display()));
                self.needs_render = true;
            }
            Err(error) => {
                self.status_note = Some(format!("editor launch failed: {error}"));
                self.needs_render = true;
            }
        }
    }

    pub(super) fn mark_rendered(&mut self) {
        self.needs_render = false;
    }

    /// Recomputes the visible transcript height from terminal dimensions and
    /// calls [`TranscriptScrollState::on_resize`] to keep the scroll state
    /// consistent after the terminal is resized.
    ///
    /// Uses a rough prompt-height estimate derived from the current prompt text
    /// so that the scroll state stays accurate without building a full view.
    pub(super) fn on_terminal_resize(&mut self, width: u16, height: u16) {
        // Estimate prompt height the same way ShellView::prompt_height() does:
        // number of text lines + 2 border rows.
        let prompt_lines = self.prompt.text().lines().count().max(1);
        let prompt_height = u16::try_from(prompt_lines)
            .unwrap_or(u16::MAX)
            .saturating_add(2);
        let layout = ShellLayout::split(Rect::new(0, 0, width, height), prompt_height);
        let total = message_lines(&self.state.messages).len();
        self.scroll_state
            .on_resize(usize::from(layout.messages.height), total);
    }

    /// Recomputes the total rendered transcript line count and notifies the
    /// scroll state so that follow-tail and scrolled-up modes remain correct.
    ///
    /// Call this after any operation that adds or removes messages from
    /// `self.state.messages`.
    pub(super) fn notify_transcript_changed(&mut self) {
        let total = message_lines(&self.state.messages).len();
        self.scroll_state.on_messages_changed(total);
    }
}

/// Build the full list of slash-command suggestions from the command registry.
///
/// Called once at TUI startup; the result is stored on `TuiController` and
/// re-used (with live filtering) on every keystroke.
pub(super) fn build_slash_suggestions(registry: &CommandRegistry) -> Vec<PromptSuggestion> {
    registry
        .all_specs()
        .into_iter()
        .filter(|spec| !spec.hidden)
        .map(|spec| {
            let slash = format!("/{}", spec.name);
            PromptSuggestion::new(spec.name.clone(), slash.clone(), slash)
                .with_description(spec.description.clone())
                .with_keywords(spec.aliases.iter().map(|a| format!("/{a}")))
        })
        .collect()
}
