use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::Duration,
};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    queue,
    style::{Attribute, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use futures::executor::block_on;
use wonder_of_u_agent::{
    CompletionRequest, ProviderResolver, ProviderRuntime, ProviderSelection, ProviderToolCall,
    ProviderToolResultMessage, ProviderToolSpec, SettingsStore, ToolConversationRound,
    ToolUseRequest, ToolUseResponse, builtin_tool_registry,
};
use wonder_of_u_core::{
    AdditionalWorkingDirectory, AppState, AuthState, CommandContext, CommandOutput, CommandQuery,
    CommandRegistry, FeatureSet, InputMode, MessageEnvelope, MessagePayload, PendingLocalToolCall,
    PendingProviderToolCall, PendingProviderToolResult, PendingToolApprovalState,
    PendingToolConversationRound, PermissionDecision, PermissionMode, PermissionRuleSource,
    ProviderReadiness, QueuePlacement, Result, SessionId, TaskState, TaskStatus, ToolContext,
    ToolQuery, ToolResult, ToolUseId, WonderError, parse_slash_command, session_footer_text,
    session_status_text,
};
use wonder_of_u_storage::{TaskStore, TranscriptStore};
use wonder_of_u_tui::{
    CrosstermControl, CrosstermEventSource, DialogView, EditAction, EventLoop, FrameBuffer,
    KeyBindingContext, KeyBindingResolver, KeyCode, KeyEvent, ResolvedKey, ShellLayout, ShellView,
    TerminalConfig, TerminalLifecycle, TextBuffer, Theme, TurnState, UiEvent, VimMode, VimState,
};

use crate::commands;
use crate::commands::prompt::{
    SessionPersistenceState, append_contextual_message, load_or_create_state,
    persist_messages_and_state, persist_prompt_state, truncate_chars,
};

pub(crate) struct TuiLaunchOptions {
    pub session_id: Option<String>,
}

const MAX_TOOL_LOOP_ITERATIONS: usize = 6;
const PICKER_CONTROLS_NOTE: &str =
    "type to filter, use Up/Down to choose, Enter to select, Esc to cancel";

pub(crate) fn run_tui<W: Write>(
    writer: &mut W,
    registry: &CommandRegistry,
    storage_dir: Option<&Path>,
    options: TuiLaunchOptions,
) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(WonderError::validation(
            "`wonder-of-u tui` requires an interactive terminal",
        ));
    }
    let authenticated = ProviderResolver::builtin()
        .load_report(storage_dir)?
        .readiness
        == ProviderReadiness::Ready;
    let context = CommandContext {
        session_id: SessionId::new(),
        cwd: std::env::current_dir()?,
        features: FeatureSet::first_release(),
        authenticated,
        interactive: true,
        permission_mode: PermissionMode::Default,
        theme: None,
        session_color: None,
        effort_level: None,
        brief_mode: false,
        fast_mode: false,
        session_tags: Vec::new(),
        additional_working_directories: Vec::new(),
    };
    let mut controller = TuiController::new(context, registry, storage_dir, options)?;
    let mut lifecycle =
        TerminalLifecycle::enter(CrosstermControl::new(writer), TerminalConfig::default())?;
    let mut events = EventLoop::new(CrosstermEventSource, Duration::from_millis(500));

    render_controller(lifecycle.control_mut().writer_mut(), &controller)?;
    controller.mark_rendered();

    while !controller.exit_requested() {
        let event = events.next_event()?;
        controller.handle_event(event, |controller| {
            render_controller(lifecycle.control_mut().writer_mut(), controller)
        })?;
        if let Some(request) = controller.take_external_editor_request() {
            lifecycle.restore()?;
            let result = launch_external_editor(&request);
            lifecycle.reenter(TerminalConfig::default())?;
            controller.finish_external_editor_request(&request, result);
        }
        if controller.needs_render() {
            render_controller(lifecycle.control_mut().writer_mut(), &controller)?;
            controller.mark_rendered();
        }
    }

    Ok(())
}

struct TuiController<'a> {
    registry: &'a CommandRegistry,
    storage_dir: Option<PathBuf>,
    state: AppState,
    persistence: SessionPersistenceState,
    prompt: TextBuffer,
    keymap: KeyBindingResolver,
    vim: VimState,
    history_recall_index: Option<usize>,
    turn_state: TurnState,
    needs_render: bool,
    exit_requested: bool,
    status_note: Option<String>,
    dialog: Option<DialogView>,
    pending_tag_removal: Option<TagRemovalState>,
    pending_theme_picker: Option<ThemePickerState>,
    pending_model_picker: Option<ModelPickerState>,
    pending_permission_picker: Option<PermissionPickerState>,
    pending_memory_picker: Option<MemoryPickerState>,
    pending_external_editor: Option<ExternalEditorRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ModelPickerOption {
    provider: String,
    provider_display: String,
    model: String,
    model_display: String,
    default: bool,
    selected: bool,
    auth: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ModelPickerState {
    original_input: String,
    options: Vec<ModelPickerOption>,
    selected_index: usize,
    query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ThemePickerOption {
    theme: String,
    label: String,
    description: String,
    selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ThemePickerState {
    original_input: String,
    options: Vec<ThemePickerOption>,
    selected_index: usize,
    query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TagRemovalState {
    original_input: String,
    tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PermissionPickerOption {
    mode: PermissionMode,
    label: String,
    description: String,
    selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PermissionPickerState {
    original_input: String,
    options: Vec<PermissionPickerOption>,
    selected_index: usize,
    query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MemoryPickerOption {
    target: crate::commands::project::MemoryTarget,
    label: String,
    description: String,
    path: PathBuf,
    selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MemoryPickerState {
    original_input: String,
    options: Vec<MemoryPickerOption>,
    selected_index: usize,
    query: TextBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExternalEditorRequest {
    cwd: PathBuf,
    path: PathBuf,
}

#[derive(Clone)]
struct LocalToolCall {
    provider_call: ProviderToolCall,
    use_id: ToolUseId,
}

enum ToolExecutionOutcome {
    Completed(ProviderToolResultMessage),
    Paused { reason: String },
}

enum RestoredPromptUiState {
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
    fn new(
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
            history_recall_index: None,
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
        };
        controller.hydrate_initial_settings()?;
        controller.refresh_runtime_state()?;
        controller.rebuild_ephemeral_state();
        controller.persist_state_snapshot()?;
        Ok(controller)
    }

    fn hydrate_initial_settings(&mut self) -> Result<()> {
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

    fn handle_event<F>(&mut self, event: UiEvent, mut before_blocking: F) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        match event {
            UiEvent::Key(key) => self.handle_key_event(key, &mut before_blocking),
            UiEvent::Paste(text) => {
                if !text.is_empty() {
                    self.prompt.insert_text(&text);
                    self.turn_state = TurnState::EditingInput;
                    self.state.input_mode = InputMode::Prompt;
                    self.reset_history_recall();
                    self.status_note = None;
                    self.needs_render = true;
                }
                Ok(())
            }
            UiEvent::Tick => {
                if self.refresh_runtime_state()? {
                    self.needs_render = true;
                }
                Ok(())
            }
            UiEvent::Resize { .. }
            | UiEvent::Mouse(_)
            | UiEvent::FocusGained
            | UiEvent::FocusLost => {
                self.needs_render = true;
                Ok(())
            }
        }
    }

    fn handle_key_event<F>(&mut self, key: KeyEvent, before_blocking: &mut F) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        let resolved = self.keymap.resolve(KeyBindingContext::Prompt, key);
        if self.dialog.is_some() {
            return self.handle_dialog_key(key, resolved, before_blocking);
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
                self.needs_render = true;
                Ok(())
            }
            ResolvedKey::Edit(EditAction::InsertNewline) => self.submit_prompt(before_blocking),
            ResolvedKey::Edit(action) => {
                self.prompt.apply_edit_action(action);
                self.turn_state = TurnState::EditingInput;
                self.state.input_mode = InputMode::Prompt;
                self.reset_history_recall();
                self.status_note = None;
                self.needs_render = true;
                Ok(())
            }
            ResolvedKey::Vim(_) => Ok(()),
        }
    }

    fn handle_vim_key(&mut self, key: KeyEvent) -> Result<()> {
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

    fn handle_system_action(&mut self, system: wonder_of_u_tui::SystemAction) -> Result<()> {
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
            wonder_of_u_tui::SystemAction::HistorySearch => self.recall_previous_prompt(),
        }
    }

    fn handle_dialog_key<F>(
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

    fn handle_permission_picker_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
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

    fn handle_model_picker_key<F>(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
        _before_blocking: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Self) -> Result<()>,
    {
        match key.code {
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

    fn handle_memory_picker_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
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

    fn handle_theme_picker_key(
        &mut self,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        match key.code {
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

    fn handle_permission_dialog_key<F>(
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
            Some(ResolvedKey::InsertChar('n'))
            | Some(ResolvedKey::InsertChar('N'))
            | Some(ResolvedKey::System(wonder_of_u_tui::SystemAction::Interrupt))
            | _ if key.code == KeyCode::Esc => {
                self.resolve_pending_tool_approval(false, before_blocking)
            }
            _ => {
                self.status_note = Some("press Enter/y to allow, n/Esc to deny".into());
                self.needs_render = true;
                Ok(())
            }
        }
    }

    fn resolve_pending_tool_approval<F>(
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
        let tool_query = ToolQuery::from(&tool_context);
        let provider_tools = registry
            .enabled_specs_for(&tool_query)
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
                            .cloned()
                            .map(|call| pending_local_call_from_runtime(&call))
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

    fn approve_pending_tool_call<F>(
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

    fn deny_pending_tool_call<F>(
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

    fn continue_tool_loop_from_rounds<F>(
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
                                        .cloned()
                                        .map(|call| pending_local_call_from_runtime(&call))
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

    fn submit_prompt<F>(&mut self, before_blocking: &mut F) -> Result<()>
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

    fn execute_prompt_submission<F>(&mut self, input: &str, on_progress: &mut F) -> Result<()>
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

    fn execute_tool_loop_submission<F>(
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
        let tool_query = ToolQuery::from(&tool_context);
        let provider_tools = registry
            .enabled_specs_for(&tool_query)
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
                                        .cloned()
                                        .map(|call| pending_local_call_from_runtime(&call))
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

    fn resolve_prompt_execution(
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
    fn execute_slash_command(&mut self, input: &str) -> Result<()> {
        self.execute_slash_command_with(input, &mut |_| Ok(()))
    }

    fn execute_slash_command_with<F>(&mut self, input: &str, before_blocking: &mut F) -> Result<()>
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
        } else if self.pending_permission_picker.is_some() {
        } else if self.pending_memory_picker.is_some() {
        } else if self.pending_tag_removal.is_some() {
        } else if self.pending_theme_picker.is_some() {
        } else if self.pending_model_picker.is_some() {
        } else if self.pending_external_editor.is_some() {
            self.status_note = Some("opening file in editor".into());
        } else if !had_queued_commands {
            self.status_note = Some("slash command recorded".into());
        }
        self.needs_render = true;
        Ok(())
    }

    fn apply_inline_command_hints(&mut self, input: &str) {
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

    fn apply_command_output_hints(&mut self, text: Option<&str>) {
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
            self.dialog = Some(DialogView::notice(title, body));
            self.status_note = Some(note.into());
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

    fn drain_queued_commands<F>(&mut self, before_blocking: &mut F) -> Result<()>
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

    fn record_command_message(&mut self, input: &str, output: Option<&str>) -> Result<()> {
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
        )
    }

    fn persist_messages(&mut self, messages: &[MessageEnvelope]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        persist_messages_and_state(
            self.storage_dir.as_deref(),
            &self.state,
            &mut self.persistence,
            messages,
        )
    }

    fn tool_context(&self) -> ToolContext {
        ToolContext {
            session_id: self.state.session.id,
            cwd: self.state.session.cwd.clone(),
            permission_mode: self.state.permission_mode,
            additional_working_directories: self.state.additional_working_directories.clone(),
            permission_rules: Vec::new(),
            features: self.state.features.clone(),
        }
    }

    fn execute_tool_call<F>(
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
                    self.turn_state = TurnState::ToolPermissionPending;
                    self.state.input_mode = InputMode::PermissionPending;
                    let reason = other.reason().to_string();
                    self.dialog = Some(DialogView::permission(
                        call.tool_name.clone(),
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

    fn run_tool_call<F>(
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

    fn finalize_tool_result<F>(
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

    fn restore_current_session(&mut self) -> Result<()> {
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
        Ok(())
    }

    fn refresh_runtime_state(&mut self) -> Result<bool> {
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
            }
            self.state.background_tasks = effective_tasks;
            changed = true;
        }

        Ok(changed)
    }

    fn persist_state_snapshot(&self) -> Result<()> {
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

    fn command_context(&self) -> CommandContext {
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

    fn view(&self) -> ShellView {
        let mut view = ShellView::from_app_state(&self.state, self.prompt.text());
        let mut status = session_status_text(&self.state);
        status.push_str(" | turn=");
        status.push_str(turn_state_label(self.turn_state));
        if let Some(note) = &self.status_note {
            status.push_str(" | ");
            status.push_str(note);
        }
        view.status = status;

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
        footer.push_str(" | vim=");
        footer.push_str(match self.vim.mode() {
            VimMode::Insert => "insert",
            VimMode::Normal => "normal",
        });
        footer.push_str(" | theme=");
        footer.push_str(match self.state.theme.as_deref() {
            Some("midnight") => "midnight",
            Some("light") => "light",
            _ => "default",
        });
        footer.push_str(" | color=");
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
        footer.push_str(" | effort=");
        footer.push_str(match self.state.effort_level.as_deref() {
            Some("low") => "low",
            Some("medium") => "medium",
            Some("high") => "high",
            Some("max") => "max",
            _ => "auto",
        });
        footer.push_str(" | fast=");
        footer.push_str(if self.state.fast_mode { "on" } else { "off" });
        footer.push_str(" | brief=");
        footer.push_str(if self.state.brief_mode { "on" } else { "off" });
        footer.push_str(" | enter submit");
        view.footer = footer;
        view.dialog = self.dialog.clone();
        view
    }

    fn open_permission_picker(&mut self, mut picker: PermissionPickerState) {
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
        self.refresh_permission_picker_dialog();
    }

    fn refresh_permission_picker_dialog(&mut self) {
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

    fn step_permission_picker(&mut self, delta: isize) {
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

    fn edit_permission_picker_query(&mut self, resolved: ResolvedKey) -> bool {
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

    fn complete_permission_picker(&mut self) -> Result<()> {
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

    fn cancel_permission_picker(&mut self) -> Result<()> {
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

    fn open_memory_picker(&mut self, mut picker: MemoryPickerState) {
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
        self.refresh_memory_picker_dialog();
    }

    fn refresh_memory_picker_dialog(&mut self) {
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

    fn step_memory_picker(&mut self, delta: isize) {
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

    fn edit_memory_picker_query(&mut self, resolved: ResolvedKey) -> bool {
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

    fn complete_memory_picker(&mut self) -> Result<()> {
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

    fn cancel_memory_picker(&mut self) -> Result<()> {
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

    fn open_tag_removal_confirmation(&mut self, pending: TagRemovalState) {
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

    fn handle_tag_removal_key(
        &mut self,
        _key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) -> Result<()> {
        if matches!(resolved, Some(ResolvedKey::Edit(EditAction::InsertNewline))) {
            return self.complete_tag_removal();
        }
        self.cancel_tag_removal()
    }

    fn complete_tag_removal(&mut self) -> Result<()> {
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

    fn cancel_tag_removal(&mut self) -> Result<()> {
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

    fn open_theme_picker(&mut self, mut picker: ThemePickerState) {
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
        self.refresh_theme_picker_dialog();
    }

    fn refresh_theme_picker_dialog(&mut self) {
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

    fn step_theme_picker(&mut self, delta: isize) {
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

    fn edit_theme_picker_query(&mut self, resolved: ResolvedKey) -> bool {
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

    fn complete_theme_picker(&mut self) -> Result<()> {
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

    fn cancel_theme_picker(&mut self) -> Result<()> {
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

    fn open_model_picker(&mut self, mut picker: ModelPickerState) {
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
        self.refresh_model_picker_dialog();
    }

    fn refresh_model_picker_dialog(&mut self) {
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

    fn step_model_picker(&mut self, delta: isize) {
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

    fn edit_model_picker_query(&mut self, resolved: ResolvedKey) -> bool {
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

    fn complete_model_picker(&mut self) -> Result<()> {
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

    fn cancel_model_picker(&mut self) -> Result<()> {
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

    fn dismiss_dialog(&mut self) {
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

    fn dismiss_notice_dialog(&mut self) {
        let status = self
            .dialog
            .as_ref()
            .map(|dialog| overlay_closed_status(&dialog.title));
        self.dismiss_dialog();
        if let Some(status) = status {
            self.status_note = Some(status);
        }
    }

    fn has_picker_overlay(&self) -> bool {
        self.pending_permission_picker.is_some()
            || self.pending_memory_picker.is_some()
            || self.pending_tag_removal.is_some()
            || self.pending_theme_picker.is_some()
            || self.pending_model_picker.is_some()
    }

    fn should_confirm_exit(&self) -> bool {
        !self.prompt.text().trim().is_empty()
            || !self.state.messages.is_empty()
            || !self.state.background_tasks.is_empty()
    }

    fn reset_history_recall(&mut self) {
        self.history_recall_index = None;
    }

    fn recall_previous_prompt(&mut self) -> Result<()> {
        let entries = prompt_history_entries(&self.state.messages);
        if entries.is_empty() {
            self.status_note = Some("history empty".into());
            self.needs_render = true;
            return Ok(());
        }
        let next_index = self
            .history_recall_index
            .map(|index| (index + 1) % entries.len())
            .unwrap_or(0);
        self.prompt = TextBuffer::from_text(&entries[next_index], false);
        self.history_recall_index = Some(next_index);
        self.turn_state = TurnState::EditingInput;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some(format!("history {}/{}", next_index + 1, entries.len()));
        self.needs_render = true;
        Ok(())
    }

    fn rebuild_ephemeral_state(&mut self) {
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
                    self.state.pending_tool_approval.is_some(),
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

    fn prompt_cursor(&self, width: u16, height: u16) -> (u16, u16) {
        prompt_cursor_position(width, height, &self.prompt.text(), self.prompt.cursor())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    fn take_external_editor_request(&mut self) -> Option<ExternalEditorRequest> {
        self.pending_external_editor.take()
    }

    fn finish_external_editor_request(
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

    fn mark_rendered(&mut self) {
        self.needs_render = false;
    }
}

fn command_output_text(output: CommandOutput) -> (Option<String>, bool) {
    match output {
        CommandOutput::Text(text)
        | CommandOutput::EnqueuePrompt(text)
        | CommandOutput::OpenUi(text) => (Some(text), false),
        CommandOutput::ExitRequested => (Some("exit requested".into()), true),
        CommandOutput::Noop => (None, false),
    }
}

fn launch_external_editor(request: &ExternalEditorRequest) -> Result<()> {
    let Some((editor, args)) = tui_editor_command() else {
        return Err(WonderError::validation(
            "set VISUAL or EDITOR to enable `/plan open`",
        ));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&request.path)
        .current_dir(&request.cwd)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(WonderError::internal(format!(
            "editor exited unsuccessfully: {editor}"
        )))
    }
}

fn tui_editor_command() -> Option<(String, Vec<String>)> {
    let raw = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })?;
    let mut tokens = shell_words::split(&raw).ok()?;
    let command = tokens.first()?.clone();
    Some((command, tokens.drain(1..).collect()))
}

fn render_controller<W: Write>(writer: &mut W, controller: &TuiController<'_>) -> Result<()> {
    let (width, height) = terminal::size()?;
    let width = width.max(20);
    let height = height.max(6);
    let view = controller.view();
    let frame = wonder_of_u_tui::render_snapshot(
        width,
        height,
        &view,
        &theme_for_state(
            controller.state.theme.as_deref(),
            controller.state.session_color.as_deref(),
        ),
    );
    let (cursor_x, cursor_y) = controller.prompt_cursor(width, height);
    render_frame(writer, &frame, cursor_x, cursor_y)?;
    Ok(())
}

fn theme_for_state(name: Option<&str>, session_color: Option<&str>) -> Theme {
    let mut theme = match name {
        Some("midnight") => Theme {
            background: wonder_of_u_tui::TextStyle::default()
                .bg(wonder_of_u_tui::Color::Black)
                .fg(wonder_of_u_tui::Color::Grey),
            border: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::DarkMagenta),
            title: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::Magenta)
                .bold(),
            messages: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::Grey),
            prompt: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::Cyan)
                .bold(),
            status: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::Yellow)
                .bold(),
            footer: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::DarkCyan),
        },
        Some("light") => Theme {
            background: wonder_of_u_tui::TextStyle::default()
                .bg(wonder_of_u_tui::Color::White)
                .fg(wonder_of_u_tui::Color::Black),
            border: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::Blue),
            title: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::DarkBlue)
                .bold(),
            messages: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::Black),
            prompt: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::DarkCyan)
                .bold(),
            status: wonder_of_u_tui::TextStyle::default()
                .fg(wonder_of_u_tui::Color::DarkBlue)
                .bold(),
            footer: wonder_of_u_tui::TextStyle::default().fg(wonder_of_u_tui::Color::DarkGrey),
        },
        _ => Theme::default(),
    };
    if let Some(color) = session_color_style(name, session_color) {
        theme.border = theme.border.fg(color);
        theme.title = theme.title.fg(color);
        theme.prompt = theme.prompt.fg(color).bold();
    }
    theme
}

fn session_color_style(
    theme_name: Option<&str>,
    color: Option<&str>,
) -> Option<wonder_of_u_tui::Color> {
    Some(match (theme_name, color?) {
        (_, "red") => wonder_of_u_tui::Color::Red,
        (Some("light"), "blue") => wonder_of_u_tui::Color::DarkBlue,
        (_, "blue") => wonder_of_u_tui::Color::Blue,
        (Some("light"), "green") => wonder_of_u_tui::Color::DarkGreen,
        (_, "green") => wonder_of_u_tui::Color::Green,
        (Some("light"), "yellow") => wonder_of_u_tui::Color::DarkYellow,
        (_, "yellow") => wonder_of_u_tui::Color::Yellow,
        (_, "purple") => wonder_of_u_tui::Color::Magenta,
        (_, "orange") => wonder_of_u_tui::Color::Rgb(255, 165, 0),
        (_, "pink") => wonder_of_u_tui::Color::Rgb(255, 105, 180),
        (Some("light"), "cyan") => wonder_of_u_tui::Color::DarkCyan,
        (_, "cyan") => wonder_of_u_tui::Color::Cyan,
        _ => return None,
    })
}

fn render_frame<W: Write>(
    writer: &mut W,
    frame: &FrameBuffer,
    cursor_x: u16,
    cursor_y: u16,
) -> Result<()> {
    queue!(writer, Hide, MoveTo(0, 0), Clear(ClearType::All))?;
    let mut active_style = None;
    for y in 0..frame.height() {
        queue!(writer, MoveTo(0, y))?;
        for x in 0..frame.width() {
            let cell = frame
                .cell(x, y)
                .ok_or_else(|| WonderError::internal("frame cell out of bounds"))?;
            if active_style != Some(cell.style) {
                queue!(writer, ResetColor, SetAttribute(Attribute::Reset))?;
                apply_style(writer, cell.style)?;
                active_style = Some(cell.style);
            }
            queue!(writer, Print(cell.symbol))?;
        }
    }
    queue!(
        writer,
        ResetColor,
        SetAttribute(Attribute::Reset),
        MoveTo(cursor_x, cursor_y),
        Show
    )?;
    writer.flush()?;
    Ok(())
}

fn apply_style<W: Write>(writer: &mut W, style: wonder_of_u_tui::TextStyle) -> Result<()> {
    if let Some(color) = style.fg {
        queue!(writer, SetForegroundColor(color.into()))?;
    }
    if let Some(color) = style.bg {
        queue!(writer, SetBackgroundColor(color.into()))?;
    }
    if style.bold {
        queue!(writer, SetAttribute(Attribute::Bold))?;
    }
    if style.dim {
        queue!(writer, SetAttribute(Attribute::Dim))?;
    }
    if style.italic {
        queue!(writer, SetAttribute(Attribute::Italic))?;
    }
    if style.underlined {
        queue!(writer, SetAttribute(Attribute::Underlined))?;
    }
    if style.reversed {
        queue!(writer, SetAttribute(Attribute::Reverse))?;
    }
    Ok(())
}

fn prompt_cursor_position(width: u16, height: u16, prompt: &str, cursor: usize) -> (u16, u16) {
    let layout = ShellLayout::split(
        wonder_of_u_tui::Rect::new(0, 0, width.max(1), height.max(1)),
        ShellView {
            title: String::new(),
            messages: Vec::new(),
            prompt: prompt.into(),
            status: String::new(),
            footer: String::new(),
            task_panel: None,
            dialog: None,
        }
        .prompt_height(),
    );
    let inner = layout.prompt.inset(1);
    let cursor_text = prompt.chars().take(cursor).collect::<String>();
    let mut line = 0u16;
    let mut column = 0u16;
    for ch in cursor_text.chars() {
        if ch == '\n' {
            line = line.saturating_add(1);
            column = 0;
        } else {
            column = column.saturating_add(1);
        }
    }
    (
        inner
            .x
            .saturating_add(column.min(inner.width.saturating_sub(1))),
        inner
            .y
            .saturating_add(line.min(inner.height.saturating_sub(1))),
    )
}

fn default_session_title(cwd: &Path) -> String {
    let leaf = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    format!("Interactive: {leaf}")
}

fn compose_conversation_prompt(messages: &[MessageEnvelope], input: &str) -> String {
    let history = messages
        .iter()
        .filter_map(conversation_line)
        .collect::<Vec<_>>();
    if history.is_empty() {
        return input.to_string();
    }

    let mut prompt = String::from(
        "Continue the conversation below. Use the prior assistant and user context when it is relevant.\n\n",
    );
    for line in history
        .into_iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        prompt.push_str(&line);
        prompt.push('\n');
    }
    prompt.push_str("user: ");
    prompt.push_str(&normalize_inline(input));
    prompt.push_str("\nassistant:");
    prompt
}

fn conversation_line(message: &MessageEnvelope) -> Option<String> {
    match &message.payload {
        MessagePayload::UserText { content } => {
            Some(format!("user: {}", normalize_inline(content)))
        }
        MessagePayload::AssistantText { content } => {
            Some(format!("assistant: {}", normalize_inline(content)))
        }
        MessagePayload::AssistantToolUse { tool, input, .. } => Some(format!(
            "assistant_tool[{tool}]: {}",
            normalize_inline(&serde_json::to_string(input).unwrap_or_default())
        )),
        MessagePayload::ToolResult {
            tool,
            success,
            content,
            ..
        } => Some(format!(
            "tool[{tool} {}]: {}",
            if *success { "ok" } else { "error" },
            normalize_inline(content)
        )),
        MessagePayload::Permission {
            tool,
            decision,
            reason,
        } => Some(format!(
            "permission[{tool} {decision}]: {}",
            normalize_inline(reason)
        )),
        MessagePayload::System { content } => {
            Some(format!("system: {}", normalize_inline(content)))
        }
        _ => None,
    }
}

fn normalize_inline(text: &str) -> String {
    truncate_chars(
        &text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ⏎ "),
        320,
    )
}

fn append_streamed_text(state: &mut AppState, assistant_index: usize, delta: &str) -> Result<()> {
    if delta.is_empty() {
        return Ok(());
    }
    let Some(message) = state.messages.get_mut(assistant_index) else {
        return Err(WonderError::internal("streamed assistant message missing"));
    };
    let MessagePayload::AssistantText { content } = &mut message.payload else {
        return Err(WonderError::internal(
            "streamed assistant placeholder was not an assistant message",
        ));
    };
    content.push_str(delta);
    Ok(())
}

fn set_assistant_message_content(
    state: &mut AppState,
    assistant_index: usize,
    content: &str,
) -> Result<()> {
    let Some(message) = state.messages.get_mut(assistant_index) else {
        return Err(WonderError::internal("assistant message missing"));
    };
    let MessagePayload::AssistantText {
        content: assistant_content,
    } = &mut message.payload
    else {
        return Err(WonderError::internal(
            "assistant placeholder was not an assistant message",
        ));
    };
    assistant_content.clear();
    assistant_content.push_str(content);
    Ok(())
}

fn tool_spec_to_provider_tool(spec: wonder_of_u_core::ToolSpec) -> ProviderToolSpec {
    ProviderToolSpec {
        name: spec.name,
        description: spec.description,
        input_schema: spec.input_schema,
    }
}

fn render_provider_tool_result(result: &ToolResult) -> String {
    if result.success {
        result.content.clone()
    } else {
        format!("ERROR: {}", result.content)
    }
}

fn permission_decision_label(decision: &PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Allow { .. } => "allow",
        PermissionDecision::Ask { .. } => "ask",
        PermissionDecision::Deny { .. } => "deny",
    }
}

fn task_status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Killed => "killed",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn latest_terminal_task(
    tasks: &BTreeMap<wonder_of_u_core::TaskId, TaskState>,
) -> Option<&TaskState> {
    tasks
        .values()
        .filter(|task| task.status.is_terminal())
        .max_by(|left, right| {
            left.finished_at
                .unwrap_or(left.started_at)
                .cmp(&right.finished_at.unwrap_or(right.started_at))
                .then_with(|| left.started_at.cmp(&right.started_at))
        })
}

fn next_task_notification<'a>(
    previous: &'a BTreeMap<wonder_of_u_core::TaskId, TaskState>,
    next: &'a BTreeMap<wonder_of_u_core::TaskId, TaskState>,
) -> Option<&'a TaskState> {
    next.values().find(|task| {
        let Some(previous_task) = previous.get(&task.id) else {
            return task.status.is_terminal();
        };
        previous_task.status != task.status && task.status.is_terminal()
    })
}

fn task_notification_lines(task: &TaskState) -> Vec<String> {
    let mut lines = vec![format!(
        "[{}] {}",
        task_status_label(task.status),
        task.description
    )];
    if let Some(status_message) = task
        .status_message
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(status_message.to_string());
    }
    if let Some(exit_code) = task.exit_code {
        lines.push(format!("exit code: {exit_code}"));
    }
    lines
}

fn permission_required_status(tool: &str) -> String {
    format!("approval required: {tool}")
}

fn restored_permission_dialog(
    messages: &[MessageEnvelope],
    has_pending_approval: bool,
) -> Option<(DialogView, String)> {
    messages
        .iter()
        .rev()
        .find_map(|message| match &message.payload {
            MessagePayload::Permission {
                tool,
                decision,
                reason,
            } => {
                let (dialog, status_note) = match decision.as_str() {
                    "ask" if has_pending_approval => (
                        DialogView::permission(tool.clone(), reason.clone()),
                        permission_required_status(tool),
                    ),
                    "ask" => (
                        DialogView::notice(
                            "Permission required",
                            [
                                format!("Tool `{tool}` requires approval."),
                                reason.clone(),
                                "Pending execution state is no longer available.".into(),
                            ],
                        ),
                        format!("pending approval unavailable: {tool}"),
                    ),
                    "deny" => (
                        DialogView::notice(
                            "Permission denied",
                            [format!("Tool `{tool}` was denied."), reason.clone()],
                        ),
                        format!("permission denied: {tool}"),
                    ),
                    other => (
                        DialogView::notice(
                            "Permission update",
                            [
                                format!("Tool `{tool}` permission is `{other}`."),
                                reason.clone(),
                            ],
                        ),
                        format!("permission update: {tool}"),
                    ),
                };
                Some((dialog, status_note))
            }
            _ => None,
        })
}

fn restored_prompt_ui_state(messages: &[MessageEnvelope]) -> Option<RestoredPromptUiState> {
    let MessagePayload::Command {
        input,
        output: Some(output),
    } = &messages.last()?.payload
    else {
        return None;
    };

    restored_command_ui_state(input, output)
}

fn restored_command_ui_state(input: &str, output: &str) -> Option<RestoredPromptUiState> {
    if let Some(mut picker) = parse_permission_picker_state(output) {
        picker.original_input = input.to_string();
        return Some(RestoredPromptUiState::PermissionPicker(picker));
    }
    if let Some(mut picker) = parse_memory_picker_state(output) {
        picker.original_input = input.to_string();
        return Some(RestoredPromptUiState::MemoryPicker(picker));
    }
    if let Some(tag) = parse_tag_remove_confirmation(output) {
        return Some(RestoredPromptUiState::TagRemoval(TagRemovalState {
            original_input: input.to_string(),
            tag,
        }));
    }
    if let Some(mut picker) = parse_theme_picker_state(output) {
        picker.original_input = input.to_string();
        return Some(RestoredPromptUiState::ThemePicker(picker));
    }
    if let Some(mut picker) = parse_model_picker_state(output) {
        picker.original_input = input.to_string();
        return Some(RestoredPromptUiState::ModelPicker(picker));
    }
    if let Some((title, body, note)) = parse_known_notice(output) {
        return Some(RestoredPromptUiState::Notice {
            dialog: DialogView::notice(title, body),
            status_note: note.into(),
        });
    }
    if let Some(view_action) = parse_view_action_hint(Some(output)) {
        return Some(RestoredPromptUiState::Status(match view_action {
            ViewActionHint::Clear => "conversation cleared".into(),
            ViewActionHint::Compact => "conversation compacted".into(),
        }));
    }
    if parse_insights_hint(output) {
        return Some(RestoredPromptUiState::Status("insights queued".into()));
    }
    if let Some(enabled) = parse_fast_mode_hint(output) {
        return Some(RestoredPromptUiState::Status(format!(
            "fast {}",
            if enabled { "on" } else { "off" }
        )));
    }
    if let Some(enabled) = parse_brief_mode_hint(output) {
        return Some(RestoredPromptUiState::Status(format!(
            "brief {}",
            if enabled { "on" } else { "off" }
        )));
    }
    if let Some(effort) = parse_effort_hint(output) {
        return Some(RestoredPromptUiState::Status(format!("effort {effort}")));
    }
    if let Some(color) = parse_session_color_hint(output) {
        return Some(RestoredPromptUiState::Status(format!("color {color}")));
    }
    None
}

fn pending_provider_call_from_runtime(call: &ProviderToolCall) -> PendingProviderToolCall {
    PendingProviderToolCall {
        call_id: call.call_id.clone(),
        tool_name: call.tool_name.clone(),
        arguments: call.arguments.clone(),
    }
}

fn pending_provider_result_from_runtime(
    result: &ProviderToolResultMessage,
) -> PendingProviderToolResult {
    PendingProviderToolResult {
        call_id: result.call_id.clone(),
        content: result.content.clone(),
    }
}

fn pending_round_from_runtime(round: &ToolConversationRound) -> PendingToolConversationRound {
    PendingToolConversationRound {
        assistant_text: round.assistant_text.clone(),
        calls: round
            .calls
            .iter()
            .map(pending_provider_call_from_runtime)
            .collect(),
        results: round
            .results
            .iter()
            .map(pending_provider_result_from_runtime)
            .collect(),
    }
}

fn pending_local_call_from_runtime(call: &LocalToolCall) -> PendingLocalToolCall {
    PendingLocalToolCall {
        provider_call: pending_provider_call_from_runtime(&call.provider_call),
        use_id: call.use_id,
    }
}

fn runtime_provider_call_from_pending(call: &PendingProviderToolCall) -> ProviderToolCall {
    ProviderToolCall {
        call_id: call.call_id.clone(),
        tool_name: call.tool_name.clone(),
        arguments: call.arguments.clone(),
    }
}

fn runtime_provider_result_from_pending(
    result: &PendingProviderToolResult,
) -> ProviderToolResultMessage {
    ProviderToolResultMessage {
        call_id: result.call_id.clone(),
        content: result.content.clone(),
    }
}

fn runtime_round_from_pending(round: &PendingToolConversationRound) -> ToolConversationRound {
    ToolConversationRound {
        assistant_text: round.assistant_text.clone(),
        calls: round
            .calls
            .iter()
            .map(runtime_provider_call_from_pending)
            .collect(),
        results: round
            .results
            .iter()
            .map(runtime_provider_result_from_pending)
            .collect(),
    }
}

fn local_call_from_pending(call: &PendingLocalToolCall) -> LocalToolCall {
    LocalToolCall {
        provider_call: runtime_provider_call_from_pending(&call.provider_call),
        use_id: call.use_id,
    }
}

fn prompt_history_entries(messages: &[MessageEnvelope]) -> Vec<String> {
    let mut entries = Vec::new();
    for message in messages.iter().rev() {
        if let MessagePayload::UserText { content } = &message.payload {
            let trimmed = content.trim();
            if !trimmed.is_empty() && !entries.iter().any(|entry| entry == trimmed) {
                entries.push(trimmed.to_string());
            }
        }
    }
    entries
}

fn runtime_label(provider: Option<&str>, model: Option<&str>) -> &'static str {
    let _ = model;
    match provider {
        Some("openai" | "anthropic" | "copilot") => "runtime=tool-loop",
        Some(_) => "runtime=non-streaming",
        None => "runtime=tool-loop-ready",
    }
}

fn turn_state_label(state: TurnState) -> &'static str {
    match state {
        TurnState::Idle => "idle",
        TurnState::EditingInput => "editing",
        TurnState::CommandQueued => "slash",
        TurnState::ModelRequestActive => "model",
        TurnState::ToolPermissionPending => "permission",
        TurnState::ToolExecuting => "tool",
        TurnState::StreamingResponse => "streaming",
        TurnState::Interrupted => "interrupted",
        TurnState::Completed => "done",
    }
}

fn parse_permission_mode_hint(value: &str) -> Option<PermissionMode> {
    match value {
        "default" => Some(PermissionMode::Default),
        "accept-edits" | "acceptEdits" => Some(PermissionMode::AcceptEdits),
        "bypass" | "bypass-permissions" | "bypassPermissions" => {
            Some(PermissionMode::BypassPermissions)
        }
        "dont-ask" | "dontAsk" => Some(PermissionMode::DontAsk),
        "plan" => Some(PermissionMode::Plan),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ViewActionHint {
    Clear,
    Compact,
}

fn parse_view_action_hint(text: Option<&str>) -> Option<ViewActionHint> {
    match text?
        .lines()
        .find_map(|line| line.strip_prefix("view_action="))?
        .trim()
    {
        "clear" => Some(ViewActionHint::Clear),
        "compact" => Some(ViewActionHint::Compact),
        _ => None,
    }
}

fn permission_mode_output_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept-edits",
        PermissionMode::BypassPermissions => "bypass-permissions",
        PermissionMode::DontAsk => "dont-ask",
        PermissionMode::Plan => "plan",
    }
}

fn parse_vim_toggle_hint(text: &str) -> bool {
    text.lines()
        .find_map(|line| line.strip_prefix("vim_toggle="))
        .map(str::trim)
        == Some("true")
}

fn parse_vim_mode_hint(text: &str) -> Option<VimMode> {
    match text
        .lines()
        .find_map(|line| line.strip_prefix("vim_mode="))?
        .trim()
    {
        "insert" => Some(VimMode::Insert),
        "normal" => Some(VimMode::Normal),
        _ => None,
    }
}

fn parse_additional_working_directory_hint(text: &str) -> Option<AdditionalWorkingDirectory> {
    let path = text
        .lines()
        .find_map(|line| line.strip_prefix("add_dir="))
        .map(str::trim)
        .filter(|line| !line.is_empty())?;
    let source = match text
        .lines()
        .find_map(|line| line.strip_prefix("add_dir_source="))
        .map(str::trim)
        .unwrap_or("session_runtime")
    {
        "cli_arg" => PermissionRuleSource::CliArg,
        "command" => PermissionRuleSource::Command,
        "local" => PermissionRuleSource::Local,
        "project" => PermissionRuleSource::Project,
        "user" => PermissionRuleSource::User,
        _ => PermissionRuleSource::SessionRuntime,
    };
    Some(AdditionalWorkingDirectory::new(path, source))
}

fn parse_permission_picker_state(text: &str) -> Option<PermissionPickerState> {
    let enabled = text
        .lines()
        .find_map(|line| line.strip_prefix("permission_picker="))?
        .trim()
        == "true";
    if !enabled {
        return None;
    }
    let options = text
        .lines()
        .filter_map(|line| line.strip_prefix("permission_option="))
        .filter_map(parse_permission_picker_option)
        .collect::<Vec<_>>();
    if options.is_empty() {
        return None;
    }
    let selected_index = options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0);
    Some(PermissionPickerState {
        original_input: "/permissions".into(),
        options,
        selected_index,
        query: TextBuffer::new(false),
    })
}

fn parse_permission_picker_option(line: &str) -> Option<PermissionPickerOption> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(PermissionPickerOption {
        mode: parse_permission_mode_hint(value.get("mode")?.as_str()?)?,
        label: value.get("label")?.as_str()?.to_string(),
        description: value.get("description")?.as_str()?.to_string(),
        selected: value.get("selected")?.as_bool().unwrap_or(false),
    })
}

fn parse_memory_picker_state(text: &str) -> Option<MemoryPickerState> {
    let enabled = text
        .lines()
        .find_map(|line| line.strip_prefix("memory_picker="))?
        .trim()
        == "true";
    if !enabled {
        return None;
    }
    let options = text
        .lines()
        .filter_map(|line| line.strip_prefix("memory_option="))
        .filter_map(parse_memory_picker_option)
        .collect::<Vec<_>>();
    if options.is_empty() {
        return None;
    }
    let selected_index = options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0);
    Some(MemoryPickerState {
        original_input: "/memory".into(),
        options,
        selected_index,
        query: TextBuffer::new(false),
    })
}

fn parse_memory_picker_option(line: &str) -> Option<MemoryPickerOption> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(MemoryPickerOption {
        target: match value.get("target")?.as_str()? {
            "project" => crate::commands::project::MemoryTarget::Project,
            "user" => crate::commands::project::MemoryTarget::User,
            _ => return None,
        },
        label: value.get("label")?.as_str()?.to_string(),
        description: value.get("description")?.as_str()?.to_string(),
        path: PathBuf::from(value.get("path")?.as_str()?),
        selected: value.get("selected")?.as_bool().unwrap_or(false),
    })
}

fn parse_theme_picker_state(text: &str) -> Option<ThemePickerState> {
    let enabled = text
        .lines()
        .find_map(|line| line.strip_prefix("theme_picker="))?
        .trim()
        == "true";
    if !enabled {
        return None;
    }
    let options = text
        .lines()
        .filter_map(|line| line.strip_prefix("theme_option="))
        .filter_map(parse_theme_picker_option)
        .collect::<Vec<_>>();
    if options.is_empty() {
        return None;
    }
    let selected_index = options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0);
    Some(ThemePickerState {
        original_input: "/theme".into(),
        options,
        selected_index,
        query: TextBuffer::new(false),
    })
}

fn parse_theme_picker_option(line: &str) -> Option<ThemePickerOption> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(ThemePickerOption {
        theme: value.get("theme")?.as_str()?.to_string(),
        label: value.get("label")?.as_str()?.to_string(),
        description: value.get("description")?.as_str()?.to_string(),
        selected: value.get("selected")?.as_bool().unwrap_or(false),
    })
}

fn parse_model_picker_state(text: &str) -> Option<ModelPickerState> {
    let enabled = text
        .lines()
        .find_map(|line| line.strip_prefix("model_picker="))?
        .trim()
        == "true";
    if !enabled {
        return None;
    }
    let mut options = text
        .lines()
        .filter_map(|line| line.strip_prefix("model_option="))
        .filter_map(parse_model_picker_option)
        .collect::<Vec<_>>();
    if options.is_empty() {
        return None;
    }
    let selected_index = options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0);
    if let Some(option) = options.get_mut(selected_index) {
        option.selected = true;
    }
    Some(ModelPickerState {
        original_input: "/model".into(),
        options,
        selected_index,
        query: TextBuffer::new(false),
    })
}

fn parse_theme_hint(text: &str) -> Option<&str> {
    let value = text.lines().find_map(|line| line.strip_prefix("theme="))?;
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn parse_session_color_hint(text: &str) -> Option<&str> {
    let value = text.lines().find_map(|line| line.strip_prefix("color="))?;
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn parse_effort_hint(text: &str) -> Option<&str> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("effort_level="))?;
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn parse_fast_mode_hint(text: &str) -> Option<bool> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("fast_mode="))?;
    match value.trim() {
        "true" | "on" | "enabled" => Some(true),
        "false" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

fn parse_brief_mode_hint(text: &str) -> Option<bool> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("brief_mode="))?;
    match value.trim() {
        "true" | "on" | "enabled" => Some(true),
        "false" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

fn parse_insights_hint(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim() == "insights_prompt_ready=true")
}

fn parse_session_tags_hint(text: &str) -> Option<Vec<String>> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("session_tags="))?;
    Some(
        value
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(ToString::to_string)
            .collect(),
    )
}

fn parse_tag_remove_confirmation(text: &str) -> Option<String> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("tag_remove_confirmation="))?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn apply_picker_query_edit(query: &mut TextBuffer, resolved: ResolvedKey) -> bool {
    match resolved {
        ResolvedKey::InsertChar(ch) => {
            query.insert_char(ch);
            true
        }
        ResolvedKey::Edit(EditAction::InsertNewline) => false,
        ResolvedKey::Edit(action) => {
            query.apply_edit_action(action);
            true
        }
        ResolvedKey::System(_) | ResolvedKey::Vim(_) => false,
    }
}

fn filtered_picker_indices<T>(
    query: &TextBuffer,
    options: &[T],
    searchable: impl Fn(&T) -> String,
) -> Vec<usize> {
    let Some(query) = normalized_picker_query(query) else {
        return (0..options.len()).collect();
    };
    options
        .iter()
        .enumerate()
        .filter_map(|(index, option)| {
            searchable(option)
                .to_ascii_lowercase()
                .contains(&query)
                .then_some(index)
        })
        .collect()
}

fn normalized_picker_query(query: &TextBuffer) -> Option<String> {
    let query = query.text();
    let trimmed = query.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
}

fn picker_query_label(query: &TextBuffer) -> String {
    let query = query.text();
    if query.trim().is_empty() {
        "(all)".into()
    } else {
        query
    }
}

fn picker_dialog_header(
    label: &str,
    query: &TextBuffer,
    filtered_count: usize,
    total_count: usize,
) -> Vec<String> {
    vec![
        format!("{label}: {}", picker_query_label(query)),
        format!("Matches: {filtered_count}/{total_count}"),
    ]
}

fn selected_picker_index(selected_index: usize, filtered: &[usize]) -> Option<usize> {
    filtered
        .iter()
        .find(|&&index| index == selected_index)
        .copied()
        .or_else(|| filtered.first().copied())
}

fn sync_picker_selection(selected_index: &mut usize, filtered: &[usize]) {
    if let Some(index) = selected_picker_index(*selected_index, filtered) {
        *selected_index = index;
    }
}

fn step_picker_selection(selected_index: usize, delta: isize, filtered: &[usize]) -> usize {
    let Some(current_position) = filtered
        .iter()
        .position(|&index| index == selected_index)
        .or_else(|| (!filtered.is_empty()).then_some(0))
    else {
        return selected_index;
    };
    let next_position =
        (current_position as isize + delta).rem_euclid(filtered.len() as isize) as usize;
    filtered[next_position]
}

fn picker_status_note(title: &str) -> String {
    format!("{title}: {PICKER_CONTROLS_NOTE}")
}

fn overlay_closed_status(title: &str) -> String {
    format!("{} closed", title.to_ascii_lowercase())
}

fn parse_known_notice(text: &str) -> Option<(String, Vec<String>, &'static str)> {
    [
        ("## Context Usage", "Context Usage", "context usage"),
        ("## Activity Stats", "Activity Stats", "activity stats"),
        ("## Usage", "Usage", "usage"),
        ("## Theme", "Theme", "theme"),
        ("## Color", "Color", "color"),
        ("## Fast", "Fast", "fast"),
        ("## Brief", "Brief", "brief"),
        ("## Effort", "Effort", "effort"),
        ("## Feedback", "Feedback", "feedback"),
        ("## Insights", "Insights", "insights"),
        ("## Upgrade", "Upgrade", "upgrade"),
        ("## Desktop", "Desktop", "desktop"),
        ("## Mobile", "Mobile", "mobile"),
        ("## Chrome", "Chrome", "chrome"),
        ("## Release Notes", "Release Notes", "release notes"),
        ("## Version", "Version", "version"),
        ("## Hooks", "Hooks", "hooks"),
        ("## Keybindings", "Keybindings", "keybindings"),
        (
            "## Privacy Settings",
            "Privacy Settings",
            "privacy settings",
        ),
        ("## Terminal Setup", "Terminal Setup", "terminal setup"),
    ]
    .into_iter()
    .find_map(|(heading, title, note)| {
        parse_notice(text, heading, title).map(|(title, body)| (title, body, note))
    })
}

fn parse_notice(text: &str, heading: &str, title: &str) -> Option<(String, Vec<String>)> {
    let mut lines = text.lines();
    if lines.next()?.trim() != heading {
        return None;
    }
    Some((
        title.into(),
        lines
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect(),
    ))
}

fn parse_model_picker_option(line: &str) -> Option<ModelPickerOption> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(ModelPickerOption {
        provider: value.get("provider")?.as_str()?.to_string(),
        provider_display: value.get("provider_display")?.as_str()?.to_string(),
        model: value.get("model")?.as_str()?.to_string(),
        model_display: value.get("model_display")?.as_str()?.to_string(),
        default: value.get("default")?.as_bool().unwrap_or(false),
        selected: value.get("selected")?.as_bool().unwrap_or(false),
        auth: value.get("auth")?.as_str()?.to_string(),
    })
}

fn parse_external_editor_request(text: &str, cwd: &Path) -> Option<ExternalEditorRequest> {
    let should_open = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("open_external=")
                .or_else(|| line.strip_prefix("plan_open_external="))
                .or_else(|| line.strip_prefix("memory_open_external="))
        })?
        .trim()
        == "true";
    if !should_open {
        return None;
    }
    let path = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("external_path=")
                .or_else(|| line.strip_prefix("plan_path="))
                .or_else(|| line.strip_prefix("memory_path="))
        })
        .map(str::trim)
        .filter(|line| !line.is_empty())?;
    Some(ExternalEditorRequest {
        cwd: cwd.to_path_buf(),
        path: PathBuf::from(path),
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        path::Path,
        thread,
    };

    use serde_json::{Value, json};
    use wonder_of_u_agent::{
        AgentSettings, AuthMaterial, CredentialStore, SettingsStore, StoredCredentials,
    };
    use wonder_of_u_core::{
        AuthState, InputMode, MessageEnvelope, MessagePayload, PendingLocalToolCall,
        PendingProviderToolCall, PendingToolApprovalState, PendingToolConversationRound,
    };
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::*;
    use crate::commands;

    fn write_provider_config_for(storage_dir: &Path, provider: &str, model: &str, api_base: &str) {
        let mut settings = AgentSettings {
            selected_provider: Some(provider.into()),
            selected_model: Some(model.into()),
            ..AgentSettings::default()
        };
        settings
            .providers
            .entry(provider.into())
            .or_default()
            .api_base = Some(api_base.into());
        SettingsStore::new(storage_dir)
            .write(&settings)
            .expect("write settings");
        CredentialStore::new(storage_dir)
            .write(&StoredCredentials {
                providers: [(
                    provider.into(),
                    AuthMaterial::ApiKey {
                        key: "test-key".into(),
                    },
                )]
                .into(),
            })
            .expect("write credentials");
    }

    fn write_provider_config(storage_dir: &Path, api_base: &str) {
        write_provider_config_for(storage_dir, "openai", "gpt-4.1", api_base);
    }

    fn read_http_request(stream: &mut TcpStream) -> (String, Value) {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut chunk).expect("read request");
            assert!(read > 0, "expected request bytes");
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8(buffer[..header_end].to_vec()).expect("headers utf8");
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::trim)
                    .map(|value| value.parse::<usize>().expect("content length"))
            })
            .unwrap_or(0);
        while buffer.len() < header_end + content_length {
            let read = stream.read(&mut chunk).expect("read body");
            assert!(read > 0, "expected request body bytes");
            buffer.extend_from_slice(&chunk[..read]);
        }
        let body = serde_json::from_slice(&buffer[header_end..header_end + content_length])
            .expect("body json");
        (headers, body)
    }

    fn spawn_json_sequence_server(
        mut assert_request: impl FnMut(usize, String, Value) + Send + 'static,
        response_bodies: Vec<String>,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("server address");
        let handle = thread::spawn(move || {
            for (index, response_body) in response_bodies.into_iter().enumerate() {
                let (mut stream, _) = listener.accept().expect("accept request");
                let (headers, body) = read_http_request(&mut stream);
                assert_request(index, headers, body);
                write!(
                    stream,
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: application/json\r\n",
                        "Content-Length: {}\r\n",
                        "Connection: close\r\n\r\n",
                        "{}"
                    ),
                    response_body.len(),
                    response_body
                )
                .expect("write response");
                stream.flush().expect("flush response");
            }
        });
        (format!("http://{address}/v1"), handle)
    }

    fn test_context(cwd: &Path) -> CommandContext {
        CommandContext {
            session_id: SessionId::new(),
            cwd: cwd.to_path_buf(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    fn picker_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: wonder_of_u_tui::KeyModifiers::default(),
        }
    }

    fn send_dialog_key(
        controller: &mut TuiController<'_>,
        key: KeyEvent,
        resolved: Option<ResolvedKey>,
    ) {
        controller
            .handle_dialog_key(key, resolved, &mut |_| Ok(()))
            .expect("handle dialog key");
    }

    #[test]
    fn compose_conversation_prompt_includes_recent_history() {
        let session_id = SessionId::new();
        let messages = vec![
            MessageEnvelope::user_text(session_id, "first question"),
            MessageEnvelope::new(
                session_id,
                MessagePayload::AssistantText {
                    content: "first answer".into(),
                },
            ),
        ];

        let prompt = compose_conversation_prompt(&messages, "next question");

        assert!(prompt.contains("user: first question"));
        assert!(prompt.contains("assistant: first answer"));
        assert!(prompt.ends_with("user: next question\nassistant:"));
    }

    #[test]
    fn controller_routes_slash_commands_and_updates_provider_context() {
        let dir = unique_test_dir("tui-slash-model");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/model openai:gpt-4.1")
            .expect("execute slash");

        assert_eq!(controller.state.provider.as_deref(), Some("openai"));
        assert_eq!(controller.state.model.as_deref(), Some("gpt-4.1"));
        assert_eq!(
            controller.state.auth,
            AuthState::missing(wonder_of_u_core::AuthMaterialKind::ApiKey)
        );
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, .. })
                if input == "/model openai:gpt-4.1"
        ));
    }

    #[test]
    fn controller_opens_model_picker_for_bare_model_command() {
        let dir = unique_test_dir("tui-model-picker-open");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/model")
            .expect("open model picker");

        assert_eq!(
            controller.status_note.as_deref(),
            Some(
                "model picker: type to filter, use Up/Down to choose, Enter to select, Esc to cancel"
            )
        );
        assert!(controller.pending_model_picker.is_some());
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Model picker"
        ));
        assert!(!controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::Command { input, .. } if input == "/model"
            )
        }));
    }

    #[test]
    fn controller_filters_model_picker_with_visible_query_and_match_count() {
        let dir = unique_test_dir("tui-model-picker-filter");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/model")
            .expect("open model picker");
        for ch in ['h', 'a', 'i', 'k', 'u'] {
            send_dialog_key(
                &mut controller,
                picker_key(KeyCode::Char(ch)),
                Some(ResolvedKey::InsertChar(ch)),
            );
        }

        let picker = controller
            .pending_model_picker
            .as_ref()
            .expect("model picker open");
        let dialog = controller.dialog.as_ref().expect("dialog");
        let expected_matches = format!("Matches: 1/{}", picker.options.len());
        assert_eq!(
            dialog.body.first().map(String::as_str),
            Some("Search: haiku")
        );
        assert_eq!(
            dialog.body.get(1).map(String::as_str),
            Some(expected_matches.as_str())
        );
        assert!(dialog.body.iter().any(|line| line.contains("haiku")));
        assert!(!dialog.body.iter().any(|line| line.contains("sonnet")));
        assert_eq!(
            controller.status_note.as_deref(),
            Some(
                "model picker: type to filter, use Up/Down to choose, Enter to select, Esc to cancel"
            )
        );
    }

    #[test]
    fn controller_selects_model_from_picker() {
        let dir = unique_test_dir("tui-model-picker-select");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/model")
            .expect("open picker");
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Enter),
            Some(ResolvedKey::Edit(EditAction::InsertNewline)),
        );

        assert!(controller.pending_model_picker.is_none());
        assert!(controller.dialog.is_none());
        assert_eq!(controller.state.provider.as_deref(), Some("anthropic"));
        assert_eq!(
            controller.state.model.as_deref(),
            Some("claude-3-5-haiku-latest")
        );
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/model"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("provider_selection=anthropic:claude-3-5-haiku-latest"))
        ));
    }

    #[test]
    fn controller_cancels_model_picker() {
        let dir = unique_test_dir("tui-model-picker-cancel");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/model")
            .expect("open picker");
        controller
            .handle_dialog_key(
                KeyEvent {
                    code: KeyCode::Esc,
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                None,
                &mut |_| Ok(()),
            )
            .expect("cancel picker");

        assert!(controller.pending_model_picker.is_none());
        assert!(controller.dialog.is_none());
        assert_eq!(
            controller.status_note.as_deref(),
            Some("model picker cancelled")
        );
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/model"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("status=model picker cancelled"))
        ));
    }

    #[test]
    fn controller_opens_theme_picker_for_bare_theme_command() {
        let dir = unique_test_dir("tui-theme-picker-open");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/theme")
            .expect("open theme picker");

        assert_eq!(
            controller.status_note.as_deref(),
            Some(
                "theme picker: type to filter, use Up/Down to choose, Enter to select, Esc to cancel"
            )
        );
        assert!(controller.pending_theme_picker.is_some());
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Theme picker"
        ));
    }

    #[test]
    fn controller_selects_theme_from_picker() {
        let dir = unique_test_dir("tui-theme-picker-select");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/theme")
            .expect("open theme picker");
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Enter),
            Some(ResolvedKey::Edit(EditAction::InsertNewline)),
        );

        assert!(controller.pending_theme_picker.is_none());
        assert!(controller.dialog.is_none());
        assert_eq!(controller.state.theme.as_deref(), Some("midnight"));
        assert!(controller.view().footer.contains("theme=midnight"));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/theme"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("theme=midnight"))
        ));
    }

    #[test]
    fn controller_shows_theme_notice_dialog() {
        let dir = unique_test_dir("tui-theme-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/theme show")
            .expect("show theme");

        assert_eq!(controller.status_note.as_deref(), Some("theme"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Theme"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/theme show"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Theme"))
        ));
    }

    #[test]
    fn controller_dismisses_theme_notice_dialog_cleanly() {
        let dir = unique_test_dir("tui-theme-notice-dismiss");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/theme show")
            .expect("show theme");
        controller
            .handle_dialog_key(
                KeyEvent {
                    code: KeyCode::Esc,
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                None,
                &mut |_| Ok(()),
            )
            .expect("dismiss theme notice");

        assert!(controller.dialog.is_none());
        assert_eq!(controller.state.input_mode, InputMode::Prompt);
        assert_eq!(controller.status_note.as_deref(), Some("theme closed"));
    }

    #[test]
    fn apply_command_output_hints_clears_stale_notice_dialog() {
        let dir = unique_test_dir("tui-notice-clear");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller.apply_command_output_hints(Some("## Theme\nCurrent theme: default\n"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Theme"
        ));

        controller.apply_command_output_hints(Some("status=theme updated\n"));

        assert!(controller.dialog.is_none());
    }

    #[test]
    fn controller_hydrates_persisted_fast_and_effort_on_launch() {
        let dir = unique_test_dir("tui-hydrate-settings");
        SettingsStore::new(&dir)
            .write(&AgentSettings {
                selected_provider: Some("openai".into()),
                effort_level: Some("high".into()),
                fast_mode: true,
                ..AgentSettings::default()
            })
            .expect("write settings");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        assert_eq!(controller.state.effort_level.as_deref(), Some("high"));
        assert!(controller.state.fast_mode);
        assert!(controller.view().footer.contains("effort=high"));
        assert!(controller.view().footer.contains("fast=on"));
    }

    #[test]
    fn controller_sets_session_color_from_command() {
        let dir = unique_test_dir("tui-color-set");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/color purple")
            .expect("set color");

        assert_eq!(controller.state.session_color.as_deref(), Some("purple"));
        assert_eq!(controller.status_note.as_deref(), Some("color purple"));
        assert!(controller.view().footer.contains("color=purple"));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/color purple"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("color=purple"))
        ));
    }

    #[test]
    fn controller_shows_color_notice_dialog() {
        let dir = unique_test_dir("tui-color-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/color")
            .expect("show color");

        assert_eq!(controller.status_note.as_deref(), Some("color"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Color"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/color"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Color"))
        ));
    }

    #[test]
    fn controller_toggles_brief_mode_from_command() {
        let dir = unique_test_dir("tui-brief-toggle");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/brief")
            .expect("toggle brief");

        assert!(controller.state.brief_mode);
        assert_eq!(controller.status_note.as_deref(), Some("brief on"));
        assert!(controller.view().footer.contains("brief=on"));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/brief"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("brief_mode=true"))
        ));
    }

    #[test]
    fn controller_shows_brief_notice_dialog() {
        let dir = unique_test_dir("tui-brief-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/brief show")
            .expect("show brief");

        assert_eq!(controller.status_note.as_deref(), Some("brief"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Brief"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/brief show"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Brief"))
        ));
    }

    #[test]
    fn controller_toggles_fast_mode_from_command() {
        let dir = unique_test_dir("tui-fast-toggle");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/fast")
            .expect("toggle fast");

        assert!(controller.state.fast_mode);
        assert_eq!(controller.status_note.as_deref(), Some("fast on"));
        assert!(controller.view().footer.contains("fast=on"));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/fast"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("fast_mode=true"))
        ));
    }

    #[test]
    fn controller_shows_fast_notice_dialog() {
        let dir = unique_test_dir("tui-fast-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/fast show")
            .expect("show fast");

        assert_eq!(controller.status_note.as_deref(), Some("fast"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Fast"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/fast show"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Fast"))
        ));
    }

    #[test]
    fn controller_sets_effort_from_command() {
        let dir = unique_test_dir("tui-effort-set");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/effort high")
            .expect("set effort");

        assert_eq!(controller.state.effort_level.as_deref(), Some("high"));
        assert_eq!(controller.status_note.as_deref(), Some("effort high"));
        assert!(controller.view().footer.contains("effort=high"));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/effort high"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("effort_level=high"))
        ));
    }

    #[test]
    fn controller_shows_effort_notice_dialog() {
        let dir = unique_test_dir("tui-effort-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/effort")
            .expect("show effort");

        assert_eq!(controller.status_note.as_deref(), Some("effort"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Effort"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/effort"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Effort"))
        ));
    }

    #[test]
    fn controller_shows_feedback_notice_dialog_from_alias() {
        let dir = unique_test_dir("tui-feedback-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/bug parity gap")
            .expect("show feedback");

        assert_eq!(controller.status_note.as_deref(), Some("feedback"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Feedback"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/bug parity gap"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Feedback") && text.contains("parity gap"))
        ));
    }

    #[test]
    fn controller_shows_release_notes_notice_dialog() {
        let dir = unique_test_dir("tui-release-notes-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/release-notes")
            .expect("show release notes");

        assert_eq!(controller.status_note.as_deref(), Some("release notes"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Release Notes"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/release-notes"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Release Notes"))
        ));
    }

    #[test]
    fn controller_shows_version_notice_dialog() {
        let dir = unique_test_dir("tui-version-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/version")
            .expect("show version");

        assert_eq!(controller.status_note.as_deref(), Some("version"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Version"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/version"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Version"))
        ));
    }

    #[test]
    fn controller_shows_desktop_notice_dialog() {
        let dir = unique_test_dir("tui-desktop-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/desktop")
            .expect("show desktop");

        assert_eq!(controller.status_note.as_deref(), Some("desktop"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Desktop"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/desktop"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Desktop") && text.contains("desktop_docs_url="))
        ));
    }

    #[test]
    fn controller_shows_mobile_notice_dialog_from_alias() {
        let dir = unique_test_dir("tui-mobile-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/ios")
            .expect("show mobile");

        assert_eq!(controller.status_note.as_deref(), Some("mobile"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Mobile"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/ios"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Mobile") && text.contains("qr_rendered=false"))
        ));
    }

    #[test]
    fn controller_shows_chrome_notice_dialog() {
        let dir = unique_test_dir("tui-chrome-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/chrome")
            .expect("show chrome");

        assert_eq!(controller.status_note.as_deref(), Some("chrome"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Chrome"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/chrome"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Chrome") && text.contains("extension_url="))
        ));
    }

    #[test]
    fn controller_routes_permissions_shorthand() {
        let dir = unique_test_dir("tui-permissions-shorthand");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/permissions accept-edits")
            .expect("set permissions mode");

        assert_eq!(
            controller.state.permission_mode,
            PermissionMode::AcceptEdits
        );
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/permissions accept-edits"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("status=permission mode updated"))
        ));
    }

    #[test]
    fn controller_adds_additional_working_directory_from_slash_command() {
        let dir = unique_test_dir("tui-add-dir");
        let extra = dir.join("extra");
        std::fs::create_dir_all(&extra).expect("create extra dir");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/add-dir extra")
            .expect("execute add-dir");

        assert_eq!(controller.state.additional_working_directories.len(), 1);
        assert_eq!(
            controller.state.additional_working_directories[0].path,
            extra.canonicalize().expect("canonical extra dir")
        );
        assert_eq!(
            controller
                .tool_context()
                .additional_working_directories
                .len(),
            1
        );
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/add-dir extra"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("status=added working directory"))
        ));
    }

    #[test]
    fn controller_opens_memory_picker_for_bare_memory_command() {
        let dir = unique_test_dir("tui-memory-picker-open");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/memory")
            .expect("open memory picker");

        assert_eq!(
            controller.status_note.as_deref(),
            Some("memory: type to filter, use Up/Down to choose, Enter to select, Esc to cancel")
        );
        assert!(controller.pending_memory_picker.is_some());
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Memory"
        ));
        assert!(!controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::Command { input, .. } if input == "/memory"
            )
        }));
    }

    #[test]
    fn controller_keeps_theme_picker_open_when_search_has_no_matches() {
        let dir = unique_test_dir("tui-theme-picker-no-matches");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/theme")
            .expect("open theme picker");
        for ch in ['z', 'z', 'z'] {
            send_dialog_key(
                &mut controller,
                picker_key(KeyCode::Char(ch)),
                Some(ResolvedKey::InsertChar(ch)),
            );
        }
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Enter),
            Some(ResolvedKey::Edit(EditAction::InsertNewline)),
        );

        let picker = controller
            .pending_theme_picker
            .as_ref()
            .expect("theme picker still open");
        let dialog = controller.dialog.as_ref().expect("dialog");
        let expected_matches = format!("Matches: 0/{}", picker.options.len());
        assert_eq!(dialog.body.first().map(String::as_str), Some("Search: zzz"));
        assert_eq!(
            dialog.body.get(1).map(String::as_str),
            Some(expected_matches.as_str())
        );
        assert_eq!(
            dialog.body.get(2).map(String::as_str),
            Some("No matching themes.")
        );
        assert_eq!(
            controller.status_note.as_deref(),
            Some("theme picker: no matching option to select")
        );
    }

    #[test]
    fn controller_filters_memory_picker_with_search_query() {
        let dir = unique_test_dir("tui-memory-picker-filter");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/memory")
            .expect("open memory picker");
        for ch in ['u', 's', 'e', 'r'] {
            send_dialog_key(
                &mut controller,
                picker_key(KeyCode::Char(ch)),
                Some(ResolvedKey::InsertChar(ch)),
            );
        }

        let picker = controller
            .pending_memory_picker
            .as_ref()
            .expect("memory picker open");
        let dialog = controller.dialog.as_ref().expect("dialog");
        let expected_matches = format!("Matches: 1/{}", picker.options.len());
        assert_eq!(
            dialog.body.first().map(String::as_str),
            Some("Search: user")
        );
        assert_eq!(
            dialog.body.get(1).map(String::as_str),
            Some(expected_matches.as_str())
        );
        assert!(dialog.body.iter().any(|line| line.contains("User memory")));
        assert!(
            !dialog
                .body
                .iter()
                .any(|line| line.contains("Project memory"))
        );
    }

    #[test]
    fn controller_selects_memory_target_from_picker() {
        let dir = unique_test_dir("tui-memory-picker-select");
        let _editor = EnvVarGuard::set("EDITOR", "vi");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/memory")
            .expect("open picker");
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Enter),
            Some(ResolvedKey::Edit(EditAction::InsertNewline)),
        );

        assert!(controller.pending_memory_picker.is_none());
        assert!(controller.dialog.is_none());
        assert_eq!(
            controller.pending_external_editor.as_ref(),
            Some(&ExternalEditorRequest {
                cwd: dir.clone(),
                path: dir.join("config/CLAUDE.md"),
            })
        );
        assert_eq!(
            controller.status_note.as_deref(),
            Some("opening file in editor")
        );
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/memory"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("memory_target=user"))
        ));
    }

    #[test]
    fn controller_cancels_memory_picker() {
        let dir = unique_test_dir("tui-memory-picker-cancel");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/memory")
            .expect("open picker");
        controller
            .handle_dialog_key(
                KeyEvent {
                    code: KeyCode::Esc,
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                None,
                &mut |_| Ok(()),
            )
            .expect("cancel picker");

        assert!(controller.pending_memory_picker.is_none());
        assert!(controller.dialog.is_none());
        assert_eq!(
            controller.status_note.as_deref(),
            Some("memory picker cancelled")
        );
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/memory"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("status=memory picker cancelled"))
        ));
    }

    #[test]
    fn controller_opens_tag_removal_confirmation_for_matching_tag() {
        let dir = unique_test_dir("tui-tag-remove-confirm");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");
        controller.state.set_session_tags(vec!["bugfix".into()]);
        controller
            .persist_state_snapshot()
            .expect("persist tagged state");

        controller
            .execute_slash_command("/tag bugfix")
            .expect("open tag removal dialog");

        assert!(matches!(
            controller.pending_tag_removal.as_ref(),
            Some(pending) if pending.tag == "bugfix"
        ));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Remove tag?"
        ));
        assert!(!controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::Command { input, .. } if input == "/tag bugfix"
            )
        }));
    }

    #[test]
    fn controller_confirms_tag_removal() {
        let dir = unique_test_dir("tui-tag-remove-complete");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");
        controller.state.set_session_tags(vec!["bugfix".into()]);
        controller
            .persist_state_snapshot()
            .expect("persist tagged state");
        controller
            .execute_slash_command("/tag bugfix")
            .expect("open tag removal dialog");

        controller
            .handle_dialog_key(
                KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                Some(ResolvedKey::Edit(EditAction::InsertNewline)),
                &mut |_| Ok(()),
            )
            .expect("confirm tag removal");

        assert!(controller.pending_tag_removal.is_none());
        assert!(controller.state.session.tags.is_empty());
        assert_eq!(controller.status_note.as_deref(), Some("removed #bugfix"));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/tag bugfix"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("session_tags="))
        ));
    }

    #[test]
    fn controller_shows_context_notice_dialog() {
        let dir = unique_test_dir("tui-context-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/context")
            .expect("show context");

        assert_eq!(controller.status_note.as_deref(), Some("context usage"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Context Usage"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/context"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Context Usage"))
        ));
    }

    #[test]
    fn controller_shows_stats_notice_dialog() {
        let dir = unique_test_dir("tui-stats-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/stats")
            .expect("show stats");

        assert_eq!(controller.status_note.as_deref(), Some("activity stats"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Activity Stats"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/stats"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Activity Stats"))
        ));
    }

    #[test]
    fn controller_shows_usage_notice_dialog() {
        let dir = unique_test_dir("tui-usage-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/usage")
            .expect("show usage");

        assert_eq!(controller.status_note.as_deref(), Some("usage"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Usage"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/usage"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Usage"))
        ));
    }

    #[test]
    fn controller_shows_keybindings_notice_dialog() {
        let dir = unique_test_dir("tui-keybindings-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/keybindings")
            .expect("show keybindings");

        assert_eq!(controller.status_note.as_deref(), Some("keybindings"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Keybindings"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/keybindings"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Keybindings"))
        ));
    }

    #[test]
    fn controller_shows_hooks_notice_dialog() {
        let dir = unique_test_dir("tui-hooks-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/hooks")
            .expect("show hooks");

        assert_eq!(controller.status_note.as_deref(), Some("hooks"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Hooks"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/hooks"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Hooks"))
        ));
    }

    #[test]
    fn controller_shows_privacy_settings_notice_dialog() {
        let dir = unique_test_dir("tui-privacy-settings-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/privacy-settings")
            .expect("show privacy settings");

        assert_eq!(controller.status_note.as_deref(), Some("privacy settings"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Privacy Settings"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/privacy-settings"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Privacy Settings"))
        ));
    }

    #[test]
    fn controller_shows_terminal_setup_notice_dialog() {
        let dir = unique_test_dir("tui-terminal-setup-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/terminal-setup")
            .expect("show terminal setup");

        assert_eq!(controller.status_note.as_deref(), Some("terminal setup"));
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Terminal Setup"
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/terminal-setup"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("## Terminal Setup"))
        ));
    }

    #[test]
    fn controller_toggles_vim_mode_from_slash_command() {
        let dir = unique_test_dir("tui-vim-command");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        assert_eq!(controller.vim.mode(), VimMode::Insert);

        controller
            .execute_slash_command("/vim")
            .expect("toggle vim");
        assert_eq!(controller.vim.mode(), VimMode::Normal);
        assert_eq!(controller.status_note.as_deref(), Some("vim normal"));

        controller
            .execute_slash_command("/vim insert")
            .expect("set vim insert");
        assert_eq!(controller.vim.mode(), VimMode::Insert);
        assert_eq!(controller.status_note.as_deref(), Some("vim insert"));
    }

    #[test]
    fn controller_opens_permissions_picker() {
        let dir = unique_test_dir("tui-permissions-picker-open");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/permissions")
            .expect("open permissions picker");

        assert_eq!(
            controller.status_note.as_deref(),
            Some(
                "permission mode: type to filter, use Up/Down to choose, Enter to select, Esc to cancel"
            )
        );
        assert!(controller.pending_permission_picker.is_some());
        assert!(matches!(
            controller.dialog.as_ref(),
            Some(dialog) if dialog.title == "Permission mode"
        ));
        assert!(!controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::Command { input, .. } if input == "/permissions"
            )
        }));
    }

    #[test]
    fn controller_clears_permission_picker_search_back_to_full_list() {
        let dir = unique_test_dir("tui-permissions-picker-clear-search");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/permissions")
            .expect("open permissions picker");
        for ch in ['p', 'l', 'a', 'n'] {
            send_dialog_key(
                &mut controller,
                picker_key(KeyCode::Char(ch)),
                Some(ResolvedKey::InsertChar(ch)),
            );
        }
        for _ in 0..4 {
            send_dialog_key(
                &mut controller,
                picker_key(KeyCode::Backspace),
                Some(ResolvedKey::Edit(EditAction::Backspace)),
            );
        }

        let picker = controller
            .pending_permission_picker
            .as_ref()
            .expect("permission picker open");
        let dialog = controller.dialog.as_ref().expect("dialog");
        let expected_matches = format!("Matches: {0}/{0}", picker.options.len());
        assert_eq!(
            dialog.body.first().map(String::as_str),
            Some("Search: (all)")
        );
        assert_eq!(
            dialog.body.get(1).map(String::as_str),
            Some(expected_matches.as_str())
        );
        assert!(dialog.body.iter().any(|line| line.contains("Default")));
        assert!(dialog.body.iter().any(|line| line.contains("Plan")));
    }

    #[test]
    fn controller_selects_permission_mode_from_picker() {
        let dir = unique_test_dir("tui-permissions-picker-select");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/permissions")
            .expect("open permissions picker");
        send_dialog_key(&mut controller, picker_key(KeyCode::Down), None);
        send_dialog_key(
            &mut controller,
            picker_key(KeyCode::Enter),
            Some(ResolvedKey::Edit(EditAction::InsertNewline)),
        );

        assert!(controller.pending_permission_picker.is_none());
        assert!(controller.dialog.is_none());
        assert_eq!(
            controller.state.permission_mode,
            PermissionMode::AcceptEdits
        );
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/permissions"
                    && output
                        .as_deref()
                        .is_some_and(|text| text.contains("permission_mode=accept-edits"))
        ));
    }

    #[test]
    fn controller_routes_plan_mode_slash_commands() {
        let dir = unique_test_dir("tui-slash-plan");
        std::fs::write(dir.join("plan.md"), "# queued plan\n- keep parity\n").expect("write plan");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/plan")
            .expect("enter plan mode");
        assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/plan"
                    && output.as_deref().is_some_and(|text| text.contains("status=plan mode enabled"))
        ));

        controller
            .execute_slash_command("/plan")
            .expect("show current plan");
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/plan"
                    && output.as_deref().is_some_and(|text| {
                        text.contains("plan_exists=true")
                            && text.contains("Current Plan")
                            && text.contains("- keep parity")
                    })
        ));

        controller
            .execute_slash_command("/plan exit")
            .expect("exit plan mode");
        assert_eq!(controller.state.permission_mode, PermissionMode::Default);
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::Command { input, output })
                if input == "/plan exit"
                    && output.as_deref().is_some_and(|text| text.contains("status=plan mode disabled"))
        ));
    }

    #[test]
    fn controller_executes_queued_plan_prompt() {
        let dir = unique_test_dir("tui-slash-plan-prompt");
        let (api_base, handle) = spawn_json_sequence_server(
            |_index, _headers, body| {
                assert_eq!(body["model"], "claude-3-7-sonnet-latest");
                assert_eq!(
                    body.pointer("/messages/0/content/0/text")
                        .and_then(Value::as_str),
                    Some("draft the migration plan")
                );
            },
            vec![
                json!({
                    "id": "msg_plan_prompt_1",
                    "type": "message",
                    "role": "assistant",
                    "content": [{
                        "type": "text",
                        "text": "queued plan reply"
                    }],
                    "stop_reason": "end_turn",
                    "usage": {
                        "input_tokens": 8,
                        "output_tokens": 4
                    }
                })
                .to_string(),
            ],
        );
        write_provider_config_for(&dir, "anthropic", "claude-3-7-sonnet-latest", &api_base);
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        let mut render_calls = 0usize;
        controller
            .execute_slash_command_with("/plan draft the migration plan", &mut |_| {
                render_calls += 1;
                Ok(())
            })
            .expect("execute plan prompt");

        handle.join().expect("server join");

        assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
        assert!(render_calls >= 2);
        assert!(controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::Command { input, output }
                    if input == "/plan draft the migration plan"
                        && output.as_deref().is_some_and(|text| {
                            text.contains("status=plan mode enabled")
                                && text.contains("enqueue_prompt=draft the migration plan")
                        })
            )
        }));
        assert!(controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::UserText { content } if content == "draft the migration plan"
            )
        }));
        assert!(controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::AssistantText { content } if content == "queued plan reply"
            )
        }));
    }

    #[test]
    fn controller_queues_external_editor_for_plan_open() {
        let dir = unique_test_dir("tui-slash-plan-open");
        std::fs::write(dir.join("plan.md"), "# plan\n").expect("write plan");
        let _editor = EnvVarGuard::set("EDITOR", "vi");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/plan open")
            .expect("open plan");

        assert_eq!(
            controller.status_note.as_deref(),
            Some("opening file in editor")
        );
        assert_eq!(
            controller.pending_external_editor.as_ref(),
            Some(&ExternalEditorRequest {
                cwd: dir.clone(),
                path: dir.join("plan.md"),
            })
        );
        assert!(!controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::Command { input, .. } if input == "/plan open"
            )
        }));
    }

    #[test]
    fn controller_queues_external_editor_for_memory_open() {
        let dir = unique_test_dir("tui-slash-memory-open");
        let _editor = EnvVarGuard::set("EDITOR", "vi");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller
            .execute_slash_command("/memory open project")
            .expect("open project memory");

        assert_eq!(
            controller.status_note.as_deref(),
            Some("opening file in editor")
        );
        assert_eq!(
            controller.pending_external_editor.as_ref(),
            Some(&ExternalEditorRequest {
                cwd: dir.clone(),
                path: dir.join("CLAUDE.md"),
            })
        );
        assert!(!controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::Command { input, .. } if input == "/memory open project"
            )
        }));
    }

    #[test]
    fn clear_reloads_live_view_without_recording_command_message() {
        let dir = unique_test_dir("tui-slash-clear");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");
        let session_id = controller.state.session.id;
        let user = MessageEnvelope::user_text(session_id, "first message");
        let assistant = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "second message".into(),
            },
        );
        controller
            .state
            .push_message(user.clone())
            .expect("push user");
        controller
            .state
            .push_message(assistant.clone())
            .expect("push assistant");
        controller
            .persist_messages(&[user, assistant])
            .expect("persist messages");

        controller
            .execute_slash_command("/clear")
            .expect("clear view");

        assert_eq!(
            controller.status_note.as_deref(),
            Some("conversation cleared")
        );
        assert_eq!(controller.state.messages.len(), 1);
        assert!(matches!(
            &controller.state.messages[0].payload,
            MessagePayload::CompactBoundary { summary }
                if summary.contains("Cleared the visible transcript")
        ));
        assert!(!controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::Command { input, .. } if input == "/clear"
            )
        }));
    }

    #[test]
    fn compact_reloads_live_view_and_preserves_tail_messages() {
        let dir = unique_test_dir("tui-slash-compact");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");
        let session_id = controller.state.session.id;
        let user = MessageEnvelope::user_text(session_id, "older user");
        let assistant = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "older assistant".into(),
            },
        );
        let recent_user = MessageEnvelope::user_text(session_id, "recent user");
        let recent_assistant = MessageEnvelope::new(
            session_id,
            MessagePayload::AssistantText {
                content: "recent assistant".into(),
            },
        );
        for message in [
            user.clone(),
            assistant.clone(),
            recent_user.clone(),
            recent_assistant.clone(),
        ] {
            controller
                .state
                .push_message(message)
                .expect("push seeded message");
        }
        controller
            .persist_messages(&[
                user,
                assistant,
                recent_user.clone(),
                recent_assistant.clone(),
            ])
            .expect("persist messages");

        controller
            .execute_slash_command("/compact --keep-last 2")
            .expect("compact view");

        assert_eq!(
            controller.status_note.as_deref(),
            Some("conversation compacted")
        );
        assert_eq!(controller.state.messages.len(), 3);
        assert!(matches!(
            &controller.state.messages[0].payload,
            MessagePayload::CompactBoundary { summary }
                if summary.contains("Compacted")
        ));
        assert!(matches!(
            &controller.state.messages[1].payload,
            MessagePayload::UserText { content } if content == "recent user"
        ));
        assert!(matches!(
            &controller.state.messages[2].payload,
            MessagePayload::AssistantText { content } if content == "recent assistant"
        ));
        assert!(!controller.state.messages.iter().any(|message| {
            matches!(
                &message.payload,
                MessagePayload::Command { input, .. } if input == "/compact --keep-last 2"
            )
        }));
    }

    #[test]
    fn controller_submits_prompt_and_persists_session() {
        let dir = unique_test_dir("tui-prompt-submit");
        let (api_base, handle) = spawn_json_sequence_server(
            |_index, _headers, body| {
                assert_eq!(body["model"], "claude-3-7-sonnet-latest");
                assert_eq!(
                    body.pointer("/messages/0/content/0/text")
                        .and_then(Value::as_str),
                    Some("hello from tui")
                );
                assert_eq!(body["max_tokens"], 1024);
                assert_eq!(
                    body.pointer("/tool_choice/type").and_then(Value::as_str),
                    Some("auto")
                );
            },
            vec![
                json!({
                    "id": "msg_tui_1",
                    "type": "message",
                    "role": "assistant",
                    "content": [{
                        "type": "text",
                        "text": "hello back"
                    }],
                    "stop_reason": "end_turn",
                    "usage": {
                        "input_tokens": 4,
                        "output_tokens": 2
                    }
                })
                .to_string(),
            ],
        );
        write_provider_config_for(&dir, "anthropic", "claude-3-7-sonnet-latest", &api_base);
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller.prompt.insert_text("hello from tui");
        let mut render_calls = 0usize;
        controller
            .submit_prompt(&mut |_| {
                render_calls += 1;
                Ok(())
            })
            .expect("submit prompt");

        handle.join().expect("server join");

        assert_eq!(controller.prompt.text(), "");
        assert_eq!(controller.turn_state, TurnState::Completed);
        assert_eq!(controller.state.messages.len(), 2);
        assert!(render_calls >= 2);
        assert!(matches!(
            &controller.state.messages[0].payload,
            MessagePayload::UserText { content } if content == "hello from tui"
        ));
        assert!(matches!(
            &controller.state.messages[1].payload,
            MessagePayload::AssistantText { content } if content == "hello back"
        ));
        assert_eq!(controller.state.provider.as_deref(), Some("anthropic"));
        assert_eq!(
            controller.state.model.as_deref(),
            Some("claude-3-7-sonnet-latest")
        );
        assert_eq!(
            controller.status_note.as_deref(),
            Some("model response recorded")
        );

        let restored = TranscriptStore::new(&dir)
            .restore_session(controller.state.session.id)
            .expect("restore session");
        assert_eq!(restored.state.messages.len(), 2);
        assert_eq!(restored.metadata.entrypoint.as_deref(), Some("tui"));
    }

    #[test]
    fn controller_executes_tool_loop_and_persists_tool_messages() {
        let dir = unique_test_dir("tui-tool-loop");
        std::fs::write(dir.join("note.txt"), "hello from file\n").expect("write note");
        let (api_base, handle) = spawn_json_sequence_server(
            |request_index, _headers, body| match request_index {
                0 => {
                    assert_eq!(body["model"], "gpt-4.1");
                    assert_eq!(body["messages"][0]["content"], "read the note");
                    assert_eq!(body["tool_choice"], "auto");
                    assert!(
                        body["tools"]
                            .as_array()
                            .expect("tools array")
                            .iter()
                            .any(|tool| tool["function"]["name"] == "file_read")
                    );
                }
                1 => {
                    assert_eq!(body["messages"][0]["content"], "read the note");
                    assert_eq!(
                        body["messages"][1]["tool_calls"][0]["function"]["name"],
                        "file_read"
                    );
                    assert_eq!(body["messages"][2]["role"], "tool");
                    assert!(
                        body["messages"][2]["content"]
                            .as_str()
                            .expect("tool content")
                            .contains("hello from file")
                    );
                }
                other => panic!("unexpected request index {other}"),
            },
            vec![
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": null,
                            "tool_calls": [{
                                "id": "call_note",
                                "type": "function",
                                "function": {
                                    "name": "file_read",
                                    "arguments": "{\"path\":\"note.txt\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 12,
                        "completion_tokens": 3
                    }
                }))
                .expect("serialize tool response"),
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": {
                            "content": "The note says hello from file."
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 18,
                        "completion_tokens": 6
                    }
                }))
                .expect("serialize final response"),
            ],
        );
        write_provider_config(&dir, &api_base);
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller.prompt.insert_text("read the note");
        let mut render_calls = 0usize;
        controller
            .submit_prompt(&mut |_| {
                render_calls += 1;
                Ok(())
            })
            .expect("submit prompt");

        handle.join().expect("server join");

        assert_eq!(controller.turn_state, TurnState::Completed);
        assert!(render_calls >= 4);
        assert_eq!(controller.state.messages.len(), 4);
        assert!(matches!(
            &controller.state.messages[0].payload,
            MessagePayload::UserText { content } if content == "read the note"
        ));
        assert!(matches!(
            &controller.state.messages[1].payload,
            MessagePayload::AssistantToolUse { tool, input, .. }
                if tool == "file_read" && input["path"] == "note.txt"
        ));
        assert!(matches!(
            &controller.state.messages[2].payload,
            MessagePayload::ToolResult { tool, success, content, .. }
                if tool == "file_read" && *success && content.contains("hello from file")
        ));
        assert!(matches!(
            &controller.state.messages[3].payload,
            MessagePayload::AssistantText { content }
                if content == "The note says hello from file."
        ));
        assert_eq!(
            controller.status_note.as_deref(),
            Some("tool loop response recorded")
        );

        let restored = TranscriptStore::new(&dir)
            .restore_session(controller.state.session.id)
            .expect("restore session");
        assert_eq!(restored.state.messages.len(), 4);
        assert!(matches!(
            &restored.state.messages[1].payload,
            MessagePayload::AssistantToolUse { .. }
        ));
        assert!(matches!(
            &restored.state.messages[2].payload,
            MessagePayload::ToolResult { .. }
        ));
    }

    #[test]
    fn controller_approves_permission_and_resumes_tool_loop() {
        let dir = unique_test_dir("tui-tool-permission-approve");
        let (api_base, handle) = spawn_json_sequence_server(
            |request_index, _headers, body| match request_index {
                0 => {
                    assert_eq!(body["messages"][0]["content"], "write the note");
                    assert_eq!(
                        body["tools"]
                            .as_array()
                            .expect("tools array")
                            .iter()
                            .any(|tool| tool["function"]["name"] == "file_write"),
                        true
                    );
                }
                1 => {
                    assert_eq!(body["messages"][0]["content"], "write the note");
                    assert_eq!(
                        body["messages"][1]["tool_calls"][0]["function"]["name"],
                        "file_write"
                    );
                    assert_eq!(body["messages"][2]["role"], "tool");
                    assert!(
                        body["messages"][2]["content"]
                            .as_str()
                            .expect("tool content")
                            .contains("note.txt")
                    );
                }
                other => panic!("unexpected request index {other}"),
            },
            vec![
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": null,
                            "tool_calls": [{
                                "id": "call_write",
                                "type": "function",
                                "function": {
                                    "name": "file_write",
                                    "arguments": "{\"path\":\"note.txt\",\"content\":\"hello after approval\\n\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 3
                    }
                }))
                .expect("serialize tool response"),
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": {
                            "content": "The note has been written."
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 16,
                        "completion_tokens": 5
                    }
                }))
                .expect("serialize final response"),
            ],
        );
        write_provider_config(&dir, &api_base);
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller.prompt.insert_text("write the note");
        let mut render_calls = 0usize;
        controller
            .submit_prompt(&mut |_| {
                render_calls += 1;
                Ok(())
            })
            .expect("submit prompt");

        assert_eq!(controller.turn_state, TurnState::ToolPermissionPending);
        assert_eq!(controller.state.input_mode, InputMode::PermissionPending);
        assert!(controller.state.pending_tool_approval.is_some());
        let dialog = controller.view().dialog.expect("permission dialog");
        assert_eq!(dialog.title, "Permission: file_write");
        assert_eq!(
            controller.status_note.as_deref(),
            Some("approval required: file_write")
        );
        assert_eq!(
            dialog.body[0],
            "Tool `file_write` needs approval to continue."
        );
        assert!(!dialog.body[1].trim().is_empty());

        controller
            .handle_key_event(
                wonder_of_u_tui::KeyEvent {
                    code: wonder_of_u_tui::KeyCode::Enter,
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                &mut |_| {
                    render_calls += 1;
                    Ok(())
                },
            )
            .expect("approve permission");

        handle.join().expect("server join");

        assert_eq!(
            std::fs::read_to_string(dir.join("note.txt")).expect("read note"),
            "hello after approval\n"
        );
        assert_eq!(controller.turn_state, TurnState::Completed);
        assert_eq!(controller.state.input_mode, InputMode::Prompt);
        assert!(controller.state.pending_tool_approval.is_none());
        assert!(controller.view().dialog.is_none());
        assert!(render_calls >= 5);
        assert!(matches!(
            &controller.state.messages[2].payload,
            MessagePayload::Permission { tool, decision, .. }
                if tool == "file_write" && decision == "ask"
        ));
        assert!(matches!(
            &controller.state.messages[3].payload,
            MessagePayload::Permission { tool, decision, .. }
                if tool == "file_write" && decision == "allow"
        ));
        assert!(matches!(
            &controller.state.messages[4].payload,
            MessagePayload::ToolResult { tool, success, .. }
                if tool == "file_write" && *success
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::AssistantText { content })
                if content == "The note has been written."
        ));
        assert_eq!(
            controller.status_note.as_deref(),
            Some("tool loop response recorded")
        );
    }

    #[test]
    fn controller_denies_permission_and_resumes_tool_loop() {
        let dir = unique_test_dir("tui-tool-permission-deny");
        let (api_base, handle) = spawn_json_sequence_server(
            |request_index, _headers, body| match request_index {
                0 => {
                    assert_eq!(body["messages"][0]["content"], "write the note");
                }
                1 => {
                    assert_eq!(
                        body["messages"][1]["tool_calls"][0]["function"]["name"],
                        "file_write"
                    );
                    assert_eq!(body["messages"][2]["role"], "tool");
                    let content = body["messages"][2]["content"]
                        .as_str()
                        .expect("tool content");
                    assert!(content.contains("ERROR:"));
                    assert!(content.contains("denied by user"));
                }
                other => panic!("unexpected request index {other}"),
            },
            vec![
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": null,
                            "tool_calls": [{
                                "id": "call_write_deny",
                                "type": "function",
                                "function": {
                                    "name": "file_write",
                                    "arguments": "{\"path\":\"note.txt\",\"content\":\"should not be written\\n\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 3
                    }
                }))
                .expect("serialize tool response"),
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": {
                            "content": "Okay, I did not write the note."
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 16,
                        "completion_tokens": 5
                    }
                }))
                .expect("serialize final response"),
            ],
        );
        write_provider_config(&dir, &api_base);
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller.prompt.insert_text("write the note");
        controller
            .submit_prompt(&mut |_| Ok(()))
            .expect("submit prompt");

        let dialog = controller.view().dialog.expect("permission dialog");
        assert_eq!(dialog.title, "Permission: file_write");
        assert_eq!(
            controller.status_note.as_deref(),
            Some("approval required: file_write")
        );

        controller
            .handle_key_event(
                wonder_of_u_tui::KeyEvent {
                    code: wonder_of_u_tui::KeyCode::Esc,
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                &mut |_| Ok(()),
            )
            .expect("deny permission");

        handle.join().expect("server join");

        assert!(!dir.join("note.txt").exists());
        assert_eq!(controller.turn_state, TurnState::Completed);
        assert_eq!(controller.state.input_mode, InputMode::Prompt);
        assert!(controller.state.pending_tool_approval.is_none());
        assert!(controller.view().dialog.is_none());
        assert!(matches!(
            &controller.state.messages[2].payload,
            MessagePayload::Permission { tool, decision, .. }
                if tool == "file_write" && decision == "ask"
        ));
        assert!(matches!(
            &controller.state.messages[3].payload,
            MessagePayload::Permission { tool, decision, .. }
                if tool == "file_write" && decision == "deny"
        ));
        assert!(matches!(
            &controller.state.messages[4].payload,
            MessagePayload::ToolResult { tool, success, content, .. }
                if tool == "file_write"
                    && !success
                    && content.contains("tool execution denied by user")
        ));
        assert!(matches!(
            controller.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::AssistantText { content })
                if content == "Okay, I did not write the note."
        ));
        assert_eq!(
            controller.status_note.as_deref(),
            Some("tool loop response recorded")
        );
    }

    #[test]
    fn controller_restores_pending_permission_and_resumes_tool_loop() {
        let dir = unique_test_dir("tui-tool-permission-resume");
        let (api_base, handle) = spawn_json_sequence_server(
            |request_index, _headers, body| match request_index {
                0 => {
                    assert_eq!(body["messages"][0]["content"], "write the note");
                }
                1 => {
                    assert_eq!(
                        body["messages"][1]["tool_calls"][0]["function"]["name"],
                        "file_write"
                    );
                    assert_eq!(body["messages"][2]["role"], "tool");
                }
                other => panic!("unexpected request index {other}"),
            },
            vec![
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "tool_calls",
                        "message": {
                            "content": null,
                            "tool_calls": [{
                                "id": "call_write_resume",
                                "type": "function",
                                "function": {
                                    "name": "file_write",
                                    "arguments": "{\"path\":\"note.txt\",\"content\":\"hello after resume\\n\"}"
                                }
                            }]
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 3
                    }
                }))
                .expect("serialize tool response"),
                serde_json::to_string(&serde_json::json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": {
                            "content": "The note has been written after resume."
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 16,
                        "completion_tokens": 5
                    }
                }))
                .expect("serialize final response"),
            ],
        );
        write_provider_config(&dir, &api_base);
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");

        controller.prompt.insert_text("write the note");
        controller
            .submit_prompt(&mut |_| Ok(()))
            .expect("submit prompt");
        assert!(controller.state.pending_tool_approval.is_some());
        let session_id = controller.state.session.id.to_string();
        drop(controller);

        let mut restored = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions {
                session_id: Some(session_id),
            },
        )
        .expect("restored controller");

        assert_eq!(restored.turn_state, TurnState::ToolPermissionPending);
        assert_eq!(restored.state.input_mode, InputMode::PermissionPending);
        assert!(restored.state.pending_tool_approval.is_some());
        let dialog = restored.view().dialog.expect("permission dialog");
        assert_eq!(dialog.title, "Permission: file_write");
        assert_eq!(
            restored.status_note.as_deref(),
            Some("approval required: file_write")
        );
        assert_eq!(
            dialog.body[0],
            "Tool `file_write` needs approval to continue."
        );
        assert!(!dialog.body[1].trim().is_empty());

        restored
            .handle_key_event(
                wonder_of_u_tui::KeyEvent {
                    code: wonder_of_u_tui::KeyCode::Enter,
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                &mut |_| Ok(()),
            )
            .expect("approve restored permission");

        handle.join().expect("server join");

        assert_eq!(
            std::fs::read_to_string(dir.join("note.txt")).expect("read note"),
            "hello after resume\n"
        );
        assert_eq!(restored.turn_state, TurnState::Completed);
        assert_eq!(restored.state.input_mode, InputMode::Prompt);
        assert!(restored.state.pending_tool_approval.is_none());
        assert!(restored.view().dialog.is_none());
        assert!(matches!(
            restored.state.messages.last().map(|message| &message.payload),
            Some(MessagePayload::AssistantText { content })
                if content == "The note has been written after resume."
        ));
    }

    #[test]
    fn controller_opens_task_notice_when_task_finishes() {
        let dir = unique_test_dir("tui-task-notice");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");
        let store = TaskStore::new(&dir);
        let mut task = TaskState::pending("run tests");
        task.status = TaskStatus::Running;
        store.write_task(&task).expect("write running task");

        controller
            .refresh_runtime_state()
            .expect("load running task");
        assert!(controller.dialog.is_none());

        task.mark_finished(
            TaskStatus::Completed,
            Some(0),
            Some("all tests passed".into()),
        );
        store.write_task(&task).expect("write completed task");

        controller
            .refresh_runtime_state()
            .expect("load completed task");

        assert_eq!(controller.state.input_mode, InputMode::TaskNotification);
        assert_eq!(
            controller.status_note.as_deref(),
            Some("task completed: run tests")
        );
        let dialog = controller.view().dialog.expect("task notification dialog");
        assert_eq!(dialog.title, "Task update");
        assert!(dialog.body.iter().any(|line| line.contains("run tests")));
        assert!(
            dialog
                .body
                .iter()
                .any(|line| line.contains("all tests passed"))
        );

        controller
            .handle_dialog_key(
                KeyEvent {
                    code: KeyCode::Esc,
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                None,
                &mut |_| Ok(()),
            )
            .expect("dismiss task notice");

        assert!(controller.dialog.is_none());
        assert_eq!(controller.state.input_mode, InputMode::Prompt);
        assert_eq!(
            controller.status_note.as_deref(),
            Some("task update closed")
        );
    }

    #[test]
    fn controller_confirms_exit_when_session_has_activity() {
        let dir = unique_test_dir("tui-exit-confirm");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");
        controller.prompt.insert_text("unsent prompt");

        controller
            .handle_key_event(
                wonder_of_u_tui::KeyEvent {
                    code: wonder_of_u_tui::KeyCode::Char('c'),
                    modifiers: wonder_of_u_tui::KeyModifiers {
                        control: true,
                        ..wonder_of_u_tui::KeyModifiers::default()
                    },
                },
                &mut |_| Ok(()),
            )
            .expect("interrupt");

        assert!(!controller.exit_requested);
        assert_eq!(controller.status_note.as_deref(), Some("confirm exit"));
        assert!(controller.dialog.is_some());

        controller
            .handle_key_event(
                wonder_of_u_tui::KeyEvent {
                    code: wonder_of_u_tui::KeyCode::Enter,
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                &mut |_| Ok(()),
            )
            .expect("confirm exit");

        assert!(controller.exit_requested);
        assert_eq!(controller.turn_state, TurnState::Interrupted);
    }

    #[test]
    fn controller_executes_vim_normal_mode_edits() {
        let dir = unique_test_dir("tui-vim-mode");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");
        controller.prompt.insert_text("abc");

        controller
            .handle_key_event(
                wonder_of_u_tui::KeyEvent {
                    code: wonder_of_u_tui::KeyCode::Esc,
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                &mut |_| Ok(()),
            )
            .expect("enter normal mode");
        assert_eq!(controller.vim.mode(), VimMode::Normal);
        assert_eq!(controller.prompt.cursor(), 2);

        controller
            .handle_key_event(
                wonder_of_u_tui::KeyEvent {
                    code: wonder_of_u_tui::KeyCode::Char('x'),
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                &mut |_| Ok(()),
            )
            .expect("delete char");
        assert_eq!(controller.prompt.text(), "ab");

        controller
            .handle_key_event(
                wonder_of_u_tui::KeyEvent {
                    code: wonder_of_u_tui::KeyCode::Char('a'),
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                &mut |_| Ok(()),
            )
            .expect("append after cursor");
        assert_eq!(controller.vim.mode(), VimMode::Insert);

        controller
            .handle_key_event(
                wonder_of_u_tui::KeyEvent {
                    code: wonder_of_u_tui::KeyCode::Char('z'),
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                &mut |_| Ok(()),
            )
            .expect("insert after append");
        assert_eq!(controller.prompt.text(), "abz");
        assert_eq!(controller.status_note, None);
        assert!(controller.view().footer.contains("vim=insert"));
    }

    #[test]
    fn controller_cycles_prompt_history_on_history_search() {
        let dir = unique_test_dir("tui-history-search");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");
        let session_id = controller.state.session.id;
        controller
            .state
            .messages
            .push(MessageEnvelope::user_text(session_id, "first prompt"));
        controller
            .state
            .messages
            .push(MessageEnvelope::user_text(session_id, "second prompt"));

        controller
            .handle_system_action(wonder_of_u_tui::SystemAction::HistorySearch)
            .expect("history search first");
        assert_eq!(controller.prompt.text(), "second prompt");
        assert_eq!(controller.status_note.as_deref(), Some("history 1/2"));

        controller
            .handle_system_action(wonder_of_u_tui::SystemAction::HistorySearch)
            .expect("history search second");
        assert_eq!(controller.prompt.text(), "first prompt");
        assert_eq!(controller.status_note.as_deref(), Some("history 2/2"));
    }

    #[test]
    fn editing_resets_prompt_history_cycle() {
        let dir = unique_test_dir("tui-history-search-reset");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions { session_id: None },
        )
        .expect("controller");
        let session_id = controller.state.session.id;
        controller
            .state
            .messages
            .push(MessageEnvelope::user_text(session_id, "first prompt"));
        controller
            .state
            .messages
            .push(MessageEnvelope::user_text(session_id, "second prompt"));

        controller
            .handle_system_action(wonder_of_u_tui::SystemAction::HistorySearch)
            .expect("history search first");
        controller
            .handle_key_event(
                wonder_of_u_tui::KeyEvent {
                    code: wonder_of_u_tui::KeyCode::Char('!'),
                    modifiers: wonder_of_u_tui::KeyModifiers::default(),
                },
                &mut |_| Ok(()),
            )
            .expect("edit prompt");

        controller
            .handle_system_action(wonder_of_u_tui::SystemAction::HistorySearch)
            .expect("history search reset");
        assert_eq!(controller.prompt.text(), "second prompt");
        assert_eq!(controller.status_note.as_deref(), Some("history 1/2"));
    }

    #[test]
    fn controller_restores_permission_dialog_from_snapshot_resume() {
        let dir = unique_test_dir("tui-resume-permission-dialog");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut state = AppState::new(dir.clone());
        state.input_mode = InputMode::PermissionPending;
        state.pending_tool_approval = Some(PendingToolApprovalState {
            request_prompt: "write the note".into(),
            rounds: Vec::new(),
            current_round: PendingToolConversationRound {
                assistant_text: None,
                calls: vec![PendingProviderToolCall {
                    call_id: "call_bash".into(),
                    tool_name: "bash".into(),
                    arguments: serde_json::json!({ "command": "echo hi" }),
                }],
                results: Vec::new(),
            },
            pending_call: PendingLocalToolCall {
                provider_call: PendingProviderToolCall {
                    call_id: "call_bash".into(),
                    tool_name: "bash".into(),
                    arguments: serde_json::json!({ "command": "echo hi" }),
                },
                use_id: ToolUseId::new(),
            },
            remaining_calls: Vec::new(),
            reason: "workspace write requires approval".into(),
        });
        let permission = MessageEnvelope::new(
            state.session.id,
            MessagePayload::Permission {
                tool: "bash".into(),
                decision: "ask".into(),
                reason: "workspace write requires approval".into(),
            },
        );
        state
            .push_message(permission.clone())
            .expect("push permission");
        let store = TranscriptStore::new(&dir);
        store
            .write_metadata(
                &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 1),
            )
            .expect("write metadata");
        store
            .append_message(&permission)
            .expect("append transcript");
        store
            .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
                &state, 1, 0,
            ))
            .expect("write snapshot");

        let controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions {
                session_id: Some(state.session.id.to_string()),
            },
        )
        .expect("controller");

        assert_eq!(controller.state.input_mode, InputMode::PermissionPending);
        assert_eq!(controller.turn_state, TurnState::ToolPermissionPending);
        assert_eq!(
            controller.status_note.as_deref(),
            Some("approval required: bash")
        );
        let dialog = controller.view().dialog.expect("permission dialog");
        assert_eq!(dialog.title, "Permission: bash");
        assert_eq!(dialog.body[0], "Tool `bash` needs approval to continue.");
        assert_eq!(dialog.body[1], "workspace write requires approval");
        assert_eq!(
            dialog.body[2],
            "Allow to continue, or deny to continue without running it."
        );
    }

    #[test]
    fn controller_restores_task_notice_from_snapshot_resume() {
        let dir = unique_test_dir("tui-resume-task-dialog");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut state = AppState::new(dir.clone());
        state.input_mode = InputMode::TaskNotification;
        let mut task = TaskState::pending("run tests");
        task.mark_finished(
            TaskStatus::Completed,
            Some(0),
            Some("all tests passed".into()),
        );
        state.upsert_task(task.clone());
        let message = MessageEnvelope::system(state.session.id, "session resumed");
        state.push_message(message.clone()).expect("push message");
        let store = TranscriptStore::new(&dir);
        store
            .write_metadata(
                &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 1),
            )
            .expect("write metadata");
        store.append_message(&message).expect("append transcript");
        store
            .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
                &state, 1, 0,
            ))
            .expect("write snapshot");

        let controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions {
                session_id: Some(state.session.id.to_string()),
            },
        )
        .expect("controller");

        assert_eq!(controller.state.input_mode, InputMode::TaskNotification);
        assert_eq!(controller.turn_state, TurnState::Completed);
        assert_eq!(
            controller.status_note.as_deref(),
            Some("task completed: run tests")
        );
        let dialog = controller.view().dialog.expect("task dialog");
        assert_eq!(dialog.title, "Task update");
        assert!(dialog.body.iter().any(|line| line.contains("run tests")));
        assert!(
            dialog
                .body
                .iter()
                .any(|line| line.contains("all tests passed"))
        );
    }

    #[test]
    fn controller_restores_notice_dialog_from_snapshot_resume() {
        let dir = unique_test_dir("tui-resume-notice-dialog");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut state = AppState::new(dir.clone());
        let message = MessageEnvelope::new(
            state.session.id,
            MessagePayload::Command {
                input: "/context".into(),
                output: Some("## Context Usage\nTokens: 42\nWindow: 8 messages\n".into()),
            },
        );
        state.push_message(message.clone()).expect("push command");
        let store = TranscriptStore::new(&dir);
        store
            .write_metadata(
                &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 1),
            )
            .expect("write metadata");
        store.append_message(&message).expect("append transcript");
        store
            .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
                &state, 1, 0,
            ))
            .expect("write snapshot");

        let controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions {
                session_id: Some(state.session.id.to_string()),
            },
        )
        .expect("controller");

        assert_eq!(controller.state.input_mode, InputMode::Prompt);
        assert_eq!(controller.turn_state, TurnState::Completed);
        assert_eq!(controller.status_note.as_deref(), Some("context usage"));
        let dialog = controller.view().dialog.expect("restored notice dialog");
        assert_eq!(dialog.title, "Context Usage");
        assert!(dialog.body.iter().any(|line| line.contains("Tokens: 42")));
    }

    #[test]
    fn controller_restores_status_note_from_snapshot_resume() {
        let dir = unique_test_dir("tui-resume-status-note");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut state = AppState::new(dir.clone());
        state.set_session_color(Some("purple".into()));
        let message = MessageEnvelope::new(
            state.session.id,
            MessagePayload::Command {
                input: "/color purple".into(),
                output: Some("color=purple\nstatus=color updated\n".into()),
            },
        );
        state.push_message(message.clone()).expect("push command");
        let store = TranscriptStore::new(&dir);
        store
            .write_metadata(
                &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 1),
            )
            .expect("write metadata");
        store.append_message(&message).expect("append transcript");
        store
            .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
                &state, 1, 0,
            ))
            .expect("write snapshot");

        let controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions {
                session_id: Some(state.session.id.to_string()),
            },
        )
        .expect("controller");

        assert_eq!(controller.state.session_color.as_deref(), Some("purple"));
        assert_eq!(controller.status_note.as_deref(), Some("color purple"));
        assert!(controller.view().dialog.is_none());
        assert!(controller.view().footer.contains("color=purple"));
    }

    #[test]
    fn controller_preserves_restored_permission_mode_on_resume() {
        let dir = unique_test_dir("tui-resume-permission-mode");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut state = AppState::new(dir.clone());
        state.permission_mode = PermissionMode::Plan;
        let store = TranscriptStore::new(&dir);
        store.ensure_layout().expect("ensure layout");
        std::fs::write(store.paths().transcript_path(state.session.id), "")
            .expect("write transcript");
        store
            .write_metadata(
                &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 0),
            )
            .expect("write metadata");
        store
            .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
                &state, 0, 0,
            ))
            .expect("write snapshot");

        let controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions {
                session_id: Some(state.session.id.to_string()),
            },
        )
        .expect("controller");

        assert_eq!(controller.state.permission_mode, PermissionMode::Plan);
        assert!(controller.view().footer.contains("permission=plan"));
    }

    #[test]
    fn controller_preserves_restored_provider_selection_on_resume() {
        let dir = unique_test_dir("tui-resume-provider-selection");
        SettingsStore::new(&dir)
            .write(&AgentSettings {
                selected_provider: Some("openai".into()),
                selected_model: Some("gpt-4.1".into()),
                ..AgentSettings::default()
            })
            .expect("write settings");
        CredentialStore::new(&dir)
            .write(&StoredCredentials {
                providers: [
                    (
                        "openai".into(),
                        AuthMaterial::ApiKey {
                            key: "openai-key".into(),
                        },
                    ),
                    (
                        "anthropic".into(),
                        AuthMaterial::ApiKey {
                            key: "anthropic-key".into(),
                        },
                    ),
                ]
                .into(),
            })
            .expect("write credentials");
        let registry = commands::registry(Some(dir.clone())).expect("registry");
        let mut state = AppState::new(dir.clone());
        state.set_provider_context(
            Some("anthropic".into()),
            Some("claude-3-7-sonnet-latest".into()),
            AuthState::default(),
        );
        let store = TranscriptStore::new(&dir);
        store.ensure_layout().expect("ensure layout");
        std::fs::write(store.paths().transcript_path(state.session.id), "")
            .expect("write transcript");
        store
            .write_metadata(
                &wonder_of_u_storage::SessionMetadata::from_state_with_transcript(&state, 0),
            )
            .expect("write metadata");
        store
            .write_snapshot(&wonder_of_u_storage::SessionSnapshot::from_app_state(
                &state, 0, 0,
            ))
            .expect("write snapshot");

        let controller = TuiController::new(
            test_context(&dir),
            &registry,
            Some(dir.as_path()),
            TuiLaunchOptions {
                session_id: Some(state.session.id.to_string()),
            },
        )
        .expect("controller");

        assert_eq!(controller.state.provider.as_deref(), Some("anthropic"));
        assert_eq!(
            controller.state.model.as_deref(),
            Some("claude-3-7-sonnet-latest")
        );
        assert!(controller.state.auth.is_ready());
        assert!(
            controller
                .view()
                .status
                .contains("anthropic:claude-3-7-sonnet-latest")
        );
    }

    #[test]
    fn prompt_cursor_tracks_edit_position_inside_prompt_panel() {
        let (x, y) = prompt_cursor_position(40, 10, "abc", 2);
        assert_eq!((x, y), (1 + 2, 6));
    }
}
