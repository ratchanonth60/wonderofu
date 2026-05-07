use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use clap::Parser;
use wonder_of_u_agent::{
    CompletionRequest, CompletionResponse, ProviderRuntime, ProviderSelection, ProviderToolCall,
    ProviderToolResultMessage, ProviderToolSpec, ToolConversationRound, ToolUseRequest,
    ToolUseResponse, builtin_tool_registry,
};
use wonder_of_u_core::{
    AppState, Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    CoordinatorState, FeatureFlag, MessageEnvelope, MessagePayload, PermissionDecision,
    PermissionMode, PromptSuggestion, QueryState, Result, ToolContext, ToolQuery, ToolResult,
    ToolUseId, WonderError, best_prompt_suggestion,
};
use wonder_of_u_storage::{
    CostStore, SessionCostLedger, SessionMemoryIndexStore, SessionMetadata, SessionSnapshot,
    TranscriptStore,
};
use wonder_of_u_tools::provider_tool_specs;

use super::{
    detect_git_branch,
    hooks::{HookOutcome, POST_TOOL_USE, POST_TOOL_USE_FAILURE, PRE_TOOL_USE, run_hooks},
    parse_command_args,
    parse_session_id,
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
                allowed_tools: None,
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
    let result = execute_prompt_turn(
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
            user_prompt: input.prompt.clone(),
            request_prompt: input.prompt,
        },
    )?;

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
    let registry = builtin_tool_registry()?;
    let tool_context = tool_context(state);
    let provider_tools = provider_tool_specs(&registry, &tool_context, allowed_tools.as_ref())
        .into_iter()
        .map(tool_spec_to_provider_tool)
        .collect::<Vec<_>>();

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
                        &tool_context,
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

fn tool_context(state: &AppState) -> ToolContext {
    ToolContext {
        session_id: state.session.id,
        cwd: state.session.cwd.clone(),
        permission_mode: state.permission_mode,
        additional_working_directories: state.additional_working_directories.clone(),
        permission_rules: Vec::new(),
        features: state.features.clone(),
        bash_session_store: None,
    }
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
            match run_hooks(
                PRE_TOOL_USE,
                &call.provider_call.tool_name,
                &call.provider_call.arguments,
                &context.cwd,
                storage_dir,
            ) {
                HookOutcome::Block { reason } => {
                    ToolResult::failure(call.use_id, format!("hook blocked tool: {reason}"))
                }
                HookOutcome::Allow => {
                    let tool_result = match tool.permission_decision(context, &call.provider_call.arguments) {
                        PermissionDecision::Allow { .. } => {
                            let tool = tool.clone();
                            let context = context.clone();
                            let arguments = call.provider_call.arguments.clone();
                            let use_id = call.use_id;
                            match std::thread::spawn(move || {
                                futures::executor::block_on(tool.execute(context, use_id, arguments))
                            })
                            .join()
                            {
                                Ok(Ok(result)) => result,
                                Ok(Err(error)) => ToolResult::failure(
                                    call.use_id,
                                    format!("tool execution failed: {error}"),
                                ),
                                Err(_) => {
                                    ToolResult::failure(call.use_id, "tool execution thread panicked")
                                }
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
                    run_hooks(
                        post_event,
                        &call.provider_call.tool_name,
                        &call.provider_call.arguments,
                        &context.cwd,
                        storage_dir,
                    );

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
