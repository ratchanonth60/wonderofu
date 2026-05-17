use std::{
    collections::BTreeSet,
    env,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use clap::Parser;
use wonder_of_u_agent::{
    CompletionRequest, CompletionResponse, ProviderRuntime, ProviderSelection, ProviderToolCall,
    ProviderToolResultMessage, ProviderToolSpec, ToolConversationRound, ToolUseRequest,
    ToolUseResponse,
};
use wonder_of_u_core::{
    AGENT_TASK_RESULT_SCHEMA_VERSION, AgentTaskResult, AppState, Command, CommandContext,
    CommandInvocation, CommandKind, CommandOutput, CommandSpec, CoordinatorState, FeatureFlag,
    FleetId, FleetSteeringMessage, ForkContextSnapshot, MessageEnvelope, MessagePayload,
    PermissionDecision, PermissionMode, PromptSuggestion, QueryState, Result, TaskId, TaskStatus,
    ToolContext, ToolEffect, ToolQuery, ToolResult, ToolUseId, WONDER_OF_U_FORK_DEPTH_ENV,
    WonderError, best_prompt_suggestion,
};
use wonder_of_u_core::{MailboxKind, MailboxMessage};
use wonder_of_u_storage::{
    AgentTaskResultStore, CostStore, FleetStore, MailboxStore, SessionCostLedger,
    SessionMemoryIndexStore, SessionMetadata, SessionSnapshot, TaskStore, TranscriptStore,
};
use wonder_of_u_tools::{builtin_registry_with_mcp_catalog, provider_tool_specs};

use super::{
    apply_worktree_tool_result, detect_git_branch,
    fleet::direct_launch_fleet_request,
    hooks::{HookOutcome, POST_TOOL_USE, POST_TOOL_USE_FAILURE, PRE_TOOL_USE, run_hooks},
    parse_command_args, parse_session_id,
    task_runtime::WONDER_OF_U_AGENT_NAME_ENV,
};

const MAX_TOOL_LOOP_ITERATIONS: usize = 6;

pub(crate) struct PromptExecutionInput {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub session_id: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tool_use: bool,
    pub allowed_tools: Option<BTreeSet<String>>,
    pub prompt: String,
    pub session_title: String,
    pub entrypoint: &'static str,
}

pub(crate) struct PromptExecutionResult {
    pub state: AppState,
    pub response: CompletionResponse,
    pub persisted: bool,
    pub tool_use_requested: bool,
    pub tool_calls: usize,
    pub query: QueryState,
    pub coordinator: CoordinatorState,
    pub prompt_suggestion: Option<PromptSuggestion>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionPersistenceState {
    pub transcript_message_count: usize,
    pub transcript_warning_count: usize,
    pub persisted: bool,
}

pub(crate) struct PromptTurnInput {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tool_use: bool,
    pub allowed_tools: Option<BTreeSet<String>>,
    pub user_prompt: String,
    pub request_prompt: String,
}

pub(crate) struct PromptTurnResult {
    pub response: CompletionResponse,
    pub tool_calls: usize,
    pub query: QueryState,
    pub coordinator: CoordinatorState,
    pub prompt_suggestion: Option<PromptSuggestion>,
}

struct PromptOrchestrationState {
    query: QueryState,
    coordinator: CoordinatorState,
}

/// Represents prompt command
pub struct PromptCommand {
    storage_dir: Option<PathBuf>,
}

impl PromptCommand {
    /// Constant fn
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "prompt",
            "Send a non-interactive prompt through the configured provider runtime",
            CommandKind::Prompt,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::ModelProvider]);
        spec
    }
}

#[derive(Debug, Parser)]
struct PromptArgs {
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    system: Option<String>,
    #[arg(long)]
    session_id: Option<String>,
    #[arg(long)]
    max_output_tokens: Option<u32>,
    #[arg(long)]
    temperature: Option<f32>,
    #[arg(long, default_value_t = false)]
    tools: bool,
    #[arg(long, value_delimiter = ',')]
    allowed_tools: Vec<String>,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
    prompt: Vec<String>,
}

#[async_trait]
impl Command for PromptCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<PromptArgs>("prompt", &invocation)?;
        let allowed_tools = parse_allowed_tools(&args.allowed_tools)?;
        if allowed_tools.is_some() && !args.tools {
            return Err(WonderError::validation(
                "--allowed-tools requires --tools so the provider tool loop is enabled",
            ));
        }
        let prompt = args.prompt.join(" ");
        let result = execute_prompt_run(
            &context,
            self.storage_dir.as_deref(),
            PromptExecutionInput {
                provider: args.provider,
                model: args.model,
                system_prompt: args
                    .system
                    .and_then(|system| (!system.trim().is_empty()).then_some(system)),
                session_id: args.session_id,
                max_output_tokens: args.max_output_tokens,
                temperature: args.temperature,
                tool_use: args.tools,
                allowed_tools,
                session_title: prompt_title(&prompt),
                prompt,
                entrypoint: "prompt",
            },
        )?;

        Ok(CommandOutput::Text(render_prompt_output(&result)))
    }
}

pub(crate) fn execute_prompt_run(
    context: &CommandContext,
    storage_dir: Option<&Path>,
    input: PromptExecutionInput,
) -> Result<PromptExecutionResult> {
    validate_prompt_execution_input(&input)?;
    let (mut state, mut persistence) = load_or_create_state(
        context,
        storage_dir,
        input.session_id.as_deref(),
        &input.session_title,
        input.entrypoint,
    )?;

    // ── inbox polling ─────────────────────────────────────────────────────────
    // When this process was launched as a named agent subprocess, check for
    // any unread mailbox messages and prepend them to the conversation turn so
    // the model sees them immediately.  Each message is consumed exactly once
    // via MailboxStore::mark_read; storage errors are surfaced as System
    // transcript entries rather than silently swallowed.
    let inbox_preamble = storage_dir
        .map(|dir| poll_and_inject_inbox_messages(dir, &mut state, &mut persistence))
        .unwrap_or_default();
    let effective_prompt = if inbox_preamble.is_empty() {
        input.prompt
    } else {
        format!("{inbox_preamble}\n\n---\n\n{}", input.prompt)
    };
    // ─────────────────────────────────────────────────────────────────────────

    let turn_result = execute_prompt_turn(
        storage_dir,
        &mut state,
        &mut persistence,
        PromptTurnInput {
            provider: input.provider,
            model: input.model,
            system_prompt: input.system_prompt,
            max_output_tokens: input.max_output_tokens,
            temperature: input.temperature,
            tool_use: input.tool_use,
            allowed_tools: input.allowed_tools,
            user_prompt: effective_prompt.clone(),
            request_prompt: effective_prompt,
        },
    );

    // Best-effort: write a result sidecar so fleet inspectors can observe this task.
    let task_status = if turn_result.is_ok() {
        TaskStatus::Completed
    } else {
        TaskStatus::Failed
    };
    let _ = try_write_agent_task_result(storage_dir, &state, task_status);

    let result = turn_result?;

    Ok(PromptExecutionResult {
        state,
        response: result.response,
        persisted: persistence.persisted,
        tool_use_requested: input.tool_use,
        tool_calls: result.tool_calls,
        query: result.query,
        coordinator: result.coordinator,
        prompt_suggestion: result.prompt_suggestion,
    })
}

/// Writes an [`AgentTaskResult`] sidecar when the current process is an agent
/// subprocess spawned by a fleet or agent tool invocation.
///
/// Reads `WONDER_OF_U_TASK_ID` from the environment; if absent, returns `Ok(())`
/// silently (the process is not a fleet-managed agent).  All errors are returned
/// to the caller so they can be discarded with `let _ = ...`.
fn try_write_agent_task_result(
    storage_dir: Option<&Path>,
    state: &AppState,
    status: TaskStatus,
) -> Result<()> {
    let Some(storage_dir) = storage_dir else {
        return Ok(());
    };
    let task_id_str = match env::var("WONDER_OF_U_TASK_ID") {
        Ok(v) => v,
        Err(_) => return Ok(()), // not running inside a fleet-managed subprocess
    };
    let task_id = task_id_str
        .parse::<TaskId>()
        .map_err(|e| WonderError::validation(format!("invalid WONDER_OF_U_TASK_ID: {e}")))?;

    let fleet_id = env::var("WONDER_OF_U_FLEET_ID")
        .ok()
        .and_then(|s| s.parse::<FleetId>().ok());
    let fleet_request_id = env::var("WONDER_OF_U_FLEET_REQUEST_ID").ok();

    // Extract the last assistant message text as the result output.
    let output_text = state.messages.iter().rev().find_map(|msg| {
        if let MessagePayload::AssistantText { content } = &msg.payload {
            Some(content.clone())
        } else {
            None
        }
    });

    const EXCERPT_MAX: usize = 300;
    let output_excerpt = output_text
        .as_deref()
        .map(|t| truncate_chars(t, EXCERPT_MAX))
        .unwrap_or_default();

    let result = AgentTaskResult {
        schema_version: AGENT_TASK_RESULT_SCHEMA_VERSION,
        task_id,
        fleet_id,
        fleet_request_id,
        session_id: Some(state.session.id.to_string()),
        status,
        output_excerpt,
        output_text,
        provider: state.provider.clone(),
        model: state.model.clone(),
        finished_at: time::OffsetDateTime::now_utc(),
    };

    AgentTaskResultStore::new(storage_dir).write_result(&result)
}

pub(crate) fn execute_prompt_turn(
    storage_dir: Option<&Path>,
    state: &mut AppState,
    persistence: &mut SessionPersistenceState,
    mut input: PromptTurnInput,
) -> Result<PromptTurnResult> {
    validate_prompt_turn_input(&input)?;
    let coordinator = CoordinatorState::default();
    let mut orchestration = PromptOrchestrationState {
        query: QueryState::start(&input.user_prompt, coordinator.mode),
        coordinator,
    };

    let selection = ProviderSelection::new(input.provider.clone(), input.model.clone());
    let runtime = ProviderRuntime::new();
    let resolved = runtime.resolve_execution(storage_dir, selection)?;
    state.set_provider_context(
        Some(resolved.provider_id().to_string()),
        Some(resolved.model().to_string()),
        resolved.auth_state(),
    );
    input.system_prompt = state.effective_system_prompt(input.system_prompt);

    if input.tool_use && runtime.supports_tool_use_for(&resolved) {
        return execute_prompt_tool_loop(
            storage_dir,
            state,
            persistence,
            &runtime,
            &resolved,
            input,
            orchestration,
        );
    }

    let request = CompletionRequest {
        prompt: input.request_prompt,
        system_prompt: input.system_prompt,
        max_output_tokens: input.max_output_tokens,
        temperature: input.temperature,
        effort_level: None,
    };

    let response = runtime.complete(&resolved, &request)?;
    let user_message = append_contextual_message(
        state,
        MessagePayload::UserText {
            content: input.user_prompt,
        },
    )?;
    let assistant_message = append_contextual_message(
        state,
        MessagePayload::AssistantText {
            content: response.output_text.clone(),
        },
    )?;
    state.set_context_window_size(response.context_window_size);
    state.record_cost_usage(response.usage, None);
    persist_messages_and_state(
        storage_dir,
        state,
        persistence,
        &[user_message, assistant_message],
    )?;
    orchestration.query.complete();
    let prompt_suggestion = best_prompt_suggestion(state, &orchestration.query);

    Ok(PromptTurnResult {
        response,
        tool_calls: 0,
        query: orchestration.query,
        coordinator: orchestration.coordinator,
        prompt_suggestion,
    })
}

pub(crate) fn append_execution_metadata_lines(
    lines: &mut Vec<String>,
    state: &AppState,
    response: &CompletionResponse,
    persisted: bool,
) {
    if let (Some(provider), Some(model)) = (&state.provider, &state.model) {
        lines.push(format!("provider_selection={provider}:{model}"));
    }
    lines.push(format!("session_id={}", state.session.id));
    lines.push(format!("persisted={persisted}"));
    lines.push(format!("input_tokens={}", response.usage.input_tokens));
    lines.push(format!("output_tokens={}", response.usage.output_tokens));
    if response.usage.cache_creation_tokens > 0 {
        lines.push(format!(
            "cache_creation_tokens={}",
            response.usage.cache_creation_tokens
        ));
    }
    if response.usage.cache_read_tokens > 0 {
        lines.push(format!(
            "cache_read_tokens={}",
            response.usage.cache_read_tokens
        ));
    }
    lines.push(format!("total_tokens={}", response.usage.total_tokens()));
    if let Some(stop_reason) = &response.stop_reason {
        lines.push(format!("stop_reason={stop_reason}"));
    }
}

pub(crate) fn prompt_title(prompt: &str) -> String {
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let summary = truncate_chars(&normalized, 56);
    if summary.is_empty() {
        "Prompt session".into()
    } else {
        format!("Prompt: {summary}")
    }
}

pub(crate) fn truncate_chars(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let truncated = text.chars().take(keep).collect::<String>();
    format!("{truncated}…")
}

fn validate_prompt_execution_input(input: &PromptExecutionInput) -> Result<()> {
    validate_prompt_options(input.max_output_tokens, input.temperature)?;
    validate_prompt_text(&input.prompt)
}

fn validate_prompt_turn_input(input: &PromptTurnInput) -> Result<()> {
    validate_prompt_options(input.max_output_tokens, input.temperature)?;
    validate_prompt_text(&input.user_prompt)?;
    validate_prompt_text(&input.request_prompt)
}

fn validate_prompt_options(max_output_tokens: Option<u32>, temperature: Option<f32>) -> Result<()> {
    if max_output_tokens == Some(0) {
        return Err(WonderError::validation(
            "max output tokens must be greater than zero",
        ));
    }
    if temperature.is_some_and(|value| !value.is_finite()) {
        return Err(WonderError::validation(
            "temperature must be a finite number",
        ));
    }
    Ok(())
}

fn validate_prompt_text(prompt: &str) -> Result<()> {
    if prompt.trim().is_empty() {
        return Err(WonderError::validation("prompt cannot be empty"));
    }
    Ok(())
}

fn parse_allowed_tools(tools: &[String]) -> Result<Option<BTreeSet<String>>> {
    if tools.is_empty() {
        return Ok(None);
    }

    let mut allowed = BTreeSet::new();
    for tool in tools {
        let trimmed = tool.trim();
        if trimmed.is_empty() {
            return Err(WonderError::validation(
                "allowed tool names must not be empty",
            ));
        }
        allowed.insert(trimmed.to_ascii_lowercase());
    }
    Ok(Some(allowed))
}

pub(crate) fn load_or_create_state(
    context: &CommandContext,
    storage_dir: Option<&Path>,
    session_id: Option<&str>,
    session_title: &str,
    entrypoint: &str,
) -> Result<(AppState, SessionPersistenceState)> {
    if let Some(session_id) = session_id {
        let storage_dir = storage_dir.ok_or_else(|| {
            WonderError::validation(format!(
                "{entrypoint} --session-id requires --storage-dir so persisted transcripts can be loaded"
            ))
        })?;
        let store = TranscriptStore::new(storage_dir);
        let restored = store.restore_session(parse_session_id(session_id)?)?;
        let mut state = restored.state;
        std::env::set_current_dir(&state.session.cwd)?;
        state.features = context.features.clone();
        state.permission_mode =
            restored_permission_mode(context.permission_mode, state.permission_mode);
        state.session.entrypoint = Some(entrypoint.into());
        state.session.app_version = Some(env!("CARGO_PKG_VERSION").into());
        return Ok((
            state,
            SessionPersistenceState {
                transcript_message_count: restored.transcript.messages.len(),
                transcript_warning_count: restored.transcript.warnings.len(),
                persisted: true,
            },
        ));
    }

    let mut state = AppState::new(context.cwd.clone());
    state.features = context.features.clone();
    state.permission_mode = context.permission_mode;
    state.brief_mode = context.brief_mode;
    state.fast_mode = context.fast_mode;
    state.session.title = session_title.into();
    state.session.git_branch = detect_git_branch(&context.cwd);
    state.session.entrypoint = Some(entrypoint.into());
    state.session.app_version = Some(env!("CARGO_PKG_VERSION").into());
    Ok((
        state,
        SessionPersistenceState {
            transcript_message_count: 0,
            transcript_warning_count: 0,
            persisted: storage_dir.is_some(),
        },
    ))
}

fn restored_permission_mode(requested: PermissionMode, restored: PermissionMode) -> PermissionMode {
    if matches!(requested, PermissionMode::Default) {
        restored
    } else {
        requested
    }
}

pub(crate) fn append_contextual_message(
    state: &mut AppState,
    payload: MessagePayload,
) -> Result<MessageEnvelope> {
    let message = contextualize_message(state, MessageEnvelope::new(state.session.id, payload));
    state.push_message(message.clone())?;
    Ok(message)
}

pub(crate) fn contextualize_message(state: &AppState, message: MessageEnvelope) -> MessageEnvelope {
    message
        .with_context(
            Some(state.session.cwd.clone()),
            state.session.git_branch.clone(),
        )
        .with_runtime(
            state.session.entrypoint.clone(),
            state.session.app_version.clone(),
        )
}

fn append_hook_progress_message(
    state: &mut AppState,
    messages: &mut Vec<MessageEnvelope>,
    event: &str,
    tool_name: &str,
    hook_count: u32,
    success: bool,
) -> Result<()> {
    if hook_count == 0 {
        return Ok(());
    }

    messages.push(append_contextual_message(
        state,
        MessagePayload::HookProgress {
            event: event.into(),
            tool_name: tool_name.into(),
            hook_count,
            success,
        },
    )?);
    Ok(())
}

pub(crate) fn persist_messages_and_state(
    storage_dir: Option<&Path>,
    state: &AppState,
    persistence: &mut SessionPersistenceState,
    messages: &[MessageEnvelope],
) -> Result<()> {
    let Some(storage_dir) = storage_dir else {
        return Ok(());
    };

    let store = TranscriptStore::new(storage_dir);
    for message in messages {
        store.append_message(message)?;
    }
    persistence.transcript_message_count += messages.len();
    persist_prompt_state(&store, state, persistence)
}

pub(crate) fn persist_prompt_state(
    store: &TranscriptStore,
    state: &AppState,
    persistence: &SessionPersistenceState,
) -> Result<()> {
    store.write_metadata(&SessionMetadata::from_state_with_transcript(
        state,
        persistence.transcript_message_count,
    ))?;
    store.write_snapshot(&SessionSnapshot::from_app_state(
        state,
        persistence.transcript_message_count,
        persistence.transcript_warning_count,
    ))?;
    rebuild_session_memory_index(store, state, persistence.transcript_message_count)?;
    CostStore::new(store.paths().base_dir()).write_costs(&SessionCostLedger::from_app_state(state))
}

fn rebuild_session_memory_index(
    store: &TranscriptStore,
    state: &AppState,
    transcript_message_count: usize,
) -> Result<()> {
    let index_store = SessionMemoryIndexStore::new(store.paths().base_dir());
    if state.messages.len() == transcript_message_count {
        index_store.rebuild_from_messages(state.session.id, &state.messages)?;
    } else {
        index_store.rebuild_from_transcript(store, state.session.id)?;
    }
    Ok(())
}

fn render_prompt_output(result: &PromptExecutionResult) -> String {
    let mut lines = vec![result.response.output_text.clone(), String::new()];
    append_execution_metadata_lines(
        &mut lines,
        &result.state,
        &result.response,
        result.persisted,
    );
    append_orchestration_metadata_lines(
        &mut lines,
        &result.query,
        &result.coordinator,
        result.prompt_suggestion.as_ref(),
    );
    if result.tool_use_requested {
        lines.push(format!("tool_use_requested={}", result.tool_use_requested));
        lines.push(format!("tool_calls={}", result.tool_calls));
    }
    lines.join("\n")
}

pub(crate) fn append_orchestration_metadata_lines(
    lines: &mut Vec<String>,
    query: &QueryState,
    coordinator: &CoordinatorState,
    prompt_suggestion: Option<&PromptSuggestion>,
) {
    lines.push(format!("query_phase={}", query.phase.label()));
    lines.push(format!("query_tool_roundtrips={}", query.tool_roundtrips));
    lines.push(format!("coordinator_mode={}", coordinator.mode.label()));
    lines.push(format!(
        "coordinator_cloud_queries={}",
        coordinator.cloud_queries.support.label()
    ));
    lines.push(format!(
        "coordinator_external_backend={}",
        coordinator.external_backend.support.label()
    ));
    if let Some(prompt_suggestion) = prompt_suggestion {
        lines.push(format!("prompt_suggestion={}", prompt_suggestion.text));
        lines.push(format!(
            "prompt_suggestion_kind={}",
            prompt_suggestion.kind.label()
        ));
    }
}

fn execute_prompt_tool_loop(
    storage_dir: Option<&Path>,
    state: &mut AppState,
    persistence: &mut SessionPersistenceState,
    runtime: &ProviderRuntime,
    resolved: &wonder_of_u_agent::ResolvedProviderExecution,
    input: PromptTurnInput,
    mut orchestration: PromptOrchestrationState,
) -> Result<PromptTurnResult> {
    let PromptTurnInput {
        system_prompt,
        max_output_tokens,
        temperature,
        allowed_tools,
        user_prompt,
        request_prompt,
        ..
    } = input;
    let registry = match storage_dir {
        Some(root) => builtin_registry_with_mcp_catalog(root),
        // No persistent storage: fall back to built-ins only.
        None => wonder_of_u_tools::builtin_registry(),
    }?;
    let user_message = append_contextual_message(
        state,
        MessagePayload::UserText {
            content: user_prompt,
        },
    )?;
    let mut staged_messages = vec![user_message];
    let mut rounds = Vec::<ToolConversationRound>::new();
    let mut tool_calls = 0usize;

    for _ in 0..MAX_TOOL_LOOP_ITERATIONS {
        let provider_tools = provider_tool_specs(
            &registry,
            &tool_context(state, system_prompt.as_deref()),
            allowed_tools.as_ref(),
        )
        .into_iter()
        .map(tool_spec_to_provider_tool)
        .collect::<Vec<_>>();
        let response = runtime.complete_with_tool_use(
            resolved,
            &ToolUseRequest {
                prompt: request_prompt.clone(),
                system_prompt: system_prompt.clone(),
                max_output_tokens,
                temperature,
                tools: provider_tools.clone(),
                rounds: rounds.clone(),
                effort_level: None,
            },
        )?;

        match response {
            ToolUseResponse::Final(response) => {
                state.set_context_window_size(response.context_window_size);
                state.record_cost_usage(response.usage, None);
                let assistant_message = append_contextual_message(
                    state,
                    MessagePayload::AssistantText {
                        content: response.output_text.clone(),
                    },
                )?;
                staged_messages.push(assistant_message);
                persist_messages_and_state(storage_dir, state, persistence, &staged_messages)?;
                orchestration.query.complete();
                let prompt_suggestion = best_prompt_suggestion(state, &orchestration.query);
                return Ok(PromptTurnResult {
                    response,
                    tool_calls,
                    query: orchestration.query,
                    coordinator: orchestration.coordinator,
                    prompt_suggestion,
                });
            }
            ToolUseResponse::ToolCalls(batch) => {
                state.set_context_window_size(batch.context_window_size);
                state.record_cost_usage(batch.usage, None);
                orchestration
                    .query
                    .record_tool_batch(batch.calls.iter().map(|call| call.tool_name.clone()));
                let local_calls = batch
                    .calls
                    .iter()
                    .map(|call| LocalToolCall {
                        provider_call: call.clone(),
                        use_id: ToolUseId::new(),
                    })
                    .collect::<Vec<_>>();
                tool_calls += local_calls.len();

                if let Some(text) = batch
                    .assistant_text
                    .as_deref()
                    .filter(|text| !text.trim().is_empty())
                {
                    staged_messages.push(append_contextual_message(
                        state,
                        MessagePayload::AssistantText {
                            content: text.to_string(),
                        },
                    )?);
                }
                for call in &local_calls {
                    staged_messages.push(append_contextual_message(
                        state,
                        MessagePayload::AssistantToolUse {
                            tool: call.provider_call.tool_name.clone(),
                            use_id: call.use_id,
                            input: call.provider_call.arguments.clone(),
                        },
                    )?);
                }
                persist_messages_and_state(storage_dir, state, persistence, &staged_messages)?;
                staged_messages.clear();

                let mut round = ToolConversationRound {
                    assistant_text: batch.assistant_text.filter(|text| !text.trim().is_empty()),
                    calls: batch.calls,
                    results: Vec::new(),
                };
                for call in local_calls {
                    let result = execute_tool_call(
                        state,
                        storage_dir,
                        persistence,
                        &registry,
                        &tool_context(state, system_prompt.as_deref()),
                        &call,
                    )?;
                    round.results.push(result);
                }
                rounds.push(round);
            }
        }
    }

    let message = format!(
        "tool loop stopped after {MAX_TOOL_LOOP_ITERATIONS} iterations without a final assistant response"
    );
    let system_message = append_contextual_message(
        state,
        MessagePayload::System {
            content: message.clone(),
        },
    )?;
    persist_messages_and_state(storage_dir, state, persistence, &[system_message])?;
    orchestration.query.fail(message.clone());
    Err(WonderError::validation(message))
}

#[derive(Clone)]
struct LocalToolCall {
    provider_call: ProviderToolCall,
    use_id: ToolUseId,
}

fn tool_context(state: &AppState, system_prompt: Option<&str>) -> ToolContext {
    ToolContext {
        session_id: state.session.id,
        cwd: state.session.cwd.clone(),
        session_worktree: state.session.worktree.clone(),
        permission_mode: state.permission_mode,
        additional_working_directories: state.additional_working_directories.clone(),
        provider: state.provider.clone(),
        model: state.model.clone(),
        permission_rules: Vec::new(),
        features: state.features.clone(),
        bash_session_store: None,
        fork_context: build_fork_context_snapshot(state, system_prompt),
    }
}

/// Builds a [`ForkContextSnapshot`] representing the current session's context
/// for propagation to a fork-mode subagent child subprocess.
///
/// Reads the current fork depth from [`WONDER_OF_U_FORK_DEPTH_ENV`] (defaulting
/// to 0 if absent), captures the parent session identity, truncates the system
/// prompt to [`FORK_SYSTEM_PROMPT_CAP_BYTES`], and derives a compact conversation
/// summary from the last 6 user+assistant text exchange pairs.
///
/// # Examples
///
/// ```ignore
/// let snapshot = build_fork_context_snapshot(&state, Some("You are helpful."));
/// assert!(snapshot.is_some());
/// ```
pub(crate) fn build_fork_context_snapshot(
    state: &AppState,
    system_prompt: Option<&str>,
) -> Option<ForkContextSnapshot> {
    use wonder_of_u_core::FORK_SYSTEM_PROMPT_CAP_BYTES;

    let fork_depth: u32 = std::env::var(WONDER_OF_U_FORK_DEPTH_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let parent_session_id = state.session.id.to_string();
    let parent_entrypoint = state.session.entrypoint.clone();

    let parent_system_prompt =
        system_prompt.map(|sp| truncate_utf8_bytes(sp, FORK_SYSTEM_PROMPT_CAP_BYTES));

    // Derive a compact summary from the last 6 user+assistant text pairs.
    let conversation_summary = {
        let pairs: Vec<String> = state
            .messages
            .iter()
            .filter_map(|msg| match &msg.payload {
                MessagePayload::UserText { content } => Some(format!("User: {}", content.trim())),
                MessagePayload::AssistantText { content } => {
                    Some(format!("Assistant: {}", content.trim()))
                }
                _ => None,
            })
            .rev()
            .take(12) // last 6 pairs = 12 messages
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if pairs.is_empty() {
            None
        } else {
            Some(truncate_utf8_bytes(
                &pairs.join("\n"),
                FORK_SYSTEM_PROMPT_CAP_BYTES,
            ))
        }
    };

    Some(ForkContextSnapshot {
        parent_session_id,
        fork_depth,
        parent_system_prompt,
        conversation_summary,
        parent_entrypoint,
    })
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

/// Processes any [`ToolEffect`]s carried in `result` and returns an updated
/// [`ToolResult`] with the effects drained.
///
/// For each [`ToolEffect::LaunchAgentTask`]:
///
/// * If `storage_dir` is `Some` and `direct_launch_fleet_request` succeeds, the
///   result content and metadata are updated to reflect the live task id.
/// * If `app_state` is `Some` and the request carries a non-empty `name`, the
///   new `task_id` is registered in [`AppState`] for local `SendMessage` routing.
/// * Otherwise the request is queued as a pending file (fallback) so `fleet
///   dispatch` can recover it later.
///
/// For each [`ToolEffect::SendAgentMessage`]:
///
/// * When `app_state` is `Some` and the recipient name resolves to a local task
///   via [`AppState::lookup_agent_by_name`], the message is appended to the
///   agent's mailbox inbox via [`MailboxStore`].  The fleet steering record is
///   also written as an immutable audit trail.
/// * When the recipient is not a locally-registered agent, the message falls
///   back to the existing fleet steering path unchanged.
/// * Broadcast (`to = "*"`) always uses fleet steering only.
///
/// Effects are always drained before this function returns so they are never
/// persisted to the transcript.
pub(crate) fn process_tool_effects(
    mut result: ToolResult,
    storage_dir: Option<&Path>,
    cwd: &Path,
    mut app_state: Option<&mut AppState>,
) -> ToolResult {
    if result.effects.is_empty() {
        return result;
    }

    // Drain effects — process each one, updating result in place.
    let effects = std::mem::take(&mut result.effects);
    for effect in effects {
        match effect {
            ToolEffect::LaunchAgentTask(spec) => {
                if let Some(dir) = storage_dir {
                    match direct_launch_fleet_request(dir, &spec.request, cwd) {
                        Ok(task_id) => {
                            result.content = format!("agent task launched: {task_id}");
                            let meta = ensure_meta(&mut result);
                            meta.insert(
                                "status".into(),
                                serde_json::Value::String("launched".into()),
                            );
                            meta.insert(
                                "task_id".into(),
                                serde_json::Value::String(task_id.to_string()),
                            );
                            meta.remove("dispatch_hint");

                            // Register explicit agent name → task_id for local
                            // SendMessage routing.  A name conflict is non-fatal;
                            // the steering audit trail is still intact.
                            if let (Some(state), Some(name)) =
                                (app_state.as_deref_mut(), &spec.request.name)
                            {
                                if !name.is_empty() {
                                    let _ = state.register_agent_name(name.clone(), task_id);
                                }
                            }
                        }
                        Err(launch_err) => {
                            // Direct launch failed — fall back to pending queue.
                            let store = FleetStore::new(dir);
                            match store.queue_member_request(&spec.request) {
                                Ok(()) => {
                                    let request_id = spec.request.id.clone();
                                    result.content = format!(
                                        "agent task queued for dispatch (launch failed: {launch_err})"
                                    );
                                    let meta = ensure_meta(&mut result);
                                    meta.insert(
                                        "status".into(),
                                        serde_json::Value::String("pending_dispatch".into()),
                                    );
                                    meta.insert(
                                        "request_id".into(),
                                        serde_json::Value::String(request_id),
                                    );
                                    meta.insert(
                                        "dispatch_hint".into(),
                                        serde_json::Value::String(
                                            "run `fleet dispatch` to launch".into(),
                                        ),
                                    );
                                }
                                Err(queue_err) => {
                                    // Both paths failed — surface combined error in content.
                                    result.success = false;
                                    result.content = format!(
                                        "agent task could not be launched ({launch_err}) \
                                         and could not be queued ({queue_err})"
                                    );
                                }
                            }
                        }
                    }
                } else {
                    // No storage_dir — try to resolve one from environment variables
                    // using the same priority chain as the tools crate's `app_root`.
                    let fallback_dir = std::env::var_os("WONDER_OF_U_STORAGE_DIR")
                        .map(PathBuf::from)
                        .or_else(|| {
                            std::env::var_os("XDG_CONFIG_HOME")
                                .map(|d| PathBuf::from(d).join("wonder-of-u"))
                        })
                        .or_else(|| {
                            std::env::var_os("HOME")
                                .map(|d| PathBuf::from(d).join(".config").join("wonder-of-u"))
                        });

                    if let Some(dir) = fallback_dir {
                        let store = FleetStore::new(&dir);
                        match store.queue_member_request(&spec.request) {
                            Ok(()) => {
                                let request_id = spec.request.id.clone();
                                result.content =
                                    "agent task queued for dispatch (no session storage)".into();
                                let meta = ensure_meta(&mut result);
                                meta.insert(
                                    "status".into(),
                                    serde_json::Value::String("pending_dispatch".into()),
                                );
                                meta.insert(
                                    "request_id".into(),
                                    serde_json::Value::String(request_id),
                                );
                            }
                            Err(e) => {
                                result.success = false;
                                result.content =
                                    format!("agent task could not be queued (no storage dir): {e}");
                            }
                        }
                    } else {
                        result.success = false;
                        result.content =
                            "agent task could not be queued: no storage directory configured"
                                .into();
                    }
                }
            }
            ToolEffect::SendAgentMessage(spec) => {
                // ── Step 1: attempt local mailbox delivery ────────────────────
                // Non-broadcast recipients are resolved through the agent name
                // registry first.  When found, the message is appended to the
                // agent's local mailbox inbox AND written as a fleet steering
                // record for the immutable audit trail.
                let sender_task_id_str = std::env::var("WONDER_OF_U_TASK_ID").ok();
                let sender_identity = sender_task_id_str.as_deref().unwrap_or("unknown");

                let local_target = if spec.to != "*" {
                    app_state
                        .as_deref()
                        .and_then(|s| s.lookup_agent_by_name(&spec.to))
                } else {
                    None
                };

                if let (Some(target_task_id), Some(dir)) = (local_target, storage_dir) {
                    // Build the mailbox message.  Subject defaults to the
                    // summary when provided, or the first 80 chars of content.
                    let subject = spec.summary.as_deref().unwrap_or_else(|| {
                        let end = spec
                            .content
                            .char_indices()
                            .nth(80)
                            .map(|(i, _)| i)
                            .unwrap_or(spec.content.len());
                        &spec.content[..end]
                    });
                    let mailbox_msg =
                        MailboxMessage::new(sender_identity, &spec.to, subject, &spec.content);
                    let mailbox_id = mailbox_msg.id;

                    let mailbox_store = MailboxStore::new(dir);
                    let mailbox_outcome =
                        mailbox_store.append(MailboxKind::Agent, &spec.to, &mailbox_msg);

                    match mailbox_outcome {
                        Ok(()) => {
                            // Also write the fleet steering record as an audit
                            // trail — best-effort; don't fail the delivery if
                            // fleet context is absent.
                            let fleet_id_str = std::env::var("WONDER_OF_U_FLEET_ID").ok();
                            let sender_tid = sender_task_id_str
                                .as_deref()
                                .and_then(|s| s.parse::<TaskId>().ok());
                            let steering_id = if let Some(fid_str) = fleet_id_str {
                                if let Ok(fleet_id) = fid_str.parse::<FleetId>() {
                                    let fleet_store = FleetStore::new(dir);
                                    if let Ok(run) = fleet_store.read_run(fleet_id) {
                                        if !run.status.is_terminal() {
                                            let steering_msg = FleetSteeringMessage::new_agent(
                                                fleet_id,
                                                &spec.content,
                                                spec.to.clone(),
                                                spec.summary.clone(),
                                                sender_tid,
                                                &spec.message_kind,
                                            );
                                            let sid = steering_msg.id.clone();
                                            let _ =
                                                fleet_store.write_steering_message(&steering_msg);
                                            Some(sid)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            result.content = format!(
                                "message delivered to agent \"{}\"\u{2019}s mailbox inbox",
                                spec.to
                            );
                            let meta = ensure_meta(&mut result);
                            meta.insert(
                                "status".into(),
                                serde_json::Value::String("delivered_mailbox".into()),
                            );
                            meta.insert(
                                "mailbox_id".into(),
                                serde_json::Value::String(mailbox_id.to_string()),
                            );
                            meta.insert(
                                "recipient".into(),
                                serde_json::Value::String(spec.to.clone()),
                            );
                            meta.insert(
                                "recipient_task_id".into(),
                                serde_json::Value::String(target_task_id.to_string()),
                            );
                            meta.insert("is_local_agent".into(), serde_json::Value::Bool(true));
                            meta.insert("live_delivery".into(), serde_json::Value::Bool(false));
                            if let Some(sid) = steering_id {
                                meta.insert("steering_id".into(), serde_json::Value::String(sid));
                            }
                            continue;
                        }
                        Err(e) => {
                            // Mailbox name sanitisation or I/O failure — fall
                            // through to the fleet steering path below with a
                            // note in metadata.
                            let meta = ensure_meta(&mut result);
                            meta.insert(
                                "mailbox_error".into(),
                                serde_json::Value::String(e.to_string()),
                            );
                        }
                    }
                }

                // ── Step 2: fleet steering path (audit + non-local fallback) ──
                // Read fleet context injected by the task runtime at launch.
                let fleet_id_str = std::env::var("WONDER_OF_U_FLEET_ID").ok();
                let sender_task_id_str = std::env::var("WONDER_OF_U_TASK_ID").ok();

                let (fleet_id_str, dir) = match (fleet_id_str, storage_dir) {
                    (None, _) => {
                        // No fleet context and not a local agent → unknown recipient.
                        let is_local = local_target.is_some();
                        result.success = false;
                        result.content = if is_local {
                            "send_message: WONDER_OF_U_FLEET_ID is not set; \
                             message not written"
                                .into()
                        } else {
                            format!(
                                "send_message: recipient \"{}\" is not a locally-registered \
                                 agent and WONDER_OF_U_FLEET_ID is not set; message not written",
                                spec.to
                            )
                        };
                        let meta = ensure_meta(&mut result);
                        meta.insert(
                            "status".into(),
                            serde_json::Value::String("failed_no_fleet_context".into()),
                        );
                        meta.insert("live_delivery".into(), serde_json::Value::Bool(false));
                        meta.insert("is_local_agent".into(), serde_json::Value::Bool(false));
                        continue;
                    }
                    (_, None) => {
                        result.success = false;
                        result.content = "send_message: no storage directory configured; \
                             message not written"
                            .into();
                        let meta = ensure_meta(&mut result);
                        meta.insert(
                            "status".into(),
                            serde_json::Value::String("failed_no_storage".into()),
                        );
                        meta.insert("live_delivery".into(), serde_json::Value::Bool(false));
                        meta.insert("is_local_agent".into(), serde_json::Value::Bool(false));
                        continue;
                    }
                    (Some(fid), Some(d)) => (fid, d),
                };

                let fleet_id = match fleet_id_str.parse::<FleetId>() {
                    Ok(id) => id,
                    Err(_) => {
                        result.success = false;
                        result.content = format!(
                            "send_message: invalid WONDER_OF_U_FLEET_ID={fleet_id_str}; \
                             message not written"
                        );
                        continue;
                    }
                };

                let store = FleetStore::new(dir);
                match store.read_run(fleet_id) {
                    Err(e) => {
                        result.success = false;
                        result.content = format!(
                            "send_message: fleet {fleet_id} not found: {e}; \
                             message not written"
                        );
                        set_send_message_failure_metadata(
                            &mut result,
                            "failed_fleet_not_found",
                            "fleet not found; message not written",
                        );
                    }
                    Ok(run) if run.status.is_terminal() => {
                        result.success = false;
                        result.content = format!(
                            "send_message: fleet {fleet_id} is {} (terminal); \
                             message not written",
                            run.status.label(),
                        );
                        set_send_message_failure_metadata(
                            &mut result,
                            "failed_fleet_terminal",
                            "fleet is terminal; message not written",
                        );
                    }
                    Ok(_) => {
                        let sender_task_id = sender_task_id_str
                            .as_deref()
                            .and_then(|s| s.parse::<TaskId>().ok());
                        let msg = FleetSteeringMessage::new_agent(
                            fleet_id,
                            &spec.content,
                            spec.to.clone(),
                            spec.summary.clone(),
                            sender_task_id,
                            &spec.message_kind,
                        );
                        let steering_id = msg.id.clone();
                        match store.write_steering_message(&msg) {
                            Ok(()) => {
                                result.content = "message queued as advisory fleet steering; \
                                     not live-delivered to any running task"
                                    .into();
                                let meta = ensure_meta(&mut result);
                                meta.insert(
                                    "status".into(),
                                    serde_json::Value::String("queued_advisory".into()),
                                );
                                meta.insert(
                                    "fleet_id".into(),
                                    serde_json::Value::String(fleet_id.to_string()),
                                );
                                meta.insert(
                                    "steering_id".into(),
                                    serde_json::Value::String(steering_id),
                                );
                                meta.insert("live_delivery".into(), serde_json::Value::Bool(false));
                                meta.insert(
                                    "is_local_agent".into(),
                                    serde_json::Value::Bool(false),
                                );
                            }
                            Err(e) => {
                                result.success = false;
                                result.content =
                                    format!("send_message: could not write steering message: {e}");
                                set_send_message_failure_metadata(
                                    &mut result,
                                    "failed_write",
                                    "could not write steering message",
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

/// Ensures `result.metadata` is a JSON object and returns a mutable reference
/// to the underlying map.  Converts `null` or non-object values in-place.
fn ensure_meta(result: &mut ToolResult) -> &mut serde_json::Map<String, serde_json::Value> {
    if !result.metadata.is_object() {
        result.metadata = serde_json::Value::Object(serde_json::Map::new());
    }
    result
        .metadata
        .as_object_mut()
        .expect("just initialized as object")
}

fn set_send_message_failure_metadata(result: &mut ToolResult, status: &str, delivery_note: &str) {
    let meta = ensure_meta(result);
    meta.insert(
        "status".into(),
        serde_json::Value::String(status.to_owned()),
    );
    meta.insert("live_delivery".into(), serde_json::Value::Bool(false));
    meta.insert("delivery_attempted".into(), serde_json::Value::Bool(false));
    meta.insert(
        "delivery_note".into(),
        serde_json::Value::String(delivery_note.to_owned()),
    );
}

fn execute_tool_call(
    state: &mut AppState,
    storage_dir: Option<&Path>,
    persistence: &mut SessionPersistenceState,
    registry: &wonder_of_u_core::ToolRegistry,
    context: &ToolContext,
    call: &LocalToolCall,
) -> Result<ProviderToolResultMessage> {
    let query = ToolQuery::from(context);
    let mut messages = Vec::new();
    let result = if let Some(tool) = registry.resolve_enabled(&call.provider_call.tool_name, &query)
    {
        if let Err(error) = tool.validate_input(&call.provider_call.arguments) {
            ToolResult::failure(
                call.use_id,
                format!("tool input validation failed: {error}"),
            )
        } else {
            // PreToolUse hooks run before permission checks so they can augment
            // or block the call before any user interaction.
            let pre_hook_report = run_hooks(
                PRE_TOOL_USE,
                &call.provider_call.tool_name,
                &call.provider_call.arguments,
                None,
                &context.cwd,
                storage_dir,
            );
            append_hook_progress_message(
                state,
                &mut messages,
                PRE_TOOL_USE,
                &call.provider_call.tool_name,
                pre_hook_report.hook_count,
                pre_hook_report.success,
            )?;
            match pre_hook_report.outcome {
                HookOutcome::Block { reason } => {
                    ToolResult::failure(call.use_id, format!("hook blocked tool: {reason}"))
                }
                HookOutcome::Allow => {
                    let tool_result = match tool
                        .permission_decision(context, &call.provider_call.arguments)
                    {
                        PermissionDecision::Allow { .. } => {
                            let tool = tool.clone();
                            let context = context.clone();
                            let arguments = call.provider_call.arguments.clone();
                            let use_id = call.use_id;
                            match std::thread::spawn(move || {
                                futures::executor::block_on(
                                    tool.execute(context, use_id, arguments),
                                )
                            })
                            .join()
                            {
                                Ok(Ok(result)) => result,
                                Ok(Err(error)) => ToolResult::failure(
                                    call.use_id,
                                    format!("tool execution failed: {error}"),
                                ),
                                Err(_) => ToolResult::failure(
                                    call.use_id,
                                    "tool execution thread panicked",
                                ),
                            }
                        }
                        other => {
                            let reason = other.reason().to_string();
                            messages.push(append_contextual_message(
                                state,
                                MessagePayload::Permission {
                                    tool: call.provider_call.tool_name.clone(),
                                    decision: permission_decision_label(&other).into(),
                                    reason: reason.clone(),
                                },
                            )?);
                            ToolResult::failure(
                                call.use_id,
                                match other {
                                    PermissionDecision::Ask { .. } => {
                                        format!("tool execution blocked pending approval: {reason}")
                                    }
                                    PermissionDecision::Deny { .. } => {
                                        format!("tool execution denied: {reason}")
                                    }
                                    PermissionDecision::Allow { .. } => unreachable!(),
                                },
                            )
                        }
                    };

                    // PostToolUse / PostToolUseFailure hooks run after execution.
                    let post_event = if tool_result.success {
                        POST_TOOL_USE
                    } else {
                        POST_TOOL_USE_FAILURE
                    };
                    // Include a minimal tool_response snapshot so hooks can
                    // inspect the outcome without needing to re-run the tool.
                    let tool_response_json = serde_json::json!({
                        "success": tool_result.success,
                        "content": tool_result.content,
                    });
                    let post_hook_report = run_hooks(
                        post_event,
                        &call.provider_call.tool_name,
                        &call.provider_call.arguments,
                        Some(&tool_response_json),
                        &context.cwd,
                        storage_dir,
                    );
                    append_hook_progress_message(
                        state,
                        &mut messages,
                        post_event,
                        &call.provider_call.tool_name,
                        post_hook_report.hook_count,
                        post_hook_report.success,
                    )?;

                    tool_result
                }
            }
        }
    } else {
        ToolResult::failure(
            call.use_id,
            format!(
                "tool `{}` is not available in this session",
                call.provider_call.tool_name
            ),
        )
    };

    let result = process_tool_effects(result, storage_dir, &context.cwd, Some(state));
    let _ = apply_worktree_tool_result(state, &result)?;
    messages.push(append_contextual_message(
        state,
        MessagePayload::ToolResult {
            tool: call.provider_call.tool_name.clone(),
            use_id: call.use_id,
            success: result.success,
            content: result.content.clone(),
        },
    )?);
    persist_messages_and_state(storage_dir, state, persistence, &messages)?;
    Ok(ProviderToolResultMessage {
        call_id: call.provider_call.call_id.clone(),
        content: render_provider_tool_result(&result),
    })
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

// ── Inbox polling ─────────────────────────────────────────────────────────────

/// Polls the agent mailbox inbox for unread messages and returns a formatted
/// preamble to prepend to the conversation turn.
///
/// # Behaviour
///
/// * Reads `WONDER_OF_U_AGENT_NAME` from the environment.  When absent or empty
///   (i.e. the process was not launched as a named agent subprocess) the
///   function returns an empty string immediately.
/// * Lists the inbox via [`MailboxStore::list`].  Any storage error is persisted
///   as a [`MessagePayload::System`] entry in the transcript and written to the
///   task log; an empty string is returned so the turn still proceeds.
/// * For each message whose ID has **not** yet been marked read, calls
///   [`MailboxStore::mark_read`].  A `true` return means the message was newly
///   consumed; a `false` return means it was already processed in a prior
///   invocation and is skipped.  Mark-read errors are surfaced the same way as
///   list errors and the message is skipped to avoid double-delivery.
/// * Returns a human-readable, XML-structured preamble containing all newly
///   consumed messages.  The caller prepends this to both `user_prompt` and
///   `request_prompt` so the model sees the inbox content in the current turn.
fn poll_and_inject_inbox_messages(
    storage_dir: &Path,
    state: &mut AppState,
    persistence: &mut SessionPersistenceState,
) -> String {
    // Only act when this process was launched as a named agent subprocess.
    let agent_name = match env::var(WONDER_OF_U_AGENT_NAME_ENV) {
        Ok(n) if !n.is_empty() => n,
        _ => return String::new(),
    };

    let store = MailboxStore::new(storage_dir);

    let messages = match store.list(MailboxKind::Agent, &agent_name) {
        Ok(msgs) => msgs,
        Err(e) => {
            let content =
                format!("[inbox-poll] storage error listing inbox for agent \"{agent_name}\": {e}");
            surface_inbox_error(storage_dir, state, persistence, &content);
            return String::new();
        }
    };

    if messages.is_empty() {
        return String::new();
    }

    let mut preamble_parts = Vec::<String>::new();
    let mut injected = 0usize;

    for msg in &messages {
        match store.mark_read(MailboxKind::Agent, &agent_name, msg.id) {
            Ok(true) => {
                // Freshly consumed — include in the model-facing preamble.
                preamble_parts.push(format_inbox_message_xml(msg));
                injected += 1;
            }
            Ok(false) => {
                // Already consumed in a prior invocation — deterministic skip.
            }
            Err(e) => {
                // Surfacing the error is critical; skipping the message avoids
                // a potential double-delivery if the next invocation succeeds.
                let content = format!(
                    "[inbox-poll] error marking message {} read for agent \"{agent_name}\": {e}; \
                     message skipped to prevent double-delivery",
                    msg.id
                );
                surface_inbox_error(storage_dir, state, persistence, &content);
            }
        }
    }

    if injected == 0 {
        return String::new();
    }

    // Write a task-log entry so operators can observe inbox activity without
    // needing to parse the transcript.
    append_inbox_poll_task_log(
        storage_dir,
        &format!(
            "[wonder-of-u inbox-poll] at={} agent={agent_name} injected={injected}\n",
            time::OffsetDateTime::now_utc(),
        ),
    );

    format!(
        "The following {injected} inbox message(s) have been delivered to agent \
         \"{agent_name}\" since the last turn:\n\n{}",
        preamble_parts.join("\n\n")
    )
}

/// Formats a [`MailboxMessage`] as an XML-like block for model injection.
///
/// The `from`, `to`, `subject`, and `body` fields are XML-escaped so special
/// characters in free-text content do not corrupt the surrounding structure.
fn format_inbox_message_xml(msg: &MailboxMessage) -> String {
    format!(
        concat!(
            "<mailbox_message",
            " id=\"{id}\"",
            " from=\"{from}\"",
            " to=\"{to}\"",
            " created_at=\"{created_at}\">\n",
            "  <subject>{subject}</subject>\n",
            "  <body>{body}</body>\n",
            "</mailbox_message>"
        ),
        id = msg.id,
        from = xml_escape(&msg.from),
        to = xml_escape(&msg.to),
        created_at = msg.created_at,
        subject = xml_escape(&msg.subject),
        body = xml_escape(&msg.body),
    )
}

/// Minimal XML character escaping for embedding free text in attribute values
/// and element content.
fn xml_escape(s: &str) -> String {
    // Only the five predefined XML entities need escaping; `&` must come first
    // to avoid double-escaping the introduced `&` characters.
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Surfaces `error` as a [`MessagePayload::System`] transcript entry and writes
/// it to the task log so neither the model nor operators miss it.
fn surface_inbox_error(
    storage_dir: &Path,
    state: &mut AppState,
    persistence: &mut SessionPersistenceState,
    error: &str,
) {
    if let Ok(msg) = append_contextual_message(
        state,
        MessagePayload::System {
            content: error.to_string(),
        },
    ) {
        let _ = persist_messages_and_state(Some(storage_dir), state, persistence, &[msg]);
    }
    append_inbox_poll_task_log(storage_dir, &format!("{error}\n"));
}

/// Appends `entry` to the task log identified by `WONDER_OF_U_TASK_ID`.
///
/// Silently does nothing when the env var is absent, unparseable, or the log
/// write fails — this is a best-effort observability path.
fn append_inbox_poll_task_log(storage_dir: &Path, entry: &str) {
    let Ok(id_str) = env::var("WONDER_OF_U_TASK_ID") else {
        return;
    };
    let Ok(task_id) = id_str.parse::<TaskId>() else {
        return;
    };
    let _ = TaskStore::new(storage_dir).append_log(task_id, entry);
}

#[cfg(test)]
mod tests {
    use wonder_of_u_core::{
        AgentCatalog, AgentLaunchSpec, AgentMessageSpec, FleetRunState, FleetRunStatus,
        PermissionMode, SteeringSource, ToolEffect, ToolResult, ToolUseId,
    };
    use wonder_of_u_storage::FleetStore;
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};
    use wonder_of_u_tools::build_fleet_member_request_with_catalog;

    use super::*;

    // Helper: build a minimal ToolResult with a LaunchAgentTask effect.
    fn launch_effect_result(prompt: &str, catalog: &AgentCatalog) -> ToolResult {
        let input = wonder_of_u_tools::AgentInput {
            prompt: prompt.into(),
            description: None,
            subagent_type: None,
            model: None,
            run_in_background: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
            cwd: None,
            tools: None,
            depends_on: None,
        };
        let request =
            build_fleet_member_request_with_catalog(&input, catalog).expect("build request");
        let use_id = ToolUseId::new();
        ToolResult::success(use_id, "launch_requested").with_effects(vec![
            ToolEffect::LaunchAgentTask(AgentLaunchSpec { request }),
        ])
    }

    /// Helper: ToolResult with a SendAgentMessage effect.
    fn send_message_effect_result(to: &str, content: &str, summary: Option<&str>) -> ToolResult {
        let use_id = ToolUseId::new();
        let spec = AgentMessageSpec {
            to: to.into(),
            summary: summary.map(Into::into),
            content: content.into(),
            message_kind: "text".into(),
        };
        ToolResult::success(use_id, "queued_advisory")
            .with_effects(vec![ToolEffect::SendAgentMessage(spec)])
    }

    #[test]
    fn fork_context_summary_is_byte_capped() {
        use wonder_of_u_core::FORK_SYSTEM_PROMPT_CAP_BYTES;

        let dir = unique_test_dir("prompt-fork-summary-cap");
        let mut state = AppState::new(dir);
        let long_text = "é".repeat(FORK_SYSTEM_PROMPT_CAP_BYTES);
        state
            .push_message(MessageEnvelope::user_text(state.session.id, long_text))
            .expect("message should belong to session");

        let snapshot =
            build_fork_context_snapshot(&state, Some("parent system")).expect("snapshot");
        let summary = snapshot
            .conversation_summary
            .expect("summary should be present");

        assert!(summary.len() <= FORK_SYSTEM_PROMPT_CAP_BYTES);
        assert!(summary.starts_with("User: "));
    }

    /// An empty-effects result passes through process_tool_effects unchanged.
    #[test]
    fn process_effects_passthrough_when_no_effects() {
        let use_id = ToolUseId::new();
        let result = ToolResult::success(use_id, "hello");
        let dir = unique_test_dir("prompt-effects-passthrough");
        let processed = process_tool_effects(result, Some(&dir), &dir, None);
        assert!(processed.success);
        assert_eq!(processed.content, "hello");
        assert!(processed.effects.is_empty());
    }

    /// When direct launch fails (no wonder-of-u binary in test), the effect
    /// handler falls back to writing a pending queue file.
    ///
    /// We test this via the `storage_dir=None` path, which resolves the storage
    /// dir from `WONDER_OF_U_STORAGE_DIR` and always writes to queue (no
    /// direct-launch attempt), making the test deterministic.
    #[test]
    fn process_effects_falls_back_to_pending_queue_when_launch_fails() {
        let dir = unique_test_dir("prompt-effects-fallback");
        let catalog = AgentCatalog::builtin();
        let result = launch_effect_result("build the crate", &catalog);

        // Pin the storage dir via env var so the None-storage-dir fallback path
        // writes to our controlled directory.
        let _guard = EnvVarGuard::set("WONDER_OF_U_STORAGE_DIR", dir.as_os_str());

        // Pass storage_dir=None to trigger the env-var-based fallback path.
        let processed = process_tool_effects(result, None, &dir, None);

        // The result should still be a success (fallback queue succeeded).
        assert!(
            processed.success,
            "fallback queue should produce a success result; content: {}",
            processed.content
        );
        assert!(processed.effects.is_empty(), "effects should be drained");

        // A pending file should now exist.
        let store = FleetStore::new(&dir);
        let pending = store.list_pending_requests().unwrap();
        assert_eq!(
            pending.len(),
            1,
            "one pending request should have been written"
        );
        assert_eq!(pending[0].prompt, "build the crate");

        // Metadata should reflect pending_dispatch.
        let status = processed.metadata["status"].as_str().unwrap_or("");
        assert_eq!(
            status, "pending_dispatch",
            "status should be pending_dispatch after fallback"
        );
    }

    /// With no storage_dir at all, the handler resolves a default dir via env
    /// vars (or marks the result as a failure when none are set).
    #[test]
    fn process_effects_without_storage_dir_does_not_panic() {
        let dir = unique_test_dir("prompt-effects-env-fallback");
        let _guard = EnvVarGuard::set("WONDER_OF_U_STORAGE_DIR", dir.as_os_str());
        let catalog = AgentCatalog::builtin();
        let result = launch_effect_result("no storage", &catalog);
        let processed = process_tool_effects(result, None, &dir, None);
        assert!(
            processed.effects.is_empty(),
            "effects should always be drained"
        );
    }

    // ── SendAgentMessage effect tests ─────────────────────────────────────────

    /// SendAgentMessage with a valid fleet context writes a steering message
    /// with source=Agent and the recipient/summary fields populated.
    #[test]
    fn process_send_agent_message_writes_agent_steering_message() {
        let dir = unique_test_dir("prompt-send-msg-writes");

        // Create a non-terminal fleet run in storage.
        let store = FleetStore::new(&dir);
        let run = FleetRunState::new("test fleet", PermissionMode::Default, None);
        let fleet_id = run.id;
        store.write_run(&run).expect("write run");

        // Set fleet/task env context.
        let _fid = EnvVarGuard::set("WONDER_OF_U_FLEET_ID", fleet_id.to_string().as_str());
        let fake_task_id = wonder_of_u_core::TaskId::new();
        let _tid = EnvVarGuard::set("WONDER_OF_U_TASK_ID", fake_task_id.to_string().as_str());

        let result = send_message_effect_result(
            "reviewer",
            "please review the auth module",
            Some("review auth"),
        );
        let processed = process_tool_effects(result, Some(&dir), &dir, None);

        assert!(
            processed.success,
            "should succeed; content: {}",
            processed.content
        );
        assert!(processed.effects.is_empty(), "effects should be drained");
        assert_eq!(
            processed.metadata["status"].as_str(),
            Some("queued_advisory")
        );
        assert_eq!(processed.metadata["live_delivery"], false);
        assert!(
            processed.metadata["steering_id"].is_string(),
            "steering_id should be set"
        );

        // Verify the persisted message has source=Agent with correct fields.
        let messages = store
            .list_steering_messages(fleet_id)
            .expect("list steering");
        assert_eq!(messages.len(), 1, "one steering message written");
        let msg = &messages[0];
        assert_eq!(msg.source, SteeringSource::Agent);
        assert_eq!(msg.recipient.as_deref(), Some("reviewer"));
        assert_eq!(msg.summary.as_deref(), Some("review auth"));
        assert_eq!(msg.message_kind.as_deref(), Some("text"));
        assert_eq!(msg.sender_task_id, Some(fake_task_id));
        assert_eq!(msg.prompt, "please review the auth module");
    }

    /// Without WONDER_OF_U_FLEET_ID the processor returns a failed result and
    /// writes nothing to storage.
    #[test]
    fn process_send_agent_message_fails_without_fleet_id() {
        let dir = unique_test_dir("prompt-send-msg-no-fleet");
        FleetStore::new(&dir).ensure_layout().expect("layout");

        // Ensure the env var is absent.
        let _fid = EnvVarGuard::remove("WONDER_OF_U_FLEET_ID");

        let result = send_message_effect_result("reviewer", "hi", Some("hi"));
        let processed = process_tool_effects(result, Some(&dir), &dir, None);

        assert!(!processed.success, "should fail without fleet id");
        assert!(
            processed.content.contains("WONDER_OF_U_FLEET_ID"),
            "error should mention env var; got: {}",
            processed.content
        );
        assert!(processed.effects.is_empty(), "effects should be drained");

        // Nothing should be written.
        let msgs = FleetStore::new(&dir)
            .list_steering_messages(wonder_of_u_core::FleetId::new())
            .unwrap_or_default();
        assert!(msgs.is_empty());
    }

    /// Without a storage_dir the processor returns a failed result and writes
    /// nothing.
    #[test]
    fn process_send_agent_message_fails_without_storage_dir() {
        let dir = unique_test_dir("prompt-send-msg-no-storage");
        let fleet_id = wonder_of_u_core::FleetId::new();
        let _fid = EnvVarGuard::set("WONDER_OF_U_FLEET_ID", fleet_id.to_string().as_str());

        let result = send_message_effect_result("reviewer", "hi", Some("hi"));
        // Pass storage_dir=None (and don't set WONDER_OF_U_STORAGE_DIR).
        let _no_storage = EnvVarGuard::remove("WONDER_OF_U_STORAGE_DIR");
        let _no_xdg = EnvVarGuard::remove("XDG_CONFIG_HOME");
        // Keep HOME set so env resolution still fails due to missing fleet.
        // Actually we pass None directly and check error.
        let processed = process_tool_effects(result, None, &dir, None);

        assert!(!processed.success, "should fail without storage dir");
        assert!(processed.effects.is_empty());
    }

    /// A terminal fleet rejects the message and nothing is written.
    #[test]
    fn process_send_agent_message_fails_for_terminal_fleet() {
        let dir = unique_test_dir("prompt-send-msg-terminal-fleet");

        let store = FleetStore::new(&dir);
        let mut run = FleetRunState::new("done fleet", PermissionMode::Default, None);
        run.status = FleetRunStatus::Completed;
        let fleet_id = run.id;
        store.write_run(&run).expect("write run");

        let _fid = EnvVarGuard::set("WONDER_OF_U_FLEET_ID", fleet_id.to_string().as_str());

        let result = send_message_effect_result("reviewer", "more work", Some("more work"));
        let processed = process_tool_effects(result, Some(&dir), &dir, None);

        assert!(!processed.success, "should fail for terminal fleet");
        assert!(
            processed.content.contains("terminal"),
            "error should mention terminal; got: {}",
            processed.content
        );
        assert_eq!(
            processed.metadata["status"].as_str(),
            Some("failed_fleet_terminal")
        );
        assert_eq!(
            processed.metadata["delivery_note"].as_str(),
            Some("fleet is terminal; message not written")
        );
        assert!(processed.effects.is_empty());

        // Nothing written.
        let msgs = store.list_steering_messages(fleet_id).unwrap_or_default();
        assert!(msgs.is_empty());
    }

    /// A broadcast message (to="*") is persisted with recipient="*".
    #[test]
    fn process_send_agent_message_broadcast_recipient() {
        let dir = unique_test_dir("prompt-send-msg-broadcast");

        let store = FleetStore::new(&dir);
        let run = FleetRunState::new("broadcast fleet", PermissionMode::Default, None);
        let fleet_id = run.id;
        store.write_run(&run).expect("write run");

        let _fid = EnvVarGuard::set("WONDER_OF_U_FLEET_ID", fleet_id.to_string().as_str());

        let result = send_message_effect_result("*", "focus on auth", Some("focus auth"));
        let processed = process_tool_effects(result, Some(&dir), &dir, None);

        assert!(processed.success, "broadcast should succeed");
        let msgs = store.list_steering_messages(fleet_id).expect("list");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].recipient.as_deref(), Some("*"));
        assert_eq!(msgs[0].source, SteeringSource::Agent);
    }

    // ── Mailbox delivery tests ─────────────────────────────────────────────────

    /// When the recipient is a locally-registered agent, the message is written
    /// to the agent's mailbox inbox and the result reflects mailbox delivery.
    #[test]
    fn process_send_message_known_agent_writes_mailbox_record() {
        use wonder_of_u_core::MailboxKind;
        use wonder_of_u_core::TaskId;
        use wonder_of_u_storage::MailboxStore;

        let dir = unique_test_dir("prompt-send-msg-mailbox-known");

        // Build an AppState with "reviewer" registered.
        let mut state = AppState::new(dir.clone());
        let target_task_id = TaskId::new();
        state
            .register_agent_name("reviewer".into(), target_task_id)
            .expect("register reviewer");

        // Set up a fleet run so the audit steering path also succeeds.
        let store = FleetStore::new(&dir);
        let run = FleetRunState::new("test fleet", PermissionMode::Default, None);
        let fleet_id = run.id;
        store.write_run(&run).expect("write run");
        let _fid = EnvVarGuard::set("WONDER_OF_U_FLEET_ID", fleet_id.to_string().as_str());
        let fake_sender = TaskId::new();
        let _tid = EnvVarGuard::set("WONDER_OF_U_TASK_ID", fake_sender.to_string().as_str());

        let result = send_message_effect_result(
            "reviewer",
            "please review the auth module",
            Some("review auth"),
        );
        let processed = process_tool_effects(result, Some(&dir), &dir, Some(&mut state));

        assert!(
            processed.success,
            "known agent delivery should succeed; content: {}",
            processed.content
        );
        assert!(processed.effects.is_empty(), "effects should be drained");
        assert_eq!(
            processed.metadata["status"].as_str(),
            Some("delivered_mailbox"),
            "status should be delivered_mailbox"
        );
        assert_eq!(processed.metadata["is_local_agent"], true);
        assert_eq!(processed.metadata["live_delivery"], false);
        assert!(
            processed.metadata["mailbox_id"].is_string(),
            "mailbox_id should be set"
        );
        assert_eq!(
            processed.metadata["recipient_task_id"].as_str(),
            Some(target_task_id.to_string().as_str())
        );

        // Verify the mailbox record was written.
        let mailbox = MailboxStore::new(&dir);
        let msgs = mailbox
            .list(MailboxKind::Agent, "reviewer")
            .expect("list mailbox");
        assert_eq!(msgs.len(), 1, "one mailbox message should be written");
        assert_eq!(msgs[0].to, "reviewer");
        assert_eq!(msgs[0].body, "please review the auth module");
        assert_eq!(msgs[0].subject, "review auth");

        // Steering audit trail should also be present.
        let steering_msgs = store
            .list_steering_messages(fleet_id)
            .expect("list steering");
        assert_eq!(
            steering_msgs.len(),
            1,
            "audit steering message should exist"
        );
        assert_eq!(steering_msgs[0].recipient.as_deref(), Some("reviewer"));
        // Steering id is reported in metadata when audit succeeds.
        assert!(
            processed.metadata["steering_id"].is_string(),
            "steering_id should be populated as audit trail"
        );
    }

    /// When the recipient is not registered locally, SendMessage falls back to
    /// fleet steering with is_local_agent=false in metadata.
    #[test]
    fn process_send_message_unknown_recipient_uses_fleet_steering() {
        let dir = unique_test_dir("prompt-send-msg-mailbox-unknown");

        // Empty AppState — no registered agents.
        let mut state = AppState::new(dir.clone());

        let store = FleetStore::new(&dir);
        let run = FleetRunState::new("fallback fleet", PermissionMode::Default, None);
        let fleet_id = run.id;
        store.write_run(&run).expect("write run");
        let _fid = EnvVarGuard::set("WONDER_OF_U_FLEET_ID", fleet_id.to_string().as_str());

        let result = send_message_effect_result("unknown-peer", "hello", Some("hello"));
        let processed = process_tool_effects(result, Some(&dir), &dir, Some(&mut state));

        // Should still succeed via fleet steering.
        assert!(
            processed.success,
            "unknown recipient should fall back to steering; content: {}",
            processed.content
        );
        assert_eq!(
            processed.metadata["status"].as_str(),
            Some("queued_advisory")
        );
        assert_eq!(
            processed.metadata["is_local_agent"], false,
            "is_local_agent should be false for unregistered recipient"
        );
        // No mailbox should be written — mailboxes dir does not exist yet.
        let mailbox_path = dir.join("mailboxes").join("agents").join("unknown-peer");
        assert!(
            !mailbox_path.exists(),
            "no mailbox should be created for unknown recipient"
        );
    }

    /// Verifies that register_agent_name works correctly on AppState for focused
    /// name-registration contract checks.  (The launch registration itself is
    /// exercised indirectly when direct_launch succeeds; here we test the
    /// AppState API in isolation.)
    #[test]
    fn app_state_register_and_lookup_agent_name() {
        use wonder_of_u_core::TaskId;

        let dir = unique_test_dir("prompt-agent-name-registry");
        let mut state = AppState::new(dir);

        let tid = TaskId::new();
        state
            .register_agent_name("planner".into(), tid)
            .expect("first registration succeeds");

        assert_eq!(
            state.lookup_agent_by_name("planner"),
            Some(tid),
            "registered name should resolve"
        );
        assert_eq!(
            state.lookup_agent_by_name("unknown"),
            None,
            "unregistered name should return None"
        );

        // Deregistering the task clears the name.
        state.deregister_agent_task(tid);
        assert_eq!(
            state.lookup_agent_by_name("planner"),
            None,
            "name should be gone after deregister"
        );
    }

    // ── Inbox polling unit tests ───────────────────────────────────────────────

    /// `format_inbox_message_xml` preserves sender, recipient, subject, and body
    /// in the structured XML output.
    #[test]
    fn format_inbox_message_xml_preserves_metadata() {
        use wonder_of_u_core::mailbox::MailboxMessage;

        let msg = MailboxMessage::new(
            "orchestrator",
            "worker-01",
            "Deploy ready",
            "Please deploy v1.2.",
        );
        let xml = format_inbox_message_xml(&msg);

        assert!(
            xml.contains(&format!("id=\"{}\"", msg.id)),
            "id in attributes"
        );
        assert!(xml.contains("from=\"orchestrator\""), "from in attributes");
        assert!(xml.contains("to=\"worker-01\""), "to in attributes");
        assert!(
            xml.contains("<subject>Deploy ready</subject>"),
            "subject element"
        );
        assert!(
            xml.contains("<body>Please deploy v1.2.</body>"),
            "body element"
        );
    }

    /// `xml_escape` converts the five XML special characters correctly.
    #[test]
    fn xml_escape_handles_special_chars() {
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape("say \"hi\""), "say &quot;hi&quot;");
        assert_eq!(xml_escape("it's"), "it&apos;s");
        // Already-escaped text should not double-escape.
        assert_eq!(xml_escape("plain text"), "plain text");
    }

    /// Special XML characters in message fields are escaped in the XML output.
    #[test]
    fn format_inbox_message_xml_escapes_special_chars() {
        use wonder_of_u_core::mailbox::MailboxMessage;

        let msg = MailboxMessage::new(
            "agent<1>",
            "team&ops",
            "subject with \"quotes\"",
            "body with <tags> & entities",
        );
        let xml = format_inbox_message_xml(&msg);

        assert!(xml.contains("from=\"agent&lt;1&gt;\""), "from is escaped");
        assert!(xml.contains("to=\"team&amp;ops\""), "to is escaped");
        assert!(
            xml.contains("<subject>subject with &quot;quotes&quot;</subject>"),
            "subject escaped"
        );
        assert!(
            xml.contains("<body>body with &lt;tags&gt; &amp; entities</body>"),
            "body escaped"
        );
    }

    /// Inbox polling returns an empty string when `WONDER_OF_U_AGENT_NAME` is
    /// not set — the function must be a no-op outside agent subprocesses.
    #[test]
    fn poll_inbox_returns_empty_without_agent_name_env() {
        let dir = unique_test_dir("inbox-poll-no-agent-name");
        let _rm = EnvVarGuard::remove(WONDER_OF_U_AGENT_NAME_ENV);

        let mut state = AppState::new(dir.clone());
        let mut persistence = SessionPersistenceState {
            transcript_message_count: 0,
            transcript_warning_count: 0,
            persisted: false,
        };
        let result = poll_and_inject_inbox_messages(&dir, &mut state, &mut persistence);
        assert!(
            result.is_empty(),
            "should be empty without agent name env var"
        );
        assert!(state.messages.is_empty(), "no messages should be appended");
    }

    /// Inbox polling returns an empty string (and does not error) when the inbox
    /// has no messages yet.
    #[test]
    fn poll_inbox_returns_empty_when_no_messages() {
        let dir = unique_test_dir("inbox-poll-empty-inbox");
        let _ag = EnvVarGuard::set(WONDER_OF_U_AGENT_NAME_ENV, "worker01");

        let mut state = AppState::new(dir.clone());
        let mut persistence = SessionPersistenceState {
            transcript_message_count: 0,
            transcript_warning_count: 0,
            persisted: false,
        };
        let result = poll_and_inject_inbox_messages(&dir, &mut state, &mut persistence);
        assert!(result.is_empty(), "no preamble for empty inbox");
    }

    /// Inbox polling reads each appended message exactly once: the first poll
    /// returns a non-empty preamble; a second poll returns nothing.
    #[test]
    fn poll_inbox_reads_messages_exactly_once() {
        use wonder_of_u_core::{MailboxKind, mailbox::MailboxMessage};
        use wonder_of_u_storage::MailboxStore;

        let dir = unique_test_dir("inbox-poll-exactly-once");
        let _ag = EnvVarGuard::set(WONDER_OF_U_AGENT_NAME_ENV, "worker01");

        // Append two messages to the agent inbox.
        let store = MailboxStore::new(&dir);
        let m1 = MailboxMessage::new("orchestrator", "worker01", "Task A", "Do task A.");
        let m2 = MailboxMessage::new("orchestrator", "worker01", "Task B", "Do task B.");
        store.append(MailboxKind::Agent, "worker01", &m1).unwrap();
        store.append(MailboxKind::Agent, "worker01", &m2).unwrap();

        let mut state = AppState::new(dir.clone());
        let mut persistence = SessionPersistenceState {
            transcript_message_count: 0,
            transcript_warning_count: 0,
            persisted: false,
        };

        // First poll: both messages should be injected.
        let first = poll_and_inject_inbox_messages(&dir, &mut state, &mut persistence);
        assert!(!first.is_empty(), "first poll should return a preamble");
        assert!(
            first.contains("Task A"),
            "first message subject in preamble"
        );
        assert!(
            first.contains("Task B"),
            "second message subject in preamble"
        );
        assert!(
            first.contains("from=\"orchestrator\""),
            "sender metadata in preamble"
        );
        assert!(
            first.contains("to=\"worker01\""),
            "recipient metadata in preamble"
        );

        // Second poll: both messages are already marked read, preamble must be empty.
        let second = poll_and_inject_inbox_messages(&dir, &mut state, &mut persistence);
        assert!(
            second.is_empty(),
            "second poll must not re-inject already-read messages"
        );

        // Confirm the read index was persisted correctly.
        assert_eq!(
            store.unread_count(MailboxKind::Agent, "worker01").unwrap(),
            0,
            "unread count should be zero after poll"
        );
    }

    /// Sender and recipient metadata is preserved in the model-facing XML output.
    #[test]
    fn poll_inbox_preserves_sender_recipient_metadata() {
        use wonder_of_u_core::{MailboxKind, mailbox::MailboxMessage};
        use wonder_of_u_storage::MailboxStore;

        let dir = unique_test_dir("inbox-poll-metadata");
        let _ag = EnvVarGuard::set(WONDER_OF_U_AGENT_NAME_ENV, "reviewer");

        let store = MailboxStore::new(&dir);
        let msg = MailboxMessage::new(
            "lead-agent",
            "reviewer",
            "Review PR #42",
            "Please review the auth module changes.",
        );
        let msg_id = msg.id;
        store.append(MailboxKind::Agent, "reviewer", &msg).unwrap();

        let mut state = AppState::new(dir.clone());
        let mut persistence = SessionPersistenceState {
            transcript_message_count: 0,
            transcript_warning_count: 0,
            persisted: false,
        };

        let preamble = poll_and_inject_inbox_messages(&dir, &mut state, &mut persistence);

        assert!(
            preamble.contains(&format!("id=\"{msg_id}\"")),
            "message id in output"
        );
        assert!(preamble.contains("from=\"lead-agent\""), "sender preserved");
        assert!(preamble.contains("to=\"reviewer\""), "recipient preserved");
        assert!(
            preamble.contains("<subject>Review PR #42</subject>"),
            "subject preserved"
        );
        assert!(
            preamble.contains("<body>Please review the auth module changes.</body>"),
            "body preserved"
        );
    }

    /// When `MailboxStore::list` fails (e.g. corrupted index), the error is
    /// surfaced as a System transcript entry rather than silently dropped.
    #[test]
    fn poll_inbox_surfaces_storage_error_in_transcript() {
        use std::io::Write;
        use wonder_of_u_storage::MailboxStore;

        let dir = unique_test_dir("inbox-poll-storage-error");
        let _ag = EnvVarGuard::set(WONDER_OF_U_AGENT_NAME_ENV, "broken-agent");

        // Write a corrupt JSONL line so `list` returns an error.
        let store = MailboxStore::new(&dir);
        store
            .ensure_layout(wonder_of_u_core::MailboxKind::Agent, "broken-agent")
            .unwrap();
        let log_path = store
            .paths()
            .mailbox_inbox_log_path(wonder_of_u_core::MailboxKind::Agent, "broken-agent");
        let mut f = std::fs::File::create(&log_path).unwrap();
        writeln!(f, "{{\"schema_version\":9999,\"invalid\":true}}").unwrap();

        let mut state = AppState::new(dir.clone());
        let mut persistence = SessionPersistenceState {
            transcript_message_count: 0,
            transcript_warning_count: 0,
            persisted: false,
        };

        // Polling should return empty (safe degradation) but persist an error.
        let preamble = poll_and_inject_inbox_messages(&dir, &mut state, &mut persistence);
        assert!(
            preamble.is_empty(),
            "should degrade gracefully on storage error"
        );

        // A System message carrying the error text must be in state.messages.
        let error_message = state.messages.iter().find(|m| {
            matches!(&m.payload, MessagePayload::System { content }
                if content.contains("[inbox-poll]") && content.contains("broken-agent"))
        });
        assert!(
            error_message.is_some(),
            "storage error should appear as System message in transcript; \
             messages: {:?}",
            state
                .messages
                .iter()
                .map(|m| &m.payload)
                .collect::<Vec<_>>()
        );
    }
}
