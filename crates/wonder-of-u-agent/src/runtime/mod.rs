//! Provider runtime: HTTP transport, shared types, and protocol dispatch.
//!
//! This module is the entry point for all provider completions.  Request
//! building, response parsing, and streaming logic live in the protocol-
//! specific sub-modules:
//!
//! - [`openai`]   – OpenAI-compatible (`/chat/completions`) logic.
//! - [`anthropic`] – Anthropic Messages API (`/v1/messages`) logic.
//! - [`copilot`]  – GitHub Copilot token exchange and request builders.
//! - [`bedrock`]  – AWS Bedrock SigV4 signing and `InvokeModel` builders.
//!
//! [`ProviderRuntime::complete`], [`ProviderRuntime::complete_streaming`], and
//! [`ProviderRuntime::complete_with_tool_use`] dispatch to the appropriate
//! sub-module by inspecting [`WireProtocol`] on the resolved provider
//! descriptor.

mod anthropic;
mod azure;
mod bedrock;
mod copilot;
mod gemini;
mod openai;
mod vertex;

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read},
    path::Path,
    sync::Arc,
    time::Duration,
};

use serde_json::Value;
use wonder_of_u_core::{Result, TokenUsage, WonderError};

use crate::{
    CredentialStore, ProviderResolver, ProviderSelection, ResolvedProviderExecution, WireProtocol,
};

// ─── Constants ────────────────────────────────────────────────────────────────

const DEFAULT_ANTHROPIC_API_VERSION: &str = "2023-06-01";
const DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS: u32 = 1024;
const COPILOT_OAUTH_REFRESH_SKEW_SECONDS: i64 = 60;

// ─── Public types ─────────────────────────────────────────────────────────────

/// Represents completion request
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionRequest {
    /// Stores the prompt
    pub prompt: String,
    /// Stores the system prompt
    pub system_prompt: Option<String>,
    /// Stores the max output tokens
    pub max_output_tokens: Option<u32>,
    /// Stores the temperature
    pub temperature: Option<f32>,
    /// Stores the effort level
    pub effort_level: Option<String>,
}

impl CompletionRequest {
    /// Creates a new value
    #[must_use]
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            system_prompt: None,
            max_output_tokens: None,
            temperature: None,
            effort_level: None,
        }
    }
}

/// Represents completion response
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionResponse {
    /// Stores the provider
    pub provider: String,
    /// Stores the model
    pub model: String,
    /// Stores the context window size for the selected model
    pub context_window_size: Option<u64>,
    /// Stores the output text
    pub output_text: String,
    /// Stores the stop reason
    pub stop_reason: Option<String>,
    /// Stores the usage
    pub usage: TokenUsage,
}

/// Represents provider tool spec
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderToolSpec {
    /// Stores the name
    pub name: String,
    /// Stores the description
    pub description: String,
    /// Stores the input schema
    pub input_schema: Value,
}

/// Represents provider tool call
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderToolCall {
    /// Stores the call identifier
    pub call_id: String,
    /// Stores the tool name
    pub tool_name: String,
    /// Stores the arguments
    pub arguments: Value,
}

/// Represents provider tool result message
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderToolResultMessage {
    /// Stores the call identifier
    pub call_id: String,
    /// Stores the content
    pub content: String,
}

/// Represents tool conversation round
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolConversationRound {
    /// Stores the assistant text
    pub assistant_text: Option<String>,
    /// Stores the calls
    pub calls: Vec<ProviderToolCall>,
    /// Stores the results
    pub results: Vec<ProviderToolResultMessage>,
}

/// Represents tool use request
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolUseRequest {
    /// Stores the prompt
    pub prompt: String,
    /// Stores the system prompt
    pub system_prompt: Option<String>,
    /// Stores the max output tokens
    pub max_output_tokens: Option<u32>,
    /// Stores the temperature
    pub temperature: Option<f32>,
    /// Stores the tools
    pub tools: Vec<ProviderToolSpec>,
    /// Stores the rounds
    pub rounds: Vec<ToolConversationRound>,
    /// Stores the effort level
    pub effort_level: Option<String>,
}

/// Represents tool call batch response
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallBatchResponse {
    /// Stores the assistant text
    pub assistant_text: Option<String>,
    /// Stores the calls
    pub calls: Vec<ProviderToolCall>,
    /// Stores the context window size for the selected model
    pub context_window_size: Option<u64>,
    /// Stores the stop reason
    pub stop_reason: Option<String>,
    /// Stores the usage
    pub usage: TokenUsage,
}

/// Enumerates tool use response
#[derive(Clone, Debug, PartialEq)]
pub enum ToolUseResponse {
    /// Represents final
    Final(CompletionResponse),
    /// Represents tool calls
    ToolCalls(ToolCallBatchResponse),
}

// ─── Internal transport types ─────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
struct HttpRequest {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HttpResponse {
    status: u16,
    body: String,
}

struct StreamingHttpResponse {
    reader: Box<dyn Read + Send>,
}

trait HttpTransport: Send + Sync {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse>;
    fn execute_stream(&self, request: &HttpRequest) -> Result<StreamingHttpResponse>;
}

#[derive(Clone)]
struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.new_agent(),
        }
    }
}

impl HttpTransport for UreqTransport {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse> {
        let mut response = send_ureq_request(&self.agent, request)
            .map_err(|e| WonderError::validation(format!("provider request failed: {e}")))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| WonderError::validation(format!("invalid provider response body: {e}")))?;
        if status >= 400 {
            Err(WonderError::validation(format!(
                "provider HTTP request failed with status {status}: {}",
                provider_error_message(&body)
            )))
        } else {
            Ok(HttpResponse { status, body })
        }
    }

    fn execute_stream(&self, request: &HttpRequest) -> Result<StreamingHttpResponse> {
        let response = send_ureq_request(&self.agent, request)
            .map_err(|e| WonderError::validation(format!("provider request failed: {e}")))?;
        let status = response.status().as_u16();
        if status >= 400 {
            let body = response.into_body().read_to_string().unwrap_or_default();
            Err(WonderError::validation(format!(
                "provider HTTP request failed with status {status}: {}",
                provider_error_message(&body)
            )))
        } else {
            Ok(StreamingHttpResponse {
                reader: Box::new(response.into_body().into_reader()),
            })
        }
    }
}

fn send_ureq_request(
    agent: &ureq::Agent,
    request: &HttpRequest,
) -> std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let mut builder = ureq::http::Request::builder()
        .method(request.method.as_str())
        .uri(request.url.as_str());
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let http_req = builder
        .body(request.body.as_str())
        .expect("valid HTTP request");
    agent.run(http_req)
}

// ─── Shared helpers (used across sub-modules) ─────────────────────────────────

fn provider_error_message(body: &str) -> String {
    if let Ok(json) = serde_json::from_str::<Value>(body) {
        if let Some(message) = json
            .pointer("/error/message")
            .and_then(Value::as_str)
            .or_else(|| json.pointer("/error/details").and_then(Value::as_str))
            .or_else(|| json.pointer("/message").and_then(Value::as_str))
        {
            return message.to_string();
        }
    }

    let trimmed = body.trim();
    if trimmed.is_empty() {
        "empty response body".into()
    } else {
        trimmed.to_string()
    }
}

fn join_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

fn context_window_for_model(model: &str) -> u64 {
    let model = model.to_ascii_lowercase();
    if model.contains("gpt-5.5")
        || model.contains("gpt-5.4")
        || model.contains("claude-opus-4-7")
        || model.contains("claude-opus-4.7")
        || model.contains("claude-sonnet-4-6")
        || model.contains("claude-sonnet-4.6")
    {
        1_050_000
    } else if model.contains("gpt-5")
        || model.contains("claude-haiku-4-5")
        || model.contains("claude-haiku-4.5")
    {
        400_000
    } else if model.contains("gpt-4.1") {
        1_047_576
    } else if model.contains("claude-3-5")
        || model.contains("claude-3-7")
        || model.contains("claude-sonnet")
        || model.contains("claude-3-opus")
    {
        200_000
    } else if model.contains("gpt-4o") {
        128_000
    } else {
        200_000
    }
}

/// Returns `true` when the model name indicates an Anthropic Claude model.
///
/// Used by the Copilot sub-module to choose between the OpenAI-compat and
/// Anthropic Messages sub-paths.
fn is_anthropic_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("claude")
}

fn is_high_effort(level: Option<&str>) -> bool {
    matches!(level, Some("high") | Some("max"))
}

fn consume_sse<R, F>(mut reader: BufReader<R>, mut on_event: F) -> Result<()>
where
    R: Read,
    F: FnMut(Option<&str>, &str) -> Result<bool>,
{
    let mut line = String::new();
    let mut event_name = None::<String>;
    let mut data_lines = Vec::<String>::new();

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line).map_err(|error| {
            WonderError::validation(format!(
                "failed to read streamed provider response: {error}"
            ))
        })?;
        if bytes == 0 {
            flush_sse_event(&mut event_name, &mut data_lines, &mut on_event)?;
            return Ok(());
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            if !flush_sse_event(&mut event_name, &mut data_lines, &mut on_event)? {
                return Ok(());
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("event:") {
            event_name = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }
}

fn flush_sse_event<F>(
    event_name: &mut Option<String>,
    data_lines: &mut Vec<String>,
    on_event: &mut F,
) -> Result<bool>
where
    F: FnMut(Option<&str>, &str) -> Result<bool>,
{
    if data_lines.is_empty() {
        event_name.take();
        return Ok(true);
    }

    let data = data_lines.join("\n");
    data_lines.clear();
    let keep_going = on_event(event_name.as_deref(), &data)?;
    event_name.take();
    Ok(keep_going)
}

// ─── ProviderRuntime ──────────────────────────────────────────────────────────

fn wire_protocol_supports_tool_use(protocol: WireProtocol) -> bool {
    matches!(
        protocol,
        WireProtocol::OpenAiCompat
            | WireProtocol::AnthropicCompat
            | WireProtocol::Copilot
            | WireProtocol::BedrockAnthropic
            | WireProtocol::AzureOpenAi
            | WireProtocol::GeminiNative
            | WireProtocol::VertexGemini
    )
}

/// Represents provider runtime
pub struct ProviderRuntime {
    resolver: ProviderResolver,
    transport: Arc<dyn HttpTransport>,
}

impl Default for ProviderRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRuntime {
    /// Creates a new value
    #[must_use]
    pub fn new() -> Self {
        Self {
            resolver: ProviderResolver::builtin(),
            transport: Arc::new(UreqTransport::default()),
        }
    }

    /// Resolves execution
    pub fn resolve_execution(
        &self,
        storage_dir: Option<&Path>,
        selection: ProviderSelection,
    ) -> Result<ResolvedProviderExecution> {
        let resolved = self
            .resolver
            .load_execution(storage_dir, selection.clone())?;
        self.refresh_copilot_oauth_if_needed(storage_dir, &selection, resolved)
    }

    /// Handles complete with storage
    pub fn complete_with_storage(
        &self,
        storage_dir: Option<&Path>,
        selection: ProviderSelection,
        request: &CompletionRequest,
    ) -> Result<(ResolvedProviderExecution, CompletionResponse)> {
        let resolved = self.resolve_execution(storage_dir, selection)?;
        let response = self.complete(&resolved, request)?;
        Ok((resolved, response))
    }

    /// Handles complete.
    ///
    /// Dispatches to the appropriate protocol implementation based on
    /// `resolved.provider().wire_protocol`.
    pub fn complete(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse> {
        if request.prompt.trim().is_empty() {
            return Err(WonderError::validation("prompt cannot be empty"));
        }

        match resolved.provider().wire_protocol {
            WireProtocol::OpenAiCompat => {
                let http_request = openai::build_openai_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                openai::parse_openai_response(resolved, &http_response.body)
            }
            WireProtocol::AnthropicCompat => {
                let http_request = anthropic::build_anthropic_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                anthropic::parse_anthropic_response(resolved, &http_response.body)
            }
            WireProtocol::Copilot => self.complete_copilot(resolved, request),
            WireProtocol::BedrockAnthropic => {
                let http_request = bedrock::build_bedrock_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                anthropic::parse_anthropic_response(resolved, &http_response.body)
            }
            WireProtocol::GeminiNative => {
                let http_request = gemini::build_gemini_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                gemini::parse_gemini_response(resolved, &http_response.body)
            }
            WireProtocol::VertexGemini => {
                let http_request = vertex::build_vertex_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                vertex::parse_vertex_response(resolved, &http_response.body)
            }
            WireProtocol::AzureOpenAi => {
                let http_request = azure::build_azure_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                azure::parse_azure_response(resolved, &http_response.body)
            }
            proto => Err(WonderError::validation(format!(
                "wire protocol `{proto:?}` is not supported yet"
            ))),
        }
    }

    /// Returns whether the provider identified by `provider_id` supports
    /// streaming.
    ///
    /// Looks up the provider's [`WireProtocol`] from the registry; unknown
    /// provider IDs return `false`.
    #[must_use]
    pub fn supports_streaming(&self, provider_id: &str) -> bool {
        self.resolver
            .registry()
            .get(provider_id)
            .map(|desc| {
                matches!(
                    desc.wire_protocol,
                    WireProtocol::OpenAiCompat
                        | WireProtocol::AnthropicCompat
                        | WireProtocol::Copilot
                        | WireProtocol::BedrockAnthropic
                        | WireProtocol::AzureOpenAi
                        | WireProtocol::GeminiNative
                        | WireProtocol::VertexGemini
                )
            })
            .unwrap_or(false)
    }

    /// Returns whether the provider identified by `provider_id` supports tool
    /// use in the runtime.
    ///
    /// Looks up the provider's [`WireProtocol`] from the registry; unknown
    /// provider IDs return `false`.
    #[must_use]
    pub fn supports_tool_use(&self, provider_id: &str) -> bool {
        self.resolver
            .registry()
            .get(provider_id)
            .is_some_and(|desc| wire_protocol_supports_tool_use(desc.wire_protocol))
    }

    /// Returns whether tool use is supported for the given resolved provider.
    ///
    /// Dispatches based on [`WireProtocol`]; all currently implemented protocols
    /// support tool use.
    #[must_use]
    pub fn supports_tool_use_for(&self, resolved: &ResolvedProviderExecution) -> bool {
        wire_protocol_supports_tool_use(resolved.provider().wire_protocol)
    }

    /// Handles complete streaming.
    ///
    /// Dispatches to the appropriate protocol implementation based on
    /// `resolved.provider().wire_protocol`.
    pub fn complete_streaming<F>(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &CompletionRequest,
        mut on_text_delta: F,
    ) -> Result<CompletionResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        if request.prompt.trim().is_empty() {
            return Err(WonderError::validation("prompt cannot be empty"));
        }

        match resolved.provider().wire_protocol {
            WireProtocol::OpenAiCompat => {
                let http_request = openai::build_openai_stream_request(resolved, request)?;
                let http_response = self.transport.execute_stream(&http_request)?;
                openai::parse_openai_stream_response(resolved, http_response, &mut on_text_delta)
            }
            WireProtocol::AnthropicCompat => {
                let http_request = anthropic::build_anthropic_stream_request(resolved, request)?;
                let http_response = self.transport.execute_stream(&http_request)?;
                anthropic::parse_anthropic_stream_response(
                    resolved,
                    http_response,
                    &mut on_text_delta,
                )
            }
            WireProtocol::Copilot => {
                self.complete_copilot_streaming(resolved, request, &mut on_text_delta)
            }
            WireProtocol::BedrockAnthropic => {
                let http_request = bedrock::build_bedrock_stream_request(resolved, request)?;
                let http_response = self.transport.execute_stream(&http_request)?;
                anthropic::parse_anthropic_stream_response(
                    resolved,
                    http_response,
                    &mut on_text_delta,
                )
            }
            WireProtocol::AzureOpenAi => {
                let http_request = azure::build_azure_stream_request(resolved, request)?;
                let http_response = self.transport.execute_stream(&http_request)?;
                azure::parse_azure_stream_response(resolved, http_response, &mut on_text_delta)
            }
            WireProtocol::GeminiNative => {
                let http_request = gemini::build_gemini_stream_request(resolved, request)?;
                let http_response = self.transport.execute_stream(&http_request)?;
                gemini::parse_gemini_stream_response(resolved, http_response, &mut on_text_delta)
            }
            WireProtocol::VertexGemini => {
                let http_request = vertex::build_vertex_stream_request(resolved, request)?;
                let http_response = self.transport.execute_stream(&http_request)?;
                vertex::parse_vertex_stream_response(resolved, http_response, &mut on_text_delta)
            }
            proto => Err(WonderError::validation(format!(
                "wire protocol `{proto:?}` streaming is not supported"
            ))),
        }
    }

    /// Handles complete with tool use.
    ///
    /// Dispatches to the appropriate protocol implementation based on
    /// `resolved.provider().wire_protocol`.
    pub fn complete_with_tool_use(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &ToolUseRequest,
    ) -> Result<ToolUseResponse> {
        if request.prompt.trim().is_empty() {
            return Err(WonderError::validation("prompt cannot be empty"));
        }

        match resolved.provider().wire_protocol {
            WireProtocol::OpenAiCompat => {
                let http_request = openai::build_openai_tool_use_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                openai::parse_openai_tool_use_response(resolved, &http_response.body)
            }
            WireProtocol::AnthropicCompat => {
                let http_request = anthropic::build_anthropic_tool_use_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                anthropic::parse_anthropic_tool_use_response(resolved, &http_response.body)
            }
            WireProtocol::Copilot => self.complete_copilot_with_tool_use(resolved, request),
            WireProtocol::BedrockAnthropic => {
                let http_request = bedrock::build_bedrock_tool_use_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                anthropic::parse_anthropic_tool_use_response(resolved, &http_response.body)
            }
            WireProtocol::AzureOpenAi => {
                let http_request = azure::build_azure_tool_use_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                azure::parse_azure_tool_use_response(resolved, &http_response.body)
            }
            WireProtocol::GeminiNative => {
                let http_request = gemini::build_gemini_tool_use_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                gemini::parse_gemini_tool_use_response(resolved, &http_response.body)
            }
            WireProtocol::VertexGemini => {
                let http_request = vertex::build_vertex_tool_use_request(resolved, request)?;
                let http_response = self.transport.execute(&http_request)?;
                vertex::parse_vertex_tool_use_response(resolved, &http_response.body)
            }
            proto => Err(WonderError::validation(format!(
                "wire protocol `{proto:?}` tool-use is not supported"
            ))),
        }
    }

    // ─── Copilot dispatch helpers ─────────────────────────────────────────────

    fn complete_copilot(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse> {
        let session = self.exchange_copilot_session(resolved)?;
        if copilot::copilot_model_uses_anthropic_path(resolved.model()) {
            let http_request = copilot::build_copilot_anthropic_request(
                resolved,
                request,
                false,
                &session.api_base,
                &session.bearer_token,
            )?;
            let http_response = self.transport.execute(&http_request)?;
            anthropic::parse_anthropic_response(resolved, &http_response.body)
        } else {
            let http_request = copilot::build_copilot_openai_request(
                resolved,
                request,
                false,
                &session.api_base,
                &session.bearer_token,
            )?;
            let http_response = self.transport.execute(&http_request)?;
            openai::parse_openai_response(resolved, &http_response.body)
        }
    }

    fn complete_copilot_streaming<F>(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &CompletionRequest,
        on_text_delta: &mut F,
    ) -> Result<CompletionResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        let session = self.exchange_copilot_session(resolved)?;
        if copilot::copilot_model_uses_anthropic_path(resolved.model()) {
            let http_request = copilot::build_copilot_anthropic_request(
                resolved,
                request,
                true,
                &session.api_base,
                &session.bearer_token,
            )?;
            let http_response = self.transport.execute_stream(&http_request)?;
            anthropic::parse_anthropic_stream_response(resolved, http_response, on_text_delta)
        } else {
            let http_request = copilot::build_copilot_openai_request(
                resolved,
                request,
                true,
                &session.api_base,
                &session.bearer_token,
            )?;
            let http_response = self.transport.execute_stream(&http_request)?;
            openai::parse_openai_stream_response(resolved, http_response, on_text_delta)
        }
    }

    fn complete_copilot_with_tool_use(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &ToolUseRequest,
    ) -> Result<ToolUseResponse> {
        let session = self.exchange_copilot_session(resolved)?;
        let http_request = if copilot::copilot_model_uses_anthropic_path(resolved.model()) {
            copilot::build_copilot_anthropic_tool_use_request(
                resolved,
                request,
                &session.api_base,
                &session.bearer_token,
            )?
        } else {
            copilot::build_copilot_openai_tool_use_request(
                resolved,
                request,
                &session.api_base,
                &session.bearer_token,
            )?
        };
        let http_response = self.transport.execute(&http_request)?;
        if copilot::copilot_model_uses_anthropic_path(resolved.model()) {
            anthropic::parse_anthropic_tool_use_response(resolved, &http_response.body)
        } else {
            openai::parse_openai_tool_use_response(resolved, &http_response.body)
        }
    }

    fn exchange_copilot_session(
        &self,
        resolved: &ResolvedProviderExecution,
    ) -> Result<copilot::CopilotSession> {
        let http_request = copilot::build_copilot_token_exchange_request(resolved)?;
        let http_response = self.transport.execute(&http_request)?;
        copilot::parse_copilot_token_exchange_response(resolved, &http_response.body)
    }

    fn refresh_copilot_oauth_if_needed(
        &self,
        storage_dir: Option<&Path>,
        selection: &ProviderSelection,
        resolved: ResolvedProviderExecution,
    ) -> Result<ResolvedProviderExecution> {
        // Only Copilot uses OAuth token refresh; skip all other protocols.
        if resolved.provider().wire_protocol != WireProtocol::Copilot
            || !copilot::copilot_oauth_should_refresh(&resolved)
        {
            return Ok(resolved);
        }
        let storage_dir = storage_dir.ok_or_else(|| {
            WonderError::validation(
                "copilot oauth token refresh requires --storage-dir so refreshed credentials can be persisted",
            )
        })?;
        let refresh_token = resolved.oauth_refresh_token().ok_or_else(|| {
            WonderError::validation(
                "stored copilot oauth access token expired and no refresh token is available; run `wonder-of-u login --provider copilot` again",
            )
        })?;
        let http_request = copilot::build_copilot_oauth_refresh_request(refresh_token);
        let http_response = self.transport.execute(&http_request)?;
        let token = copilot::parse_copilot_oauth_refresh_response(&http_response.body)?;
        CredentialStore::new(storage_dir).set_oauth_token(
            "copilot",
            token.access_token,
            token.refresh_token,
            token.expires_at,
        )?;
        self.resolver
            .load_execution(Some(storage_dir), selection.clone())
    }
}

#[cfg(test)]
impl ProviderRuntime {
    fn with_transport(transport: Arc<dyn HttpTransport>) -> Self {
        Self {
            resolver: ProviderResolver::builtin(),
            transport,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, io::Cursor, sync::Mutex};

    use time::OffsetDateTime;
    use wonder_of_u_test_support::unique_test_dir;

    use crate::{
        AgentSettings, AuthMaterial, CredentialStore, SettingsStore, StoredCredentials,
        auth::AwsCredentials,
    };

    use super::*;
    // Explicitly import test-visible helpers from sub-modules.
    use bedrock::{sha256_hex, sign_request_headers, sigv4_datetime};

    #[derive(Default)]
    struct RecordingTransport {
        requests: Mutex<Vec<HttpRequest>>,
        responses: Mutex<Vec<HttpResponse>>,
        stream_bodies: Mutex<Vec<String>>,
        force_error: Mutex<Option<String>>,
    }

    impl RecordingTransport {
        fn with_json_body(body: serde_json::Value) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(vec![HttpResponse {
                    status: 200,
                    body: serde_json::to_string(&body).expect("serialize response"),
                }]),
                stream_bodies: Mutex::new(Vec::new()),
                force_error: Mutex::new(None),
            })
        }

        fn take_request(&self) -> HttpRequest {
            self.requests
                .lock()
                .expect("lock requests")
                .pop()
                .expect("recorded request")
        }

        fn take_requests(&self) -> Vec<HttpRequest> {
            std::mem::take(&mut *self.requests.lock().expect("lock requests"))
        }

        fn with_json_responses(bodies: Vec<serde_json::Value>) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(
                    bodies
                        .into_iter()
                        .map(|body| HttpResponse {
                            status: 200,
                            body: serde_json::to_string(&body).expect("serialize response"),
                        })
                        .collect(),
                ),
                stream_bodies: Mutex::new(Vec::new()),
                force_error: Mutex::new(None),
            })
        }

        fn with_stream_body(body: &str) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(Vec::new()),
                stream_bodies: Mutex::new(vec![body.into()]),
                force_error: Mutex::new(None),
            })
        }

        fn with_json_and_stream_bodies(
            json_bodies: Vec<serde_json::Value>,
            stream_bodies: Vec<&str>,
        ) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(
                    json_bodies
                        .into_iter()
                        .map(|body| HttpResponse {
                            status: 200,
                            body: serde_json::to_string(&body).expect("serialize response"),
                        })
                        .collect(),
                ),
                stream_bodies: Mutex::new(
                    stream_bodies.into_iter().map(ToString::to_string).collect(),
                ),
                force_error: Mutex::new(None),
            })
        }

        fn with_http_error(status: u16, body: serde_json::Value) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(vec![HttpResponse {
                    status,
                    body: serde_json::to_string(&body).expect("serialize error body"),
                }]),
                ..Self::default()
            })
        }

        fn with_raw_body(status: u16, body: impl Into<String>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(vec![HttpResponse {
                    status,
                    body: body.into(),
                }]),
                ..Self::default()
            })
        }

        fn with_network_error(message: impl Into<String>) -> Arc<Self> {
            Arc::new(Self {
                force_error: Mutex::new(Some(message.into())),
                ..Self::default()
            })
        }
    }

    impl HttpTransport for RecordingTransport {
        fn execute(&self, request: &HttpRequest) -> Result<HttpResponse> {
            self.requests
                .lock()
                .expect("lock requests")
                .push(request.clone());
            if let Some(msg) = self.force_error.lock().expect("lock force_error").take() {
                return Err(WonderError::validation(format!(
                    "provider request failed: {msg}"
                )));
            }
            let mut responses = self.responses.lock().expect("lock response");
            if responses.is_empty() {
                return Err(WonderError::internal("missing recorded response"));
            }
            let response = responses.remove(0);
            if response.status >= 400 {
                return Err(WonderError::validation(format!(
                    "provider HTTP request failed with status {}: {}",
                    response.status,
                    provider_error_message(&response.body),
                )));
            }
            Ok(response)
        }

        fn execute_stream(&self, request: &HttpRequest) -> Result<StreamingHttpResponse> {
            self.requests
                .lock()
                .expect("lock requests")
                .push(request.clone());
            let mut stream_bodies = self.stream_bodies.lock().expect("lock stream body");
            if stream_bodies.is_empty() {
                return Err(WonderError::internal("missing recorded stream body"));
            }
            let body = stream_bodies.remove(0);
            Ok(StreamingHttpResponse {
                reader: Box::new(Cursor::new(body.into_bytes())),
            })
        }
    }

    fn resolved_provider(provider: &str, model: Option<&str>) -> ResolvedProviderExecution {
        let settings = AgentSettings {
            selected_provider: Some(provider.into()),
            selected_model: model.map(ToString::to_string),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: BTreeMap::from([(
                provider.into(),
                AuthMaterial::ApiKey {
                    key: "secret-key".into(),
                },
            )]),
        };

        ProviderResolver::builtin()
            .resolve_execution_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
                &ProviderSelection::default(),
            )
            .expect("resolve provider")
    }

    fn resolved_oauth_provider(provider: &str, model: Option<&str>) -> ResolvedProviderExecution {
        let settings = AgentSettings {
            selected_provider: Some(provider.into()),
            selected_model: model.map(ToString::to_string),
            ..AgentSettings::default()
        };
        let credentials = StoredCredentials {
            providers: BTreeMap::from([(
                provider.into(),
                AuthMaterial::OAuth {
                    access_token: Some("oauth-token".into()),
                    refresh_token: None,
                    expires_at: None,
                },
            )]),
        };

        ProviderResolver::builtin()
            .resolve_execution_with_env(
                &settings,
                &credentials,
                std::iter::empty::<(&str, String)>(),
                &ProviderSelection::default(),
            )
            .expect("resolve provider")
    }

    #[test]
    fn runtime_refreshes_expired_copilot_oauth_credentials_before_execution() {
        let dir = unique_test_dir("agent-copilot-oauth-refresh");
        SettingsStore::new(&dir)
            .write(&AgentSettings {
                selected_provider: Some("copilot".into()),
                selected_model: Some("gpt-4.1".into()),
                ..AgentSettings::default()
            })
            .expect("write settings");
        CredentialStore::new(&dir)
            .set_oauth_token(
                "copilot",
                "expired-oauth-token",
                Some("refresh-token-1".into()),
                Some(OffsetDateTime::now_utc() - time::Duration::minutes(5)),
            )
            .expect("write credentials");
        let transport = RecordingTransport::with_json_responses(vec![serde_json::json!({
            "access_token": "fresh-oauth-token",
            "refresh_token": "refresh-token-2",
            "expires_in": 3600
        })]);
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);

        let resolved = runtime
            .resolve_execution(
                Some(dir.as_path()),
                ProviderSelection::new(Some("copilot".into()), Some("gpt-4.1".into())),
            )
            .expect("resolve execution with refresh");
        let requests = transport.take_requests();
        let stored = CredentialStore::new(&dir)
            .read()
            .expect("read refreshed credentials");

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(
            requests[0].url,
            "https://github.com/login/oauth/access_token"
        );
        assert!(requests[0].body.contains("grant_type=refresh_token"));
        assert!(
            requests[0]
                .body
                .contains("refresh_token=refresh%2Dtoken%2D1")
        );
        assert_eq!(
            resolved.oauth_access_token().expect("oauth access token"),
            "fresh-oauth-token"
        );
        assert!(matches!(
            stored.providers.get("copilot"),
            Some(AuthMaterial::OAuth {
                access_token: Some(token),
                refresh_token: Some(refresh_token),
                expires_at: Some(_),
            }) if token == "fresh-oauth-token" && refresh_token == "refresh-token-2"
        ));
    }

    #[test]
    fn runtime_rejects_expired_copilot_oauth_credentials_without_refresh_token() {
        let dir = unique_test_dir("agent-copilot-oauth-expired");
        SettingsStore::new(&dir)
            .write(&AgentSettings {
                selected_provider: Some("copilot".into()),
                selected_model: Some("gpt-4.1".into()),
                ..AgentSettings::default()
            })
            .expect("write settings");
        CredentialStore::new(&dir)
            .set_oauth_token(
                "copilot",
                "expired-oauth-token",
                None,
                Some(OffsetDateTime::now_utc() - time::Duration::minutes(5)),
            )
            .expect("write credentials");
        let runtime = ProviderRuntime::with_transport(Arc::new(RecordingTransport::default()));

        let error = runtime
            .resolve_execution(
                Some(dir.as_path()),
                ProviderSelection::new(Some("copilot".into()), Some("gpt-4.1".into())),
            )
            .expect_err("expired oauth token should fail without refresh token");

        assert!(
            error
                .to_string()
                .contains("stored copilot oauth access token expired")
        );
    }

    #[test]
    fn openai_runtime_builds_chat_completions_request() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"content": "Hello back"}
            }],
            "usage": {
                "prompt_tokens": 11,
                "completion_tokens": 7,
                "prompt_tokens_details": {"cached_tokens": 2}
            }
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("openai", Some("gpt-4.1"));
        let request = CompletionRequest {
            prompt: "Say hi".into(),
            system_prompt: Some("Be concise".into()),
            max_output_tokens: Some(64),
            temperature: Some(0.2),
            effort_level: None,
        };

        let response = runtime
            .complete(&resolved, &request)
            .expect("openai response");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(
            recorded.headers.get("authorization").map(String::as_str),
            Some("Bearer secret-key")
        );
        assert_eq!(
            body.pointer("/model").and_then(serde_json::Value::as_str),
            Some("gpt-4.1")
        );
        assert_eq!(
            body.pointer("/messages/0/role")
                .and_then(serde_json::Value::as_str),
            Some("system")
        );
        assert_eq!(
            body.pointer("/messages/0/content")
                .and_then(serde_json::Value::as_str),
            Some("Be concise")
        );
        assert_eq!(
            body.pointer("/messages/1/content")
                .and_then(serde_json::Value::as_str),
            Some("Say hi")
        );
        assert_eq!(
            body.pointer("/max_completion_tokens")
                .and_then(serde_json::Value::as_u64),
            Some(64)
        );
        let temperature = body
            .pointer("/temperature")
            .and_then(serde_json::Value::as_f64)
            .expect("temperature");
        assert!((temperature - 0.2).abs() < 1e-6);
        assert_eq!(response.output_text, "Hello back");
        assert_eq!(response.stop_reason.as_deref(), Some("stop"));
        assert_eq!(response.context_window_size, Some(1_047_576));
        assert_eq!(response.usage.input_tokens, 11);
        assert_eq!(response.usage.output_tokens, 7);
        assert_eq!(response.usage.cache_read_tokens, 2);
    }

    #[test]
    fn openai_tool_use_runtime_builds_tools_request_with_prior_rounds() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"content": "Done"}
            }],
            "usage": {
                "prompt_tokens": 21,
                "completion_tokens": 4
            }
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("openai", Some("gpt-4.1"));

        let response = runtime
            .complete_with_tool_use(
                &resolved,
                &ToolUseRequest {
                    prompt: "Summarize the file".into(),
                    system_prompt: Some("Use tools when needed".into()),
                    max_output_tokens: Some(128),
                    temperature: Some(0.1),
                    effort_level: None,
                    tools: vec![ProviderToolSpec {
                        name: "file_read".into(),
                        description: "Read a UTF-8 file".into(),
                        input_schema: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "path": {"type": "string"}
                            },
                            "required": ["path"],
                            "additionalProperties": false
                        }),
                    }],
                    rounds: vec![ToolConversationRound {
                        assistant_text: Some("Let me inspect that file.".into()),
                        calls: vec![ProviderToolCall {
                            call_id: "call_123".into(),
                            tool_name: "file_read".into(),
                            arguments: serde_json::json!({"path": "src/main.rs"}),
                        }],
                        results: vec![ProviderToolResultMessage {
                            call_id: "call_123".into(),
                            content: "1. fn main() {}".into(),
                        }],
                    }],
                },
            )
            .expect("tool-use response");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert!(matches!(response, ToolUseResponse::Final(_)));
        assert_eq!(
            body.pointer("/tool_choice")
                .and_then(serde_json::Value::as_str),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/parallel_tool_calls")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            body.pointer("/tools/0/function/name")
                .and_then(serde_json::Value::as_str),
            Some("file_read")
        );
        assert_eq!(
            body.pointer("/messages/0/role")
                .and_then(serde_json::Value::as_str),
            Some("system")
        );
        assert_eq!(
            body.pointer("/messages/1/content")
                .and_then(serde_json::Value::as_str),
            Some("Summarize the file")
        );
        assert_eq!(
            body.pointer("/messages/2/tool_calls/0/id")
                .and_then(serde_json::Value::as_str),
            Some("call_123")
        );
        assert_eq!(
            body.pointer("/messages/2/tool_calls/0/function/arguments")
                .and_then(serde_json::Value::as_str),
            Some("{\"path\":\"src/main.rs\"}")
        );
        assert_eq!(
            body.pointer("/messages/3/role")
                .and_then(serde_json::Value::as_str),
            Some("tool")
        );
        assert_eq!(
            body.pointer("/messages/3/tool_call_id")
                .and_then(serde_json::Value::as_str),
            Some("call_123")
        );
    }

    #[test]
    fn openai_tool_use_runtime_parses_structured_tool_calls() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": "Checking that now.",
                    "tool_calls": [{
                        "id": "call_456",
                        "type": "function",
                        "function": {
                            "name": "glob",
                            "arguments": "{\"pattern\":\"src/**/*.rs\"}"
                        }
                    }]
                }
            }],
            "usage": {
                "prompt_tokens": 18,
                "completion_tokens": 3
            }
        }));
        let runtime = ProviderRuntime::with_transport(transport as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("openai", Some("gpt-4.1"));

        let response = runtime
            .complete_with_tool_use(
                &resolved,
                &ToolUseRequest {
                    prompt: "Find the Rust source files".into(),
                    ..ToolUseRequest::default()
                },
            )
            .expect("tool-call response");

        match response {
            ToolUseResponse::ToolCalls(batch) => {
                assert_eq!(batch.assistant_text.as_deref(), Some("Checking that now."));
                assert_eq!(batch.calls.len(), 1);
                assert_eq!(batch.calls[0].call_id, "call_456");
                assert_eq!(batch.calls[0].tool_name, "glob");
                assert_eq!(
                    batch.calls[0].arguments,
                    serde_json::json!({"pattern": "src/**/*.rs"})
                );
                assert_eq!(batch.context_window_size, Some(1_047_576));
                assert_eq!(batch.stop_reason.as_deref(), Some("tool_calls"));
                assert_eq!(batch.usage.input_tokens, 18);
                assert_eq!(batch.usage.output_tokens, 3);
            }
            other => panic!("expected tool call response, got {other:?}"),
        }
    }

    #[test]
    fn anthropic_runtime_builds_messages_request() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "content": [
                {"type": "text", "text": "Part one. "},
                {"type": "text", "text": "Part two."}
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 13,
                "output_tokens": 5,
                "cache_creation_input_tokens": 4,
                "cache_read_input_tokens": 1
            }
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("anthropic", Some("claude-opus-4-7"));
        let request = CompletionRequest::new("Explain the slice");

        let response = runtime
            .complete(&resolved, &request)
            .expect("anthropic response");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(
            recorded.headers.get("x-api-key").map(String::as_str),
            Some("secret-key")
        );
        assert_eq!(
            recorded
                .headers
                .get("anthropic-version")
                .map(String::as_str),
            Some(DEFAULT_ANTHROPIC_API_VERSION)
        );
        assert_eq!(
            body.pointer("/model").and_then(serde_json::Value::as_str),
            Some("claude-opus-4-7")
        );
        assert_eq!(
            body.pointer("/messages/0/content")
                .and_then(serde_json::Value::as_str),
            Some("Explain the slice")
        );
        assert_eq!(
            body.pointer("/max_tokens")
                .and_then(serde_json::Value::as_u64),
            Some(u64::from(DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS))
        );
        assert_eq!(response.output_text, "Part one. Part two.");
        assert_eq!(response.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(response.context_window_size, Some(1_050_000));
        assert_eq!(response.usage.input_tokens, 13);
        assert_eq!(response.usage.output_tokens, 5);
        assert_eq!(response.usage.cache_creation_tokens, 4);
        assert_eq!(response.usage.cache_read_tokens, 1);
    }

    #[test]
    fn openai_streaming_runtime_builds_stream_request_and_collects_deltas() {
        let transport = RecordingTransport::with_stream_body(concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" back\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":2}}}\n\n",
            "data: [DONE]\n\n",
        ));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("openai", Some("gpt-4.1"));
        let mut streamed = String::new();

        let response = runtime
            .complete_streaming(&resolved, &CompletionRequest::new("Say hi"), |delta| {
                streamed.push_str(delta);
                Ok(())
            })
            .expect("openai stream");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(
            body.pointer("/stream").and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            body.pointer("/stream_options/include_usage")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(streamed, "Hello back");
        assert_eq!(response.output_text, "Hello back");
        assert_eq!(response.stop_reason.as_deref(), Some("stop"));
        assert_eq!(response.context_window_size, Some(1_047_576));
        assert_eq!(response.usage.input_tokens, 11);
        assert_eq!(response.usage.output_tokens, 7);
        assert_eq!(response.usage.cache_read_tokens, 2);
    }

    #[test]
    fn anthropic_streaming_runtime_builds_stream_request_and_collects_deltas() {
        let transport = RecordingTransport::with_stream_body(concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":13}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Part one. \"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Part two.\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5,\"cache_creation_input_tokens\":4,\"cache_read_input_tokens\":1}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        ));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("anthropic", Some("claude-opus-4-7"));
        let mut streamed = String::new();

        let response = runtime
            .complete_streaming(
                &resolved,
                &CompletionRequest::new("Explain the slice"),
                |delta| {
                    streamed.push_str(delta);
                    Ok(())
                },
            )
            .expect("anthropic stream");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(
            body.pointer("/stream").and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(streamed, "Part one. Part two.");
        assert_eq!(response.output_text, "Part one. Part two.");
        assert_eq!(response.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(response.context_window_size, Some(1_050_000));
        assert_eq!(response.usage.input_tokens, 13);
        assert_eq!(response.usage.output_tokens, 5);
        assert_eq!(response.usage.cache_creation_tokens, 4);
        assert_eq!(response.usage.cache_read_tokens, 1);
    }

    #[test]
    fn copilot_runtime_exchanges_oauth_token_and_builds_openai_request() {
        let transport = RecordingTransport::with_json_responses(vec![
            serde_json::json!({
                "token": "copilot-bearer",
                "expires_at": 1_750_000_000,
                "refresh_in": 900,
                "endpoints": {"api": "https://api.githubcopilot.com"}
            }),
            serde_json::json!({
                "choices": [{
                    "finish_reason": "stop",
                    "message": {"content": "Copilot reply"}
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 4
                }
            }),
        ]);
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_oauth_provider("copilot", Some("gpt-4.1"));

        let response = runtime
            .complete(&resolved, &CompletionRequest::new("Say hi from Copilot"))
            .expect("copilot response");
        let requests = transport.take_requests();
        assert_eq!(requests.len(), 2);
        let exchange = &requests[0];
        let completion = &requests[1];
        let completion_body: serde_json::Value =
            serde_json::from_str(&completion.body).expect("completion request json");

        assert_eq!(exchange.method, "GET");
        assert_eq!(
            exchange.url,
            "https://api.github.com/copilot_internal/v2/token"
        );
        assert_eq!(
            exchange.headers.get("authorization").map(String::as_str),
            Some("Bearer oauth-token")
        );
        assert_eq!(
            exchange
                .headers
                .get("copilot-integration-id")
                .map(String::as_str),
            Some("vscode-chat")
        );
        assert_eq!(completion.method, "POST");
        assert_eq!(
            completion.url,
            "https://api.githubcopilot.com/chat/completions"
        );
        assert_eq!(
            completion.headers.get("authorization").map(String::as_str),
            Some("Bearer copilot-bearer")
        );
        assert_eq!(
            completion
                .headers
                .get("editor-plugin-version")
                .map(String::as_str),
            Some("copilot-chat/0.26.7")
        );
        assert_eq!(
            completion_body
                .pointer("/model")
                .and_then(serde_json::Value::as_str),
            Some("gpt-4.1")
        );
        assert_eq!(response.output_text, "Copilot reply");
        assert_eq!(response.usage.input_tokens, 12);
        assert_eq!(response.usage.output_tokens, 4);
    }

    #[test]
    fn copilot_tool_use_support_covers_gpt_and_claude_style_models() {
        let runtime = ProviderRuntime::new();

        assert!(
            runtime.supports_tool_use_for(&resolved_oauth_provider("copilot", Some("gpt-4.1")))
        );
        assert!(runtime.supports_tool_use_for(&resolved_oauth_provider(
            "copilot",
            Some("claude-sonnet-4.6")
        )));
    }

    #[test]
    fn anthropic_runtime_builds_tool_use_request_and_parses_tool_calls() {
        let transport = RecordingTransport::with_json_responses(vec![serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "Let me inspect that."},
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "glob",
                    "input": {"pattern": "Cargo.toml"}
                }
            ],
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 21,
                "output_tokens": 5
            }
        })]);
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("anthropic", None);

        let response = runtime
            .complete_with_tool_use(
                &resolved,
                &ToolUseRequest {
                    prompt: "Inspect the workspace".into(),
                    system_prompt: Some("You are helpful.".into()),
                    tools: vec![ProviderToolSpec {
                        name: "glob".into(),
                        description: "Find matching files".into(),
                        input_schema: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "pattern": {"type": "string"}
                            },
                            "required": ["pattern"],
                            "additionalProperties": false
                        }),
                    }],
                    rounds: vec![ToolConversationRound {
                        assistant_text: Some("Checking files.".into()),
                        calls: vec![ProviderToolCall {
                            call_id: "toolu_prev".into(),
                            tool_name: "glob".into(),
                            arguments: serde_json::json!({"pattern": "src/**/*.rs"}),
                        }],
                        results: vec![ProviderToolResultMessage {
                            call_id: "toolu_prev".into(),
                            content: "[\"src/main.rs\"]".into(),
                        }],
                    }],
                    ..ToolUseRequest::default()
                },
            )
            .expect("anthropic tool-use response");
        let requests = transport.take_requests();
        assert_eq!(requests.len(), 1);
        let completion = &requests[0];
        let body: serde_json::Value =
            serde_json::from_str(&completion.body).expect("tool-use request json");

        assert_eq!(completion.method, "POST");
        assert_eq!(completion.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(
            completion.headers.get("x-api-key").map(String::as_str),
            Some("secret-key")
        );
        assert_eq!(
            body.pointer("/tool_choice/type")
                .and_then(serde_json::Value::as_str),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/messages/0/content/0/text")
                .and_then(serde_json::Value::as_str),
            Some("Inspect the workspace")
        );
        assert_eq!(
            body.pointer("/messages/1/content/1/type")
                .and_then(serde_json::Value::as_str),
            Some("tool_use")
        );
        assert_eq!(
            body.pointer("/messages/2/content/0/tool_use_id")
                .and_then(serde_json::Value::as_str),
            Some("toolu_prev")
        );

        match response {
            ToolUseResponse::ToolCalls(batch) => {
                assert_eq!(
                    batch.assistant_text.as_deref(),
                    Some("Let me inspect that.")
                );
                assert_eq!(batch.calls.len(), 1);
                assert_eq!(batch.calls[0].call_id, "toolu_1");
                assert_eq!(batch.calls[0].tool_name, "glob");
                assert_eq!(
                    batch.calls[0].arguments,
                    serde_json::json!({"pattern": "Cargo.toml"})
                );
                assert_eq!(batch.stop_reason.as_deref(), Some("tool_use"));
                assert_eq!(batch.usage.input_tokens, 21);
                assert_eq!(batch.usage.output_tokens, 5);
            }
            other => panic!("expected anthropic tool-call response, got {other:?}"),
        }
    }

    #[test]
    fn copilot_runtime_exchanges_oauth_token_and_builds_tool_use_request() {
        let transport = RecordingTransport::with_json_responses(vec![
            serde_json::json!({
                "token": "copilot-bearer",
                "expires_at": 1_750_000_000,
                "refresh_in": 900,
                "endpoints": {"api": "https://api.githubcopilot.com"}
            }),
            serde_json::json!({
                "choices": [{
                    "finish_reason": "tool_calls",
                    "message": {
                        "content": "Checking repo state.",
                        "tool_calls": [{
                            "id": "call_copilot_1",
                            "type": "function",
                            "function": {
                                "name": "glob",
                                "arguments": "{\"pattern\":\"Cargo.toml\"}"
                            }
                        }]
                    }
                }],
                "usage": {
                    "prompt_tokens": 16,
                    "completion_tokens": 3
                }
            }),
        ]);
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_oauth_provider("copilot", Some("gpt-4.1"));

        let response = runtime
            .complete_with_tool_use(
                &resolved,
                &ToolUseRequest {
                    prompt: "Inspect the workspace".into(),
                    tools: vec![ProviderToolSpec {
                        name: "glob".into(),
                        description: "Find matching files".into(),
                        input_schema: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "pattern": {"type": "string"}
                            },
                            "required": ["pattern"],
                            "additionalProperties": false
                        }),
                    }],
                    ..ToolUseRequest::default()
                },
            )
            .expect("copilot tool-use response");
        let requests = transport.take_requests();
        assert_eq!(requests.len(), 2);
        let exchange = &requests[0];
        let completion = &requests[1];
        let body: serde_json::Value =
            serde_json::from_str(&completion.body).expect("tool-use request json");

        assert_eq!(exchange.method, "GET");
        assert_eq!(
            completion.url,
            "https://api.githubcopilot.com/chat/completions"
        );
        assert_eq!(
            completion.headers.get("authorization").map(String::as_str),
            Some("Bearer copilot-bearer")
        );
        assert_eq!(
            body.pointer("/tool_choice")
                .and_then(serde_json::Value::as_str),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/tools/0/function/name")
                .and_then(serde_json::Value::as_str),
            Some("glob")
        );

        match response {
            ToolUseResponse::ToolCalls(batch) => {
                assert_eq!(
                    batch.assistant_text.as_deref(),
                    Some("Checking repo state.")
                );
                assert_eq!(batch.calls.len(), 1);
                assert_eq!(batch.calls[0].tool_name, "glob");
                assert_eq!(
                    batch.calls[0].arguments,
                    serde_json::json!({"pattern": "Cargo.toml"})
                );
            }
            other => panic!("expected copilot tool-call response, got {other:?}"),
        }
    }

    #[test]
    fn copilot_runtime_streams_anthropic_models_after_token_exchange() {
        let transport = RecordingTransport::with_json_and_stream_bodies(
            vec![serde_json::json!({
                "token": "copilot-bearer",
                "expires_at": 1_750_000_000,
                "refresh_in": 900,
                "endpoints": {"api": "https://api.githubcopilot.com"}
            })],
            vec![concat!(
                "event: message_start\n",
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":15}}}\n\n",
                "event: content_block_delta\n",
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}\n\n",
                "event: content_block_delta\n",
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\" Copilot\"}}\n\n",
                "event: message_delta\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":6}}\n\n",
                "event: message_stop\n",
                "data: {\"type\":\"message_stop\"}\n\n",
            )],
        );
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_oauth_provider("copilot", Some("claude-sonnet-4.6"));
        let mut streamed = String::new();

        let response = runtime
            .complete_streaming(
                &resolved,
                &CompletionRequest::new("Explain the slice"),
                |delta| {
                    streamed.push_str(delta);
                    Ok(())
                },
            )
            .expect("copilot stream");
        let requests = transport.take_requests();
        assert_eq!(requests.len(), 2);
        let stream_request = &requests[1];
        let body: serde_json::Value =
            serde_json::from_str(&stream_request.body).expect("request json");

        assert_eq!(stream_request.method, "POST");
        assert_eq!(
            stream_request.url,
            "https://api.githubcopilot.com/v1/messages"
        );
        assert_eq!(
            stream_request
                .headers
                .get("authorization")
                .map(String::as_str),
            Some("Bearer copilot-bearer")
        );
        assert_eq!(
            stream_request
                .headers
                .get("anthropic-version")
                .map(String::as_str),
            Some(DEFAULT_ANTHROPIC_API_VERSION)
        );
        assert_eq!(
            stream_request
                .headers
                .get("copilot-integration-id")
                .map(String::as_str),
            Some("vscode-chat")
        );
        assert_eq!(
            body.pointer("/model").and_then(serde_json::Value::as_str),
            Some("claude-sonnet-4.6")
        );
        assert_eq!(streamed, "Hello Copilot");
        assert_eq!(response.output_text, "Hello Copilot");
        assert_eq!(response.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(response.usage.input_tokens, 15);
        assert_eq!(response.usage.output_tokens, 6);
    }

    #[test]
    fn copilot_runtime_supports_tool_use_for_claude_models() {
        let transport = RecordingTransport::with_json_responses(vec![
            serde_json::json!({
                "token": "copilot-bearer",
                "expires_at": 1_750_000_000,
                "refresh_in": 900,
                "endpoints": {"api": "https://api.githubcopilot.com"}
            }),
            serde_json::json!({
                "id": "msg_456",
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_copilot_1",
                    "name": "glob",
                    "input": {"pattern": "Cargo.toml"}
                }],
                "stop_reason": "tool_use",
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 2
                }
            }),
        ]);
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_oauth_provider("copilot", Some("claude-sonnet-4.6"));

        let response = runtime
            .complete_with_tool_use(
                &resolved,
                &ToolUseRequest {
                    prompt: "Inspect the workspace".into(),
                    tools: vec![ProviderToolSpec {
                        name: "glob".into(),
                        description: "Find matching files".into(),
                        input_schema: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "pattern": {"type": "string"}
                            },
                            "required": ["pattern"],
                            "additionalProperties": false
                        }),
                    }],
                    ..ToolUseRequest::default()
                },
            )
            .expect("copilot claude tool-use response");
        let requests = transport.take_requests();
        assert_eq!(requests.len(), 2);
        let completion = &requests[1];
        let body: serde_json::Value =
            serde_json::from_str(&completion.body).expect("tool-use request json");

        assert_eq!(completion.url, "https://api.githubcopilot.com/v1/messages");
        assert_eq!(
            completion.headers.get("authorization").map(String::as_str),
            Some("Bearer copilot-bearer")
        );
        assert_eq!(
            body.pointer("/tool_choice/type")
                .and_then(serde_json::Value::as_str),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/tools/0/name")
                .and_then(serde_json::Value::as_str),
            Some("glob")
        );

        match response {
            ToolUseResponse::ToolCalls(batch) => {
                assert_eq!(batch.calls.len(), 1);
                assert_eq!(batch.calls[0].call_id, "toolu_copilot_1");
                assert_eq!(batch.calls[0].tool_name, "glob");
                assert_eq!(
                    batch.calls[0].arguments,
                    serde_json::json!({"pattern": "Cargo.toml"})
                );
                assert_eq!(batch.stop_reason.as_deref(), Some("tool_use"));
            }
            other => panic!("expected copilot claude tool-call response, got {other:?}"),
        }
    }

    #[test]
    fn openai_request_includes_reasoning_effort_for_high() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "ok"}}],
            "usage": {"prompt_tokens": 4, "completion_tokens": 1}
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("openai", Some("gpt-4.1"));
        let request = CompletionRequest {
            prompt: "ping".into(),
            effort_level: Some("high".into()),
            ..CompletionRequest::default()
        };

        runtime.complete(&resolved, &request).expect("complete");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("body json");

        assert_eq!(
            body.pointer("/reasoning_effort")
                .and_then(serde_json::Value::as_str),
            Some("high"),
            "reasoning_effort should be 'high' for high effort level"
        );
    }

    #[test]
    fn anthropic_request_includes_thinking_for_high_effort_claude_model() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "id": "msg_think_1",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "deep thought"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 5, "output_tokens": 2}
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("anthropic", Some("claude-opus-4-7"));
        let request = CompletionRequest {
            prompt: "think deeply".into(),
            effort_level: Some("max".into()),
            ..CompletionRequest::default()
        };

        runtime.complete(&resolved, &request).expect("complete");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("body json");

        assert_eq!(
            body.pointer("/thinking/type")
                .and_then(serde_json::Value::as_str),
            Some("enabled"),
            "thinking type should be 'enabled' for high effort with claude model"
        );
        assert_eq!(
            body.pointer("/thinking/budget_tokens")
                .and_then(serde_json::Value::as_u64),
            Some(10000),
            "thinking budget_tokens should be 10000"
        );
    }

    #[test]
    fn copilot_openai_request_includes_reasoning_effort_header_for_max_effort() {
        let transport = RecordingTransport::with_json_responses(vec![
            serde_json::json!({
                "token": "copilot-bearer",
                "expires_at": 1_750_000_000,
                "refresh_in": 900,
                "endpoints": {"api": "https://api.githubcopilot.com"}
            }),
            serde_json::json!({
                "choices": [{"finish_reason": "stop", "message": {"content": "done"}}],
                "usage": {"prompt_tokens": 4, "completion_tokens": 1}
            }),
        ]);
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_oauth_provider("copilot", Some("gpt-4.1"));
        let request = CompletionRequest {
            prompt: "ping".into(),
            effort_level: Some("max".into()),
            ..CompletionRequest::default()
        };

        runtime.complete(&resolved, &request).expect("complete");
        let requests = transport.take_requests();
        // requests[0] is the copilot token exchange; requests[1] is the completion
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1]
                .headers
                .get("x-reasoning-effort")
                .map(String::as_str),
            Some("high"),
            "x-reasoning-effort header should be 'high' for max effort level"
        );
    }

    #[test]
    fn openai_401_auth_error_surfaces_as_validation_error() {
        let transport = RecordingTransport::with_http_error(
            401,
            serde_json::json!({"error": {"message": "Invalid API key"}}),
        );
        let runtime = ProviderRuntime::with_transport(transport as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("openai", Some("gpt-4.1"));
        let request = CompletionRequest::new("hello");

        let err = runtime
            .complete(&resolved, &request)
            .expect_err("401 should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("401"),
            "error should mention status code; got: {msg}"
        );
        assert!(
            msg.contains("Invalid API key"),
            "error should include provider message; got: {msg}"
        );
    }

    #[test]
    fn openai_429_rate_limit_error_surfaces_message() {
        let transport = RecordingTransport::with_http_error(
            429,
            serde_json::json!({"error": {"message": "Rate limit exceeded. Please retry after 60 seconds."}}),
        );
        let runtime = ProviderRuntime::with_transport(transport as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("openai", Some("gpt-4.1"));
        let request = CompletionRequest::new("ping");

        let err = runtime
            .complete(&resolved, &request)
            .expect_err("429 should fail");
        let msg = err.to_string();
        assert!(msg.contains("429"), "error should mention 429; got: {msg}");
        assert!(
            msg.contains("Rate limit"),
            "error should surface rate-limit message; got: {msg}"
        );
    }

    #[test]
    fn anthropic_500_server_error_surfaces_as_validation_error() {
        let transport = RecordingTransport::with_http_error(
            500,
            serde_json::json!({"error": {"message": "Internal server error"}}),
        );
        let runtime = ProviderRuntime::with_transport(transport as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("anthropic", Some("claude-opus-4-7"));
        let request = CompletionRequest::new("hello");

        let err = runtime
            .complete(&resolved, &request)
            .expect_err("500 should fail");
        let msg = err.to_string();
        assert!(msg.contains("500"), "error should mention 500; got: {msg}");
    }

    #[test]
    fn openai_malformed_json_body_surfaces_parse_error() {
        let transport = RecordingTransport::with_raw_body(200, "not valid json {{{");
        let runtime = ProviderRuntime::with_transport(transport as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("openai", Some("gpt-4.1"));
        let request = CompletionRequest::new("hello");

        let err = runtime
            .complete(&resolved, &request)
            .expect_err("malformed json should fail");
        assert!(
            !err.to_string().is_empty(),
            "error message should be non-empty; got: {err}"
        );
    }

    #[test]
    fn network_error_surfaces_as_validation_error() {
        let transport = RecordingTransport::with_network_error("connection refused");
        let runtime = ProviderRuntime::with_transport(transport as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("openai", Some("gpt-4.1"));
        let request = CompletionRequest::new("hello");

        let err = runtime
            .complete(&resolved, &request)
            .expect_err("network error should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("provider request failed"),
            "error should mention provider request failed; got: {msg}"
        );
        assert!(
            msg.contains("connection refused"),
            "error should include underlying cause; got: {msg}"
        );
    }

    // ─── SigV4 signing ────────────────────────────────────────────────────────

    #[test]
    fn sigv4_datetime_formats_correctly() {
        let dt = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let formatted = sigv4_datetime(dt);
        // 1700000000 = 2023-11-14T22:13:20Z
        assert_eq!(formatted, "20231114T221320Z");
        assert_eq!(formatted.len(), 16);
    }

    #[test]
    fn sha256_hex_produces_known_value() {
        // Empty string SHA-256 is well-known.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sign_request_headers_produces_authorization_header() {
        let credentials = AwsCredentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
            region: "us-east-1".into(),
        };
        let mut headers = BTreeMap::from([
            ("accept".into(), "application/json".into()),
            ("content-type".into(), "application/json".into()),
        ]);
        let body = br#"{"anthropic_version":"bedrock-2023-05-31"}"#;
        let datetime = "20231114T221320Z";
        let url = "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-opus-4-7/invoke";

        sign_request_headers(&mut headers, "POST", url, body, &credentials, datetime)
            .expect("sign headers");

        let auth = headers.get("authorization").expect("authorization header");
        assert!(auth.starts_with("AWS4-HMAC-SHA256 "), "auth header: {auth}");
        assert!(
            auth.contains("Credential=AKIAIOSFODNN7EXAMPLE/"),
            "auth header: {auth}"
        );
        assert!(auth.contains("SignedHeaders="), "auth header: {auth}");
        assert!(auth.contains("Signature="), "auth header: {auth}");
        assert!(
            auth.contains("bedrock"),
            "auth header should contain service: {auth}"
        );

        // Standard SigV4 headers should be injected.
        assert!(headers.contains_key("x-amz-date"));
        assert!(headers.contains_key("x-amz-content-sha256"));
        assert!(
            !headers.contains_key("x-amz-security-token"),
            "no session token"
        );
    }

    #[test]
    fn sign_request_headers_includes_session_token_when_present() {
        let credentials = AwsCredentials {
            access_key_id: "AKID".into(),
            secret_access_key: "secret".into(),
            session_token: Some("session-xyz".into()),
            region: "us-west-2".into(),
        };
        let mut headers = BTreeMap::from([("content-type".into(), "application/json".into())]);
        sign_request_headers(
            &mut headers,
            "POST",
            "https://bedrock-runtime.us-west-2.amazonaws.com/model/x/invoke",
            b"{}",
            &credentials,
            "20231114T221320Z",
        )
        .expect("sign headers with session token");

        assert_eq!(
            headers.get("x-amz-security-token").map(String::as_str),
            Some("session-xyz")
        );
    }

    // ─── Bedrock request builder ──────────────────────────────────────────────

    fn resolved_bedrock_provider(model: Option<&str>) -> ResolvedProviderExecution {
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            selected_model: model.map(ToString::to_string),
            ..AgentSettings::default()
        };

        ProviderResolver::builtin()
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    ("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE".to_string()),
                    (
                        "AWS_SECRET_ACCESS_KEY",
                        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
                    ),
                    ("AWS_REGION", "us-east-1".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect("resolve bedrock provider")
    }

    #[test]
    fn bedrock_runtime_builds_invoke_request_with_sigv4_headers() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "content": [{"type": "text", "text": "Hello from Bedrock"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_bedrock_provider(None);
        let request = CompletionRequest::new("Tell me a joke");

        let response = runtime
            .complete(&resolved, &request)
            .expect("bedrock response");
        let recorded = transport.take_request();
        let body: serde_json::Value =
            serde_json::from_str(&recorded.body).expect("bedrock request json");

        assert_eq!(recorded.method, "POST");
        assert!(
            recorded.url.contains("/model/"),
            "URL should contain model path: {}",
            recorded.url
        );
        assert!(
            recorded.url.ends_with("/invoke"),
            "URL should end with /invoke: {}",
            recorded.url
        );

        // Must have SigV4 headers.
        assert!(
            recorded.headers.contains_key("authorization"),
            "missing authorization header"
        );
        assert!(
            recorded
                .headers
                .get("authorization")
                .unwrap()
                .starts_with("AWS4-HMAC-SHA256"),
            "auth header should be SigV4"
        );
        assert!(recorded.headers.contains_key("x-amz-date"));
        assert!(recorded.headers.contains_key("x-amz-content-sha256"));

        // Body must contain Bedrock-specific field.
        assert_eq!(
            body.get("anthropic_version")
                .and_then(serde_json::Value::as_str),
            Some("bedrock-2023-05-31")
        );

        assert_eq!(response.output_text, "Hello from Bedrock");
    }

    #[test]
    fn bedrock_runtime_builds_invoke_stream_request() {
        let transport = RecordingTransport::with_stream_body(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Streaming!\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
        );
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_bedrock_provider(None);
        let request = CompletionRequest::new("Stream this");

        let mut deltas = Vec::new();
        let response = runtime
            .complete_streaming(&resolved, &request, |delta| {
                deltas.push(delta.to_string());
                Ok(())
            })
            .expect("bedrock streaming");
        let recorded = transport.take_request();

        assert!(
            recorded.url.ends_with("/invoke-with-response-stream"),
            "streaming URL should end with /invoke-with-response-stream: {}",
            recorded.url
        );
        assert!(
            recorded
                .headers
                .get("authorization")
                .unwrap()
                .starts_with("AWS4-HMAC-SHA256"),
            "must have SigV4 auth"
        );
        assert_eq!(response.output_text, "Streaming!");
        assert_eq!(deltas, vec!["Streaming!"]);
    }

    // ── provider-local-models: runtime ────────────────────────────────────────

    fn resolved_local_provider(model: Option<&str>) -> ResolvedProviderExecution {
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            selected_model: model.map(ToString::to_string),
            ..AgentSettings::default()
        };
        ProviderResolver::builtin()
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                std::iter::empty::<(&str, String)>(),
                &ProviderSelection::default(),
            )
            .expect("resolve local provider")
    }

    #[test]
    fn local_provider_request_omits_authorization_header() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 1}
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_local_provider(Some("llama3.2"));
        let request = CompletionRequest::new("Hello");

        runtime
            .complete(&resolved, &request)
            .expect("local completion");
        let recorded = transport.take_request();

        assert!(
            !recorded.headers.contains_key("authorization"),
            "local provider must not send an Authorization header; got: {:?}",
            recorded.headers
        );
    }

    #[test]
    fn local_provider_request_targets_ollama_default_base() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "ok"}}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 1}
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_local_provider(Some("llama3.2"));
        let request = CompletionRequest::new("Ping");

        runtime
            .complete(&resolved, &request)
            .expect("local completion");
        let recorded = transport.take_request();

        assert_eq!(
            recorded.url, "http://localhost:11434/v1/chat/completions",
            "local provider should default to Ollama base URL"
        );
        assert_eq!(recorded.method, "POST");
    }

    #[test]
    fn local_provider_request_uses_arbitrary_model_id() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "ok"}}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 1}
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_local_provider(Some("phi4:14b-q4_K_M"));
        let request = CompletionRequest::new("Ping");

        runtime
            .complete(&resolved, &request)
            .expect("local completion");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("parse body");

        assert_eq!(
            body.pointer("/model").and_then(serde_json::Value::as_str),
            Some("phi4:14b-q4_K_M"),
            "arbitrary model id must be forwarded verbatim"
        );
    }

    // ─── Bedrock AwsBearer ────────────────────────────────────────────────────

    fn resolved_bedrock_bearer_provider(model: Option<&str>) -> ResolvedProviderExecution {
        let settings = AgentSettings {
            selected_provider: Some("bedrock".into()),
            selected_model: model.map(ToString::to_string),
            ..AgentSettings::default()
        };

        ProviderResolver::builtin()
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    (
                        "AWS_BEARER_TOKEN_BEDROCK",
                        "my-bedrock-bearer-token".to_string(),
                    ),
                    ("AWS_REGION", "us-west-2".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect("resolve bedrock bearer provider")
    }

    #[test]
    fn bedrock_runtime_uses_bearer_auth_when_aws_bearer_token_is_set() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "content": [{"type": "text", "text": "Bearer response"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 4, "output_tokens": 3}
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_bedrock_bearer_provider(None);
        let request = CompletionRequest::new("Hello via bearer");

        let response = runtime
            .complete(&resolved, &request)
            .expect("bedrock bearer response");
        let recorded = transport.take_request();

        assert_eq!(recorded.method, "POST");
        assert!(
            recorded.url.ends_with("/invoke"),
            "URL should end with /invoke: {}",
            recorded.url
        );
        // Must use Bearer auth, NOT SigV4.
        assert_eq!(
            recorded.headers.get("authorization").map(String::as_str),
            Some("Bearer my-bedrock-bearer-token"),
            "authorization header should be a Bearer token"
        );
        assert!(
            !recorded.headers.contains_key("x-amz-date"),
            "bearer auth must not include SigV4 date header"
        );
        assert!(
            !recorded.headers.contains_key("x-amz-content-sha256"),
            "bearer auth must not include SigV4 hash header"
        );
        assert_eq!(response.output_text, "Bearer response");
    }

    // ─── Gemini ───────────────────────────────────────────────────────────────

    fn resolved_gemini_provider(model: Option<&str>) -> ResolvedProviderExecution {
        let settings = AgentSettings {
            selected_provider: Some("gemini".into()),
            selected_model: model.map(ToString::to_string),
            ..AgentSettings::default()
        };

        ProviderResolver::builtin()
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [("GEMINI_API_KEY", "gemini-secret".to_string())],
                &ProviderSelection::default(),
            )
            .expect("resolve gemini provider")
    }

    #[test]
    fn gemini_runtime_builds_generate_content_request() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Gemini reply"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 6,
                "candidatesTokenCount": 3
            }
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_gemini_provider(Some("gemini-2.0-flash"));
        let request = CompletionRequest {
            prompt: "Hello Gemini".into(),
            system_prompt: Some("Be brief".into()),
            max_output_tokens: Some(256),
            temperature: Some(0.5),
            effort_level: None,
        };

        let response = runtime
            .complete(&resolved, &request)
            .expect("gemini response");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        // URL must point at Gemini generateContent with key in query string.
        assert!(
            recorded.url.contains("generativelanguage.googleapis.com"),
            "URL should target Gemini: {}",
            recorded.url
        );
        assert!(
            recorded.url.contains(":generateContent"),
            "URL should contain :generateContent: {}",
            recorded.url
        );
        assert!(
            recorded.url.contains("key=gemini-secret"),
            "URL should include API key: {}",
            recorded.url
        );
        // No Authorization header – key is in query param.
        assert!(
            !recorded.headers.contains_key("authorization"),
            "Gemini native must not use Authorization header"
        );
        // System instruction in body.
        assert_eq!(
            body.pointer("/systemInstruction/parts/0/text")
                .and_then(serde_json::Value::as_str),
            Some("Be brief"),
        );
        // User content.
        assert_eq!(
            body.pointer("/contents/0/parts/0/text")
                .and_then(serde_json::Value::as_str),
            Some("Hello Gemini"),
        );
        // generationConfig fields.
        assert_eq!(
            body.pointer("/generationConfig/maxOutputTokens")
                .and_then(serde_json::Value::as_u64),
            Some(256),
        );
        let temp = body
            .pointer("/generationConfig/temperature")
            .and_then(serde_json::Value::as_f64)
            .expect("temperature");
        assert!((temp - 0.5).abs() < 1e-6);

        assert_eq!(response.output_text, "Gemini reply");
        assert_eq!(response.stop_reason.as_deref(), Some("STOP"));
        assert_eq!(response.usage.input_tokens, 6);
        assert_eq!(response.usage.output_tokens, 3);
    }

    #[test]
    fn gemini_streaming_runtime_builds_stream_request_and_collects_deltas() {
        // Each SSE event carries the same generateContent JSON shape.
        let transport = RecordingTransport::with_stream_body(concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello \"}],\"role\":\"model\"}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Gemini\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":2}}\n\n",
        ));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_gemini_provider(Some("gemini-2.0-flash"));
        let mut streamed = String::new();

        let response = runtime
            .complete_streaming(&resolved, &CompletionRequest::new("Say hi"), |delta| {
                streamed.push_str(delta);
                Ok(())
            })
            .expect("gemini stream");

        let recorded = transport.take_request();
        assert_eq!(recorded.method, "POST");
        // Must use streamGenerateContent action.
        assert!(
            recorded.url.contains(":streamGenerateContent"),
            "URL should use streamGenerateContent: {}",
            recorded.url
        );
        // Must request SSE format.
        assert!(
            recorded.url.contains("alt=sse"),
            "URL should include alt=sse: {}",
            recorded.url
        );
        // API key in query param, no Authorization header.
        assert!(
            recorded.url.contains("key=gemini-secret"),
            "URL should include API key: {}",
            recorded.url
        );
        assert!(
            !recorded.headers.contains_key("authorization"),
            "Gemini native must not use Authorization header"
        );

        assert_eq!(streamed, "Hello Gemini");
        assert_eq!(response.output_text, "Hello Gemini");
        assert_eq!(response.stop_reason.as_deref(), Some("STOP"));
        assert_eq!(response.usage.input_tokens, 5);
        assert_eq!(response.usage.output_tokens, 2);
    }

    #[test]
    fn gemini_streaming_returns_empty_text_error() {
        // An SSE stream with no text parts should yield a validation error.
        let transport = RecordingTransport::with_stream_body(
            "data: {\"candidates\":[{\"content\":{\"parts\":[]}}]}\n\n",
        );
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_gemini_provider(None);

        let err = runtime
            .complete_streaming(&resolved, &CompletionRequest::new("ping"), |_| Ok(()))
            .expect_err("empty stream should fail");
        assert!(
            err.to_string().contains("text content"),
            "error should mention missing text: {err}"
        );
    }

    #[test]
    fn gemini_tool_use_builds_request_and_returns_tool_calls() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": {"location": "London"}
                        }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5
            }
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_gemini_provider(None);

        let response = runtime
            .complete_with_tool_use(
                &resolved,
                &ToolUseRequest {
                    prompt: "use a tool".into(),
                    tools: vec![ProviderToolSpec {
                        name: "get_weather".into(),
                        description: "Get weather".into(),
                        input_schema: serde_json::json!({
                            "type": "object",
                            "properties": {"location": {"type": "string"}}
                        }),
                    }],
                    ..ToolUseRequest::default()
                },
            )
            .expect("gemini tool use");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert!(recorded.url.contains(":generateContent"));
        assert!(recorded.url.contains("key=gemini-secret"));
        assert_eq!(
            body.pointer("/tools/0/functionDeclarations/0/name")
                .and_then(serde_json::Value::as_str),
            Some("get_weather")
        );
        assert_eq!(
            body.pointer("/tool_config/function_calling_config/mode")
                .and_then(serde_json::Value::as_str),
            Some("AUTO")
        );

        match response {
            ToolUseResponse::ToolCalls(batch) => {
                assert_eq!(batch.calls.len(), 1);
                assert_eq!(batch.calls[0].call_id, "gemini-call-0");
                assert_eq!(batch.calls[0].tool_name, "get_weather");
                assert_eq!(batch.calls[0].arguments["location"], "London");
                assert_eq!(batch.usage.input_tokens, 10);
                assert_eq!(batch.usage.output_tokens, 5);
            }
            other => panic!("expected tool calls, got {other:?}"),
        }
    }

    #[test]
    fn gemini_tool_use_returns_final_text_when_no_function_call() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "All done"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 3,
                "candidatesTokenCount": 2
            }
        }));
        let runtime = ProviderRuntime::with_transport(transport as Arc<dyn HttpTransport>);
        let resolved = resolved_gemini_provider(Some("gemini-2.0-flash"));

        let response = runtime
            .complete_with_tool_use(
                &resolved,
                &ToolUseRequest {
                    prompt: "finish".into(),
                    ..ToolUseRequest::default()
                },
            )
            .expect("gemini final text");

        match response {
            ToolUseResponse::Final(final_response) => {
                assert_eq!(final_response.output_text, "All done");
                assert_eq!(final_response.stop_reason.as_deref(), Some("STOP"));
                assert_eq!(final_response.usage.input_tokens, 3);
                assert_eq!(final_response.usage.output_tokens, 2);
            }
            other => panic!("expected final text, got {other:?}"),
        }
    }

    #[test]
    fn gemini_tool_use_includes_prior_rounds_as_contents() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "candidates": [{
                "content": {"parts": [{"text": "Done"}], "role": "model"},
                "finishReason": "STOP"
            }]
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_gemini_provider(Some("gemini-2.0-flash"));

        runtime
            .complete_with_tool_use(
                &resolved,
                &ToolUseRequest {
                    prompt: "Summarize".into(),
                    tools: vec![ProviderToolSpec {
                        name: "file_read".into(),
                        description: "Read a file".into(),
                        input_schema: serde_json::json!({"type": "object"}),
                    }],
                    rounds: vec![ToolConversationRound {
                        assistant_text: Some("Reading first.".into()),
                        calls: vec![ProviderToolCall {
                            call_id: "gemini-call-0".into(),
                            tool_name: "file_read".into(),
                            arguments: serde_json::json!({"path": "README.md"}),
                        }],
                        results: vec![ProviderToolResultMessage {
                            call_id: "gemini-call-0".into(),
                            content: "README contents".into(),
                        }],
                    }],
                    ..ToolUseRequest::default()
                },
            )
            .expect("gemini request with rounds");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(
            body.pointer("/contents/0/role")
                .and_then(serde_json::Value::as_str),
            Some("user")
        );
        assert_eq!(
            body.pointer("/contents/1/parts/0/text")
                .and_then(serde_json::Value::as_str),
            Some("Reading first.")
        );
        assert_eq!(
            body.pointer("/contents/1/parts/1/functionCall/name")
                .and_then(serde_json::Value::as_str),
            Some("file_read")
        );
        assert_eq!(
            body.pointer("/contents/2/parts/0/functionResponse/name")
                .and_then(serde_json::Value::as_str),
            Some("file_read")
        );
        assert_eq!(
            body.pointer("/contents/2/parts/0/functionResponse/response/output")
                .and_then(serde_json::Value::as_str),
            Some("README contents")
        );
    }

    // ─── Vertex AI ────────────────────────────────────────────────────────────

    fn resolved_vertex_provider(model: Option<&str>) -> ResolvedProviderExecution {
        let settings = AgentSettings {
            selected_provider: Some("vertex".into()),
            selected_model: model.map(ToString::to_string),
            ..AgentSettings::default()
        };

        ProviderResolver::builtin()
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [
                    ("VERTEXAI_PROJECT", "my-gcp-project".to_string()),
                    ("VERTEXAI_LOCATION", "us-central1".to_string()),
                    ("GOOGLE_BEARER_TOKEN", "gcp-oauth-token".to_string()),
                ],
                &ProviderSelection::default(),
            )
            .expect("resolve vertex provider")
    }

    #[test]
    fn vertex_runtime_builds_generate_content_request_with_bearer_auth() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Vertex reply"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 8,
                "candidatesTokenCount": 4
            }
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_vertex_provider(Some("gemini-2.0-flash"));
        let request = CompletionRequest::new("Hello Vertex");

        let response = runtime
            .complete(&resolved, &request)
            .expect("vertex response");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        // URL must be location-specific Vertex AI endpoint.
        assert!(
            recorded
                .url
                .contains("us-central1-aiplatform.googleapis.com"),
            "URL should be location-specific: {}",
            recorded.url
        );
        assert!(
            recorded.url.contains("/projects/my-gcp-project/"),
            "URL should contain project: {}",
            recorded.url
        );
        assert!(
            recorded.url.contains("/locations/us-central1/"),
            "URL should contain location: {}",
            recorded.url
        );
        assert!(
            recorded.url.contains(":generateContent"),
            "URL should end with :generateContent: {}",
            recorded.url
        );
        // Bearer auth header.
        assert_eq!(
            recorded.headers.get("authorization").map(String::as_str),
            Some("Bearer gcp-oauth-token"),
            "Vertex must use Bearer auth"
        );
        // Body has standard generateContent shape.
        assert_eq!(
            body.pointer("/contents/0/parts/0/text")
                .and_then(serde_json::Value::as_str),
            Some("Hello Vertex"),
        );
        assert_eq!(response.output_text, "Vertex reply");
        assert_eq!(response.usage.input_tokens, 8);
        assert_eq!(response.usage.output_tokens, 4);
    }

    #[test]
    fn vertex_tool_use_builds_request_with_bearer_auth() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "lookup",
                            "args": {"id": 7}
                        }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_vertex_provider(Some("gemini-2.0-flash"));

        let response = runtime
            .complete_with_tool_use(
                &resolved,
                &ToolUseRequest {
                    prompt: "lookup".into(),
                    tools: vec![ProviderToolSpec {
                        name: "lookup".into(),
                        description: "Lookup by id".into(),
                        input_schema: serde_json::json!({"type": "object"}),
                    }],
                    ..ToolUseRequest::default()
                },
            )
            .expect("vertex tool use");
        let recorded = transport.take_request();

        assert!(
            recorded
                .url
                .contains("us-central1-aiplatform.googleapis.com")
        );
        assert!(recorded.url.contains(":generateContent"));
        assert_eq!(
            recorded.headers.get("authorization").map(String::as_str),
            Some("Bearer gcp-oauth-token")
        );
        match response {
            ToolUseResponse::ToolCalls(batch) => {
                assert_eq!(batch.calls[0].tool_name, "lookup");
                assert_eq!(batch.calls[0].arguments["id"], 7);
            }
            other => panic!("expected tool calls, got {other:?}"),
        }
    }

    #[test]
    fn vertex_streaming_runtime_builds_stream_request_and_collects_deltas() {
        let transport = RecordingTransport::with_stream_body(concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello \"}],\"role\":\"model\"}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Vertex\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":3}}\n\n",
        ));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_vertex_provider(Some("gemini-2.0-flash"));
        let mut streamed = String::new();

        let response = runtime
            .complete_streaming(&resolved, &CompletionRequest::new("Say hi"), |delta| {
                streamed.push_str(delta);
                Ok(())
            })
            .expect("vertex stream");

        let recorded = transport.take_request();
        assert_eq!(recorded.method, "POST");
        // Must use streamGenerateContent action.
        assert!(
            recorded.url.contains(":streamGenerateContent"),
            "URL should use streamGenerateContent: {}",
            recorded.url
        );
        // Must be Vertex AI endpoint, not Gemini REST.
        assert!(
            recorded
                .url
                .contains("us-central1-aiplatform.googleapis.com"),
            "URL should be Vertex endpoint: {}",
            recorded.url
        );
        // Bearer auth header.
        assert_eq!(
            recorded.headers.get("authorization").map(String::as_str),
            Some("Bearer gcp-oauth-token"),
            "Vertex stream must use Bearer auth"
        );

        assert_eq!(streamed, "Hello Vertex");
        assert_eq!(response.output_text, "Hello Vertex");
        assert_eq!(response.stop_reason.as_deref(), Some("STOP"));
        assert_eq!(response.usage.input_tokens, 7);
        assert_eq!(response.usage.output_tokens, 3);
    }

    // ─── Azure OpenAI ─────────────────────────────────────────────────────────

    fn resolved_azure_provider(model: Option<&str>) -> ResolvedProviderExecution {
        let settings = AgentSettings {
            selected_provider: Some("azure".into()),
            selected_model: model.map(ToString::to_string),
            // Override api_base via settings since the env-based endpoint is not
            // injectable through the standard env map in unit tests.
            providers: std::collections::BTreeMap::from([(
                "azure".into(),
                crate::ProviderOverride {
                    api_base: Some("https://my-resource.openai.azure.com".into()),
                    ..crate::ProviderOverride::default()
                },
            )]),
            ..AgentSettings::default()
        };

        ProviderResolver::builtin()
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [("AZURE_OPENAI_API_KEY", "azure-secret".to_string())],
                &ProviderSelection::default(),
            )
            .expect("resolve azure provider")
    }

    #[test]
    fn azure_runtime_builds_chat_completions_request_with_api_key_header() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"content": "Azure reply"}
            }],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 3
            }
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_azure_provider(Some("gpt-4o"));
        let request = CompletionRequest {
            prompt: "Hello Azure".into(),
            system_prompt: Some("Be concise".into()),
            max_output_tokens: Some(128),
            temperature: Some(0.3),
            effort_level: None,
        };

        let response = runtime
            .complete(&resolved, &request)
            .expect("azure response");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        // URL must use Azure deployment format.
        assert!(
            recorded
                .url
                .contains("my-resource.openai.azure.com/openai/deployments/gpt-4o/"),
            "URL should be Azure deployment path: {}",
            recorded.url
        );
        assert!(
            recorded.url.contains("api-version="),
            "URL should include api-version: {}",
            recorded.url
        );
        // Auth uses api-key header, NOT Authorization Bearer.
        assert_eq!(
            recorded.headers.get("api-key").map(String::as_str),
            Some("azure-secret"),
            "Azure must use api-key header"
        );
        assert!(
            !recorded.headers.contains_key("authorization"),
            "Azure must not use Authorization header"
        );
        // System message in body.
        assert_eq!(
            body.pointer("/messages/0/role")
                .and_then(serde_json::Value::as_str),
            Some("system")
        );
        assert_eq!(
            body.pointer("/messages/0/content")
                .and_then(serde_json::Value::as_str),
            Some("Be concise")
        );
        // User content.
        assert_eq!(
            body.pointer("/messages/1/content")
                .and_then(serde_json::Value::as_str),
            Some("Hello Azure")
        );
        assert_eq!(response.output_text, "Azure reply");
        assert_eq!(response.stop_reason.as_deref(), Some("stop"));
        assert_eq!(response.usage.input_tokens, 5);
        assert_eq!(response.usage.output_tokens, 3);
    }

    #[test]
    fn azure_runtime_streaming_builds_stream_request() {
        let transport = RecordingTransport::with_stream_body(concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Azure\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" stream\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n",
        ));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_azure_provider(Some("gpt-4o"));
        let mut streamed = String::new();

        let response = runtime
            .complete_streaming(&resolved, &CompletionRequest::new("stream"), |delta| {
                streamed.push_str(delta);
                Ok(())
            })
            .expect("azure stream");
        let recorded = transport.take_request();

        assert!(
            recorded
                .url
                .contains("my-resource.openai.azure.com/openai/deployments/gpt-4o/"),
            "streaming URL should be Azure deployment path: {}",
            recorded.url
        );
        assert_eq!(
            recorded.headers.get("api-key").map(String::as_str),
            Some("azure-secret")
        );
        assert_eq!(streamed, "Azure stream");
        assert_eq!(response.output_text, "Azure stream");
    }

    #[test]
    fn azure_supports_streaming_and_tool_use() {
        let runtime = ProviderRuntime::new();
        assert!(runtime.supports_streaming("azure"));
        assert!(runtime.supports_tool_use("azure"));
    }

    #[test]
    fn string_tool_use_helper_matches_resolved_runtime_protocols() {
        let runtime = ProviderRuntime::new();
        let cases = [
            ("openai", resolved_provider("openai", Some("gpt-4.1"))),
            (
                "anthropic",
                resolved_provider("anthropic", Some("claude-opus-4-7")),
            ),
            (
                "copilot",
                resolved_oauth_provider("copilot", Some("gpt-4.1")),
            ),
            ("bedrock", resolved_bedrock_provider(None)),
            ("azure", resolved_azure_provider(None)),
            ("local", resolved_local_provider(Some("llama3.2"))),
            ("groq", resolved_provider("groq", Some("llama-3.3-70b"))),
            ("gemini", resolved_gemini_provider(Some("gemini-2.0-flash"))),
            ("vertex", resolved_vertex_provider(Some("gemini-2.0-flash"))),
        ];

        for (provider_id, resolved) in cases {
            assert_eq!(
                runtime.supports_tool_use(provider_id),
                runtime.supports_tool_use_for(&resolved),
                "string helper and resolved helper should agree for {provider_id}"
            );
        }
    }

    #[test]
    fn gemini_and_vertex_support_streaming_and_tool_use() {
        let runtime = ProviderRuntime::new();
        assert!(runtime.supports_streaming("gemini"));
        assert!(runtime.supports_streaming("vertex"));
        assert!(runtime.supports_tool_use("gemini"));
        assert!(runtime.supports_tool_use("vertex"));
    }

    // ── provider-matrix-tests: protocol dispatch per family ───────────────────

    /// Validates that an OpenAI-compatible *gateway* provider (groq used as
    /// representative) dispatches through the OpenAI chat-completions path.
    ///
    /// This complements the `openai_runtime_builds_chat_completions_request` test
    /// which uses the first-party `openai` provider.  Both share
    /// `WireProtocol::OpenAiCompat`, but a gateway uses a different base URL and
    /// carries its own API key — explicitly exercising the gateway code-path.
    #[test]
    fn openai_compat_gateway_runtime_dispatches_via_openai_protocol() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"content": "Groq gateway reply"}
            }],
            "usage": {
                "prompt_tokens": 8,
                "completion_tokens": 4
            }
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        // Use the groq gateway as a representative OpenAI-compat gateway provider.
        let resolved = resolved_provider("groq", None);
        let request = CompletionRequest::new("Hello via gateway");

        let response = runtime
            .complete(&resolved, &request)
            .expect("groq gateway response");
        let recorded = transport.take_request();
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        // URL must target the groq-specific base, not api.openai.com.
        assert!(
            recorded.url.contains("api.groq.com"),
            "URL should target groq base: {}",
            recorded.url
        );
        assert!(
            recorded.url.ends_with("/chat/completions"),
            "URL should end with /chat/completions: {}",
            recorded.url
        );
        // Auth follows standard Bearer pattern.
        assert!(
            recorded
                .headers
                .get("authorization")
                .map(|v| v.starts_with("Bearer "))
                .unwrap_or(false),
            "gateway must send Authorization: Bearer <key>"
        );
        // Body must include the model field.
        assert!(
            body.pointer("/model")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "request body must include a model field"
        );
        assert_eq!(response.output_text, "Groq gateway reply");
    }

    /// Validates the local auth-free provider dispatches via OpenAI-compat
    /// protocol and explicitly targets the resolved Ollama-style base URL.
    #[test]
    fn local_provider_runtime_dispatches_to_openai_compat_with_custom_host() {
        let transport = RecordingTransport::with_json_body(serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "local reply"}}],
            "usage": {"prompt_tokens": 4, "completion_tokens": 2}
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);

        // Resolve local provider with an OLLAMA_HOST env override.
        let settings = AgentSettings {
            selected_provider: Some("local".into()),
            selected_model: Some("phi4:14b".into()),
            ..AgentSettings::default()
        };
        let resolved = ProviderResolver::builtin()
            .resolve_execution_with_env(
                &settings,
                &StoredCredentials::default(),
                [("OLLAMA_HOST", "http://gpu-server:11434".to_string())],
                &ProviderSelection::default(),
            )
            .expect("local provider resolves with OLLAMA_HOST");

        runtime
            .complete(&resolved, &CompletionRequest::new("Hello local"))
            .expect("local completion");
        let recorded = transport.take_request();

        assert_eq!(
            recorded.url, "http://gpu-server:11434/v1/chat/completions",
            "local must route to OLLAMA_HOST-derived base"
        );
        assert!(
            !recorded.headers.contains_key("authorization"),
            "local provider must not send an Authorization header"
        );
    }
}
