use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read},
    path::Path,
    sync::Arc,
    time::Duration,
};

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use time::OffsetDateTime;
use wonder_of_u_core::{Result, TokenUsage, WonderError};

use crate::{
    CredentialStore, ProviderResolver, ProviderSelection, ResolvedProviderExecution,
    auth::{
        copilot_device_flow_client_id, copilot_standard_headers, copilot_token_url,
        github_device_access_token_url, parse_copilot_oauth_token_response,
    },
};

const DEFAULT_ANTHROPIC_API_VERSION: &str = "2023-06-01";
const DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS: u32 = 1024;
const COPILOT_OAUTH_REFRESH_SKEW_SECONDS: i64 = 60;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionRequest {
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub effort_level: Option<String>,
}

impl CompletionRequest {
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionResponse {
    pub provider: String,
    pub model: String,
    pub output_text: String,
    pub stop_reason: Option<String>,
    pub usage: TokenUsage,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderToolCall {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderToolResultMessage {
    pub call_id: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolConversationRound {
    pub assistant_text: Option<String>,
    pub calls: Vec<ProviderToolCall>,
    pub results: Vec<ProviderToolResultMessage>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolUseRequest {
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tools: Vec<ProviderToolSpec>,
    pub rounds: Vec<ToolConversationRound>,
    pub effort_level: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallBatchResponse {
    pub assistant_text: Option<String>,
    pub calls: Vec<ProviderToolCall>,
    pub stop_reason: Option<String>,
    pub usage: TokenUsage,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolUseResponse {
    Final(CompletionResponse),
    ToolCalls(ToolCallBatchResponse),
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
struct CopilotSession {
    api_base: String,
    bearer_token: String,
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
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(60))
                .build(),
        }
    }
}

impl HttpTransport for UreqTransport {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse> {
        match send_ureq_request(&self.agent, request) {
            Ok(response) => read_http_response(response),
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                Err(WonderError::validation(format!(
                    "provider HTTP request failed with status {status}: {}",
                    provider_error_message(&body)
                )))
            }
            Err(ureq::Error::Transport(error)) => Err(WonderError::validation(format!(
                "provider request failed: {error}"
            ))),
        }
    }

    fn execute_stream(&self, request: &HttpRequest) -> Result<StreamingHttpResponse> {
        match send_ureq_request(&self.agent, request) {
            Ok(response) => Ok(StreamingHttpResponse {
                reader: Box::new(response.into_reader()),
            }),
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                Err(WonderError::validation(format!(
                    "provider HTTP request failed with status {status}: {}",
                    provider_error_message(&body)
                )))
            }
            Err(ureq::Error::Transport(error)) => Err(WonderError::validation(format!(
                "provider request failed: {error}"
            ))),
        }
    }
}

fn send_ureq_request(
    agent: &ureq::Agent,
    request: &HttpRequest,
) -> std::result::Result<ureq::Response, ureq::Error> {
    let mut transport = agent.request(request.method.as_str(), request.url.as_str());
    for (name, value) in &request.headers {
        transport = transport.set(name, value);
    }
    if request.body.is_empty() && request.method.eq_ignore_ascii_case("GET") {
        transport.call()
    } else {
        transport.send_string(&request.body)
    }
}

fn read_http_response(response: ureq::Response) -> Result<HttpResponse> {
    let status = response.status();
    let body = response.into_string().map_err(|error| {
        WonderError::validation(format!("invalid provider response body: {error}"))
    })?;
    Ok(HttpResponse { status, body })
}

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
    #[must_use]
    pub fn new() -> Self {
        Self {
            resolver: ProviderResolver::builtin(),
            transport: Arc::new(UreqTransport::default()),
        }
    }

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

    pub fn complete(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse> {
        if request.prompt.trim().is_empty() {
            return Err(WonderError::validation("prompt cannot be empty"));
        }

        match resolved.provider_id() {
            "openai" => self.complete_openai(resolved, request),
            "anthropic" => self.complete_anthropic(resolved, request),
            "copilot" => self.complete_copilot(resolved, request),
            other => Err(WonderError::validation(format!(
                "provider `{other}` runtime is not implemented yet; supported providers in this slice: openai, anthropic, copilot"
            ))),
        }
    }

    #[must_use]
    pub fn supports_streaming(&self, provider_id: &str) -> bool {
        matches!(provider_id, "openai" | "anthropic" | "copilot")
    }

    #[must_use]
    pub fn supports_tool_use(&self, provider_id: &str) -> bool {
        provider_id == "openai"
    }

    #[must_use]
    pub fn supports_tool_use_for(&self, resolved: &ResolvedProviderExecution) -> bool {
        match resolved.provider_id() {
            "openai" => true,
            "anthropic" | "copilot" => true,
            _ => false,
        }
    }

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

        match resolved.provider_id() {
            "openai" => self.complete_openai_streaming(resolved, request, &mut on_text_delta),
            "anthropic" => self.complete_anthropic_streaming(resolved, request, &mut on_text_delta),
            "copilot" => self.complete_copilot_streaming(resolved, request, &mut on_text_delta),
            other => Err(WonderError::validation(format!(
                "provider `{other}` streaming runtime is not implemented yet; supported providers in this slice: openai, anthropic, copilot"
            ))),
        }
    }

    pub fn complete_with_tool_use(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &ToolUseRequest,
    ) -> Result<ToolUseResponse> {
        if request.prompt.trim().is_empty() {
            return Err(WonderError::validation("prompt cannot be empty"));
        }

        match resolved.provider_id() {
            "openai" => self.complete_openai_with_tool_use(resolved, request),
            "anthropic" => self.complete_anthropic_with_tool_use(resolved, request),
            "copilot" => self.complete_copilot_with_tool_use(resolved, request),
            other => Err(WonderError::validation(format!(
                "provider `{other}` tool-use orchestration is not implemented yet; supported providers in this slice: openai, anthropic, copilot"
            ))),
        }
    }

    fn complete_openai(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse> {
        let http_request = build_openai_request(resolved, request)?;
        let http_response = self.transport.execute(&http_request)?;
        parse_openai_response(resolved, &http_response.body)
    }

    fn complete_openai_streaming<F>(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &CompletionRequest,
        on_text_delta: &mut F,
    ) -> Result<CompletionResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        let http_request = build_openai_stream_request(resolved, request)?;
        let http_response = self.transport.execute_stream(&http_request)?;
        parse_openai_stream_response(resolved, http_response.reader, on_text_delta)
    }

    fn complete_openai_with_tool_use(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &ToolUseRequest,
    ) -> Result<ToolUseResponse> {
        let http_request = build_openai_tool_use_request(resolved, request)?;
        let http_response = self.transport.execute(&http_request)?;
        parse_openai_tool_use_response(resolved, &http_response.body)
    }

    fn complete_anthropic(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse> {
        let http_request = build_anthropic_request(resolved, request)?;
        let http_response = self.transport.execute(&http_request)?;
        parse_anthropic_response(resolved, &http_response.body)
    }

    fn complete_anthropic_streaming<F>(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &CompletionRequest,
        on_text_delta: &mut F,
    ) -> Result<CompletionResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        let http_request = build_anthropic_stream_request(resolved, request)?;
        let http_response = self.transport.execute_stream(&http_request)?;
        parse_anthropic_stream_response(resolved, http_response.reader, on_text_delta)
    }

    fn complete_anthropic_with_tool_use(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &ToolUseRequest,
    ) -> Result<ToolUseResponse> {
        let http_request = build_anthropic_tool_use_request(resolved, request)?;
        let http_response = self.transport.execute(&http_request)?;
        parse_anthropic_tool_use_response(resolved, &http_response.body)
    }

    fn complete_copilot(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse> {
        let session = self.exchange_copilot_session(resolved)?;
        if is_anthropic_model(resolved.model()) {
            let http_request = build_copilot_anthropic_request(
                resolved,
                request,
                false,
                &session.api_base,
                &session.bearer_token,
            )?;
            let http_response = self.transport.execute(&http_request)?;
            parse_anthropic_response(resolved, &http_response.body)
        } else {
            let http_request = build_copilot_openai_request(
                resolved,
                request,
                false,
                &session.api_base,
                &session.bearer_token,
            )?;
            let http_response = self.transport.execute(&http_request)?;
            parse_openai_response(resolved, &http_response.body)
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
        if is_anthropic_model(resolved.model()) {
            let http_request = build_copilot_anthropic_request(
                resolved,
                request,
                true,
                &session.api_base,
                &session.bearer_token,
            )?;
            let http_response = self.transport.execute_stream(&http_request)?;
            parse_anthropic_stream_response(resolved, http_response.reader, on_text_delta)
        } else {
            let http_request = build_copilot_openai_request(
                resolved,
                request,
                true,
                &session.api_base,
                &session.bearer_token,
            )?;
            let http_response = self.transport.execute_stream(&http_request)?;
            parse_openai_stream_response(resolved, http_response.reader, on_text_delta)
        }
    }

    fn complete_copilot_with_tool_use(
        &self,
        resolved: &ResolvedProviderExecution,
        request: &ToolUseRequest,
    ) -> Result<ToolUseResponse> {
        let session = self.exchange_copilot_session(resolved)?;
        let http_request = if is_anthropic_model(resolved.model()) {
            build_copilot_anthropic_tool_use_request(
                resolved,
                request,
                &session.api_base,
                &session.bearer_token,
            )?
        } else {
            build_copilot_openai_tool_use_request(
                resolved,
                request,
                &session.api_base,
                &session.bearer_token,
            )?
        };
        let http_response = self.transport.execute(&http_request)?;
        if is_anthropic_model(resolved.model()) {
            parse_anthropic_tool_use_response(resolved, &http_response.body)
        } else {
            parse_openai_tool_use_response(resolved, &http_response.body)
        }
    }

    fn exchange_copilot_session(
        &self,
        resolved: &ResolvedProviderExecution,
    ) -> Result<CopilotSession> {
        let http_request = build_copilot_token_exchange_request(resolved)?;
        let http_response = self.transport.execute(&http_request)?;
        parse_copilot_token_exchange_response(resolved, &http_response.body)
    }

    fn refresh_copilot_oauth_if_needed(
        &self,
        storage_dir: Option<&Path>,
        selection: &ProviderSelection,
        resolved: ResolvedProviderExecution,
    ) -> Result<ResolvedProviderExecution> {
        if resolved.provider_id() != "copilot" || !copilot_oauth_should_refresh(&resolved) {
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
        let http_request = build_copilot_oauth_refresh_request(refresh_token);
        let http_response = self.transport.execute(&http_request)?;
        let token = parse_copilot_oauth_refresh_response(&http_response.body)?;
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

fn build_openai_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_openai_request_with_mode(resolved, request, false)
}

fn build_openai_stream_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_openai_request_with_mode(resolved, request, true)
}

fn build_openai_request_with_mode(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
    stream: bool,
) -> Result<HttpRequest> {
    let mut messages = Vec::new();
    if let Some(system_prompt) = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        messages.push(json!({
            "role": "system",
            "content": system_prompt,
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": request.prompt,
    }));

    let mut body = json!({
        "model": resolved.model(),
        "messages": messages,
    });
    let body_map = body
        .as_object_mut()
        .expect("openai request body should be an object");
    if stream {
        body_map.insert("stream".into(), Value::Bool(true));
        body_map.insert(
            "stream_options".into(),
            json!({
                "include_usage": true,
            }),
        );
    }
    if let Some(temperature) = request.temperature {
        body_map.insert("temperature".into(), json!(temperature));
    }
    if let Some(max_output_tokens) = request.max_output_tokens {
        body_map.insert("max_completion_tokens".into(), json!(max_output_tokens));
    }
    // wire reasoning effort for openai
    if is_high_effort(request.effort_level.as_deref()) {
        body_map.insert("reasoning_effort".into(), json!("high"));
    }

    Ok(HttpRequest {
        method: "POST".into(),
        url: join_url(resolved.api_base(), "/chat/completions"),
        headers: BTreeMap::from([
            ("accept".into(), "application/json".into()),
            (
                "authorization".into(),
                format!("Bearer {}", resolved.api_key()?),
            ),
            ("content-type".into(), "application/json".into()),
        ]),
        body: serde_json::to_string(&body)?,
    })
}

fn build_openai_tool_use_request(
    resolved: &ResolvedProviderExecution,
    request: &ToolUseRequest,
) -> Result<HttpRequest> {
    let mut messages = Vec::new();
    if let Some(system_prompt) = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        messages.push(json!({
            "role": "system",
            "content": system_prompt,
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": request.prompt,
    }));
    for round in &request.rounds {
        let tool_calls = round
            .calls
            .iter()
            .map(|call| {
                Ok(json!({
                    "id": call.call_id,
                    "type": "function",
                    "function": {
                        "name": call.tool_name,
                        "arguments": serde_json::to_string(&call.arguments)?,
                    },
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        messages.push(json!({
            "role": "assistant",
            "content": round.assistant_text,
            "tool_calls": tool_calls,
        }));
        for result in &round.results {
            messages.push(json!({
                "role": "tool",
                "tool_call_id": result.call_id,
                "content": result.content,
            }));
        }
    }

    let tools = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                },
            })
        })
        .collect::<Vec<_>>();

    let mut body = json!({
        "model": resolved.model(),
        "messages": messages,
        "tool_choice": "auto",
        "parallel_tool_calls": true,
        "tools": tools,
    });
    let body_map = body
        .as_object_mut()
        .expect("openai request body should be an object");
    if let Some(temperature) = request.temperature {
        body_map.insert("temperature".into(), json!(temperature));
    }
    if let Some(max_output_tokens) = request.max_output_tokens {
        body_map.insert("max_completion_tokens".into(), json!(max_output_tokens));
    }
    // wire reasoning effort for openai
    if is_high_effort(request.effort_level.as_deref()) {
        body_map.insert("reasoning_effort".into(), json!("high"));
    }

    Ok(HttpRequest {
        method: "POST".into(),
        url: join_url(resolved.api_base(), "/chat/completions"),
        headers: BTreeMap::from([
            ("accept".into(), "application/json".into()),
            (
                "authorization".into(),
                format!("Bearer {}", resolved.api_key()?),
            ),
            ("content-type".into(), "application/json".into()),
        ]),
        body: serde_json::to_string(&body)?,
    })
}

fn build_anthropic_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_anthropic_request_with_mode(resolved, request, false)
}

fn build_anthropic_stream_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_anthropic_request_with_mode(resolved, request, true)
}

fn build_anthropic_request_with_mode(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
    stream: bool,
) -> Result<HttpRequest> {
    let mut body = json!({
        "model": resolved.model(),
        "max_tokens": request
            .max_output_tokens
            .unwrap_or(DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS),
        "messages": [{
            "role": "user",
            "content": request.prompt,
        }],
    });
    let body_map = body
        .as_object_mut()
        .expect("anthropic request body should be an object");
    if stream {
        body_map.insert("stream".into(), Value::Bool(true));
    }
    if let Some(system_prompt) = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body_map.insert("system".into(), json!(system_prompt));
    }
    if let Some(temperature) = request.temperature {
        body_map.insert("temperature".into(), json!(temperature));
    }
    // wire reasoning effort for anthropic
    if is_high_effort(request.effort_level.as_deref()) && is_anthropic_model(resolved.model()) {
        body_map.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": 10000}),
        );
    }

    Ok(HttpRequest {
        method: "POST".into(),
        url: join_url(resolved.api_base(), "/v1/messages"),
        headers: BTreeMap::from([
            ("accept".into(), "application/json".into()),
            (
                "anthropic-version".into(),
                DEFAULT_ANTHROPIC_API_VERSION.into(),
            ),
            ("content-type".into(), "application/json".into()),
            ("x-api-key".into(), resolved.api_key()?.to_string()),
        ]),
        body: serde_json::to_string(&body)?,
    })
}

fn build_anthropic_tool_use_request(
    resolved: &ResolvedProviderExecution,
    request: &ToolUseRequest,
) -> Result<HttpRequest> {
    build_anthropic_tool_use_request_with_headers(
        resolved.model(),
        resolved.api_base(),
        BTreeMap::from([
            ("accept".into(), "application/json".into()),
            (
                "anthropic-version".into(),
                DEFAULT_ANTHROPIC_API_VERSION.into(),
            ),
            ("content-type".into(), "application/json".into()),
            ("x-api-key".into(), resolved.api_key()?.to_string()),
        ]),
        request,
    )
}

fn build_anthropic_tool_use_request_with_headers(
    model: &str,
    api_base: &str,
    headers: BTreeMap<String, String>,
    request: &ToolUseRequest,
) -> Result<HttpRequest> {
    let mut messages = vec![json!({
        "role": "user",
        "content": [{
            "type": "text",
            "text": request.prompt.as_str(),
        }],
    })];
    for round in &request.rounds {
        let mut assistant_content = Vec::new();
        if let Some(text) = round
            .assistant_text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            assistant_content.push(json!({
                "type": "text",
                "text": text,
            }));
        }
        assistant_content.extend(round.calls.iter().map(|call| {
            json!({
                "type": "tool_use",
                "id": call.call_id.as_str(),
                "name": call.tool_name.as_str(),
                "input": call.arguments.clone(),
            })
        }));
        messages.push(json!({
            "role": "assistant",
            "content": assistant_content,
        }));
        messages.push(json!({
            "role": "user",
            "content": round.results.iter().map(|result| json!({
                "type": "tool_result",
                "tool_use_id": result.call_id.as_str(),
                "content": result.content.as_str(),
            })).collect::<Vec<_>>(),
        }));
    }

    let mut body = json!({
        "model": model,
        "max_tokens": request
            .max_output_tokens
            .unwrap_or(DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS),
        "messages": messages,
    });
    let body_map = body
        .as_object_mut()
        .expect("anthropic tool-use request body should be an object");
    if !request.tools.is_empty() {
        body_map.insert(
            "tools".into(),
            Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name.as_str(),
                            "description": tool.description.as_str(),
                            "input_schema": tool.input_schema.clone(),
                        })
                    })
                    .collect(),
            ),
        );
        body_map.insert("tool_choice".into(), json!({ "type": "auto" }));
    }
    if let Some(system_prompt) = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body_map.insert("system".into(), json!(system_prompt));
    }
    if let Some(temperature) = request.temperature {
        body_map.insert("temperature".into(), json!(temperature));
    }
    // wire reasoning effort for anthropic
    if is_high_effort(request.effort_level.as_deref()) && is_anthropic_model(model) {
        body_map.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": 10000}),
        );
    }

    Ok(HttpRequest {
        method: "POST".into(),
        url: join_url(api_base, "/v1/messages"),
        headers,
        body: serde_json::to_string(&body)?,
    })
}

fn build_copilot_token_exchange_request(
    resolved: &ResolvedProviderExecution,
) -> Result<HttpRequest> {
    let mut headers = copilot_standard_headers();
    headers.insert("accept".into(), "application/json".into());
    headers.insert(
        "authorization".into(),
        format!("Bearer {}", resolved.oauth_access_token()?),
    );
    Ok(HttpRequest {
        method: "GET".into(),
        url: copilot_token_url(),
        headers,
        body: String::new(),
    })
}

fn build_copilot_oauth_refresh_request(refresh_token: &str) -> HttpRequest {
    let body = [
        (
            "client_id",
            utf8_percent_encode(copilot_device_flow_client_id().as_str(), NON_ALPHANUMERIC)
                .to_string(),
        ),
        ("grant_type", "refresh_token".to_string()),
        (
            "refresh_token",
            utf8_percent_encode(refresh_token, NON_ALPHANUMERIC).to_string(),
        ),
    ]
    .into_iter()
    .map(|(key, value)| format!("{key}={value}"))
    .collect::<Vec<_>>()
    .join("&");
    HttpRequest {
        method: "POST".into(),
        url: github_device_access_token_url(),
        headers: BTreeMap::from([
            ("accept".into(), "application/json".into()),
            (
                "content-type".into(),
                "application/x-www-form-urlencoded".into(),
            ),
        ]),
        body,
    }
}

fn parse_copilot_token_exchange_response(
    resolved: &ResolvedProviderExecution,
    body: &str,
) -> Result<CopilotSession> {
    let json: Value = serde_json::from_str(body)?;
    let bearer_token = json
        .get("token")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WonderError::validation("Copilot token exchange response missing token"))?;
    let api_base = json
        .pointer("/endpoints/api")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| resolved.api_base());
    Ok(CopilotSession {
        api_base: api_base.to_string(),
        bearer_token: bearer_token.to_string(),
    })
}

fn parse_copilot_oauth_refresh_response(body: &str) -> Result<crate::CopilotOAuthToken> {
    let json: Value = serde_json::from_str(body)?;
    match json
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "" => parse_copilot_oauth_token_response(&json),
        other => Err(WonderError::validation(format!(
            "GitHub oauth refresh failed: {other}"
        ))),
    }
}

fn build_copilot_openai_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
    stream: bool,
    api_base: &str,
    bearer_token: &str,
) -> Result<HttpRequest> {
    let mut messages = Vec::new();
    if let Some(system_prompt) = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        messages.push(json!({
            "role": "system",
            "content": system_prompt,
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": request.prompt,
    }));

    let mut body = json!({
        "model": resolved.model(),
        "messages": messages,
    });
    let body_map = body
        .as_object_mut()
        .expect("copilot openai request body should be an object");
    if stream {
        body_map.insert("stream".into(), Value::Bool(true));
        body_map.insert(
            "stream_options".into(),
            json!({
                "include_usage": true,
            }),
        );
    }
    if let Some(temperature) = request.temperature {
        body_map.insert("temperature".into(), json!(temperature));
    }
    if let Some(max_output_tokens) = request.max_output_tokens {
        body_map.insert("max_completion_tokens".into(), json!(max_output_tokens));
    }

    let mut headers = copilot_standard_headers();
    headers.insert("accept".into(), "application/json".into());
    headers.insert("content-type".into(), "application/json".into());
    headers.insert("authorization".into(), format!("Bearer {bearer_token}"));
    // wire reasoning effort for copilot
    if is_high_effort(request.effort_level.as_deref()) {
        headers.insert("x-reasoning-effort".into(), "high".into());
    }
    Ok(HttpRequest {
        method: "POST".into(),
        url: join_url(api_base, "/chat/completions"),
        headers,
        body: serde_json::to_string(&body)?,
    })
}

fn build_copilot_openai_tool_use_request(
    resolved: &ResolvedProviderExecution,
    request: &ToolUseRequest,
    api_base: &str,
    bearer_token: &str,
) -> Result<HttpRequest> {
    let mut messages = Vec::new();
    if let Some(system_prompt) = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        messages.push(json!({
            "role": "system",
            "content": system_prompt,
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": request.prompt,
    }));
    for round in &request.rounds {
        let tool_calls = round
            .calls
            .iter()
            .map(|call| {
                Ok(json!({
                    "id": call.call_id,
                    "type": "function",
                    "function": {
                        "name": call.tool_name,
                        "arguments": serde_json::to_string(&call.arguments)?,
                    },
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        messages.push(json!({
            "role": "assistant",
            "content": round.assistant_text,
            "tool_calls": tool_calls,
        }));
        for result in &round.results {
            messages.push(json!({
                "role": "tool",
                "tool_call_id": result.call_id,
                "content": result.content,
            }));
        }
    }

    let tools = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                },
            })
        })
        .collect::<Vec<_>>();

    let mut body = json!({
        "model": resolved.model(),
        "messages": messages,
        "tool_choice": "auto",
        "parallel_tool_calls": true,
        "tools": tools,
    });
    let body_map = body
        .as_object_mut()
        .expect("copilot openai tool-use request body should be an object");
    if let Some(temperature) = request.temperature {
        body_map.insert("temperature".into(), json!(temperature));
    }
    if let Some(max_output_tokens) = request.max_output_tokens {
        body_map.insert("max_completion_tokens".into(), json!(max_output_tokens));
    }

    let mut headers = copilot_standard_headers();
    headers.insert("accept".into(), "application/json".into());
    headers.insert("content-type".into(), "application/json".into());
    headers.insert("authorization".into(), format!("Bearer {bearer_token}"));
    // wire reasoning effort for copilot
    if is_high_effort(request.effort_level.as_deref()) {
        headers.insert("x-reasoning-effort".into(), "high".into());
    }
    Ok(HttpRequest {
        method: "POST".into(),
        url: join_url(api_base, "/chat/completions"),
        headers,
        body: serde_json::to_string(&body)?,
    })
}

fn build_copilot_anthropic_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
    stream: bool,
    api_base: &str,
    bearer_token: &str,
) -> Result<HttpRequest> {
    let mut body = json!({
        "model": resolved.model(),
        "max_tokens": request
            .max_output_tokens
            .unwrap_or(DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS),
        "messages": [{
            "role": "user",
            "content": request.prompt,
        }],
    });
    let body_map = body
        .as_object_mut()
        .expect("copilot anthropic request body should be an object");
    if stream {
        body_map.insert("stream".into(), Value::Bool(true));
    }
    if let Some(system_prompt) = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body_map.insert("system".into(), json!(system_prompt));
    }
    if let Some(temperature) = request.temperature {
        body_map.insert("temperature".into(), json!(temperature));
    }
    // wire reasoning effort for anthropic
    if is_high_effort(request.effort_level.as_deref()) && is_anthropic_model(resolved.model()) {
        body_map.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": 10000}),
        );
    }

    let mut headers = copilot_standard_headers();
    headers.insert("accept".into(), "application/json".into());
    headers.insert(
        "anthropic-version".into(),
        DEFAULT_ANTHROPIC_API_VERSION.into(),
    );
    headers.insert("authorization".into(), format!("Bearer {bearer_token}"));
    headers.insert("content-type".into(), "application/json".into());
    Ok(HttpRequest {
        method: "POST".into(),
        url: join_url(api_base, "/v1/messages"),
        headers,
        body: serde_json::to_string(&body)?,
    })
}

fn build_copilot_anthropic_tool_use_request(
    resolved: &ResolvedProviderExecution,
    request: &ToolUseRequest,
    api_base: &str,
    bearer_token: &str,
) -> Result<HttpRequest> {
    let mut headers = copilot_standard_headers();
    headers.insert("accept".into(), "application/json".into());
    headers.insert(
        "anthropic-version".into(),
        DEFAULT_ANTHROPIC_API_VERSION.into(),
    );
    headers.insert("authorization".into(), format!("Bearer {bearer_token}"));
    headers.insert("content-type".into(), "application/json".into());
    build_anthropic_tool_use_request_with_headers(resolved.model(), api_base, headers, request)
}

fn parse_openai_response(
    resolved: &ResolvedProviderExecution,
    body: &str,
) -> Result<CompletionResponse> {
    let json: Value = serde_json::from_str(body)?;
    let choice = json
        .pointer("/choices/0")
        .ok_or_else(|| WonderError::validation("OpenAI response missing choices[0]"))?;
    let output_text = extract_openai_text(choice.pointer("/message/content")).ok_or_else(|| {
        WonderError::validation("OpenAI response missing assistant message content")
    })?;
    let usage = parse_openai_usage(json.get("usage"));

    Ok(CompletionResponse {
        provider: resolved.provider_id().to_string(),
        model: resolved.model().to_string(),
        output_text,
        stop_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        usage,
    })
}

fn parse_openai_tool_use_response(
    resolved: &ResolvedProviderExecution,
    body: &str,
) -> Result<ToolUseResponse> {
    let json: Value = serde_json::from_str(body)?;
    let choice = json
        .pointer("/choices/0")
        .ok_or_else(|| WonderError::validation("OpenAI response missing choices[0]"))?;
    let assistant_text = extract_openai_text(choice.pointer("/message/content"))
        .filter(|text| !text.trim().is_empty());
    let usage = parse_openai_usage(json.get("usage"));
    let stop_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let tool_calls = choice
        .pointer("/message/tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .map(parse_openai_tool_call)
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();

    if !tool_calls.is_empty() {
        return Ok(ToolUseResponse::ToolCalls(ToolCallBatchResponse {
            assistant_text,
            calls: tool_calls,
            stop_reason,
            usage,
        }));
    }

    let output_text = assistant_text.ok_or_else(|| {
        WonderError::validation("OpenAI response missing assistant message content")
    })?;
    Ok(ToolUseResponse::Final(CompletionResponse {
        provider: resolved.provider_id().to_string(),
        model: resolved.model().to_string(),
        output_text,
        stop_reason,
        usage,
    }))
}

fn parse_anthropic_response(
    resolved: &ResolvedProviderExecution,
    body: &str,
) -> Result<CompletionResponse> {
    let json: Value = serde_json::from_str(body)?;
    let content = json
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| WonderError::validation("Anthropic response missing content array"))?;
    let output_text = extract_anthropic_text(content);
    if output_text.trim().is_empty() {
        return Err(WonderError::validation(
            "Anthropic response did not contain any text content",
        ));
    }

    Ok(CompletionResponse {
        provider: resolved.provider_id().to_string(),
        model: resolved.model().to_string(),
        output_text,
        stop_reason: json
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        usage: parse_anthropic_usage(json.get("usage")),
    })
}

fn parse_anthropic_tool_use_response(
    resolved: &ResolvedProviderExecution,
    body: &str,
) -> Result<ToolUseResponse> {
    let json: Value = serde_json::from_str(body)?;
    let content = json
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| WonderError::validation("Anthropic response missing content array"))?;
    let assistant_text = {
        let text = extract_anthropic_text(content);
        (!text.trim().is_empty()).then_some(text)
    };
    let tool_calls = content
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("tool_use"))
        .map(parse_anthropic_tool_call)
        .collect::<Result<Vec<_>>>()?;
    let stop_reason = json
        .get("stop_reason")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let usage = parse_anthropic_usage(json.get("usage"));

    if !tool_calls.is_empty() {
        return Ok(ToolUseResponse::ToolCalls(ToolCallBatchResponse {
            assistant_text,
            calls: tool_calls,
            stop_reason,
            usage,
        }));
    }

    let output_text = assistant_text.ok_or_else(|| {
        WonderError::validation("Anthropic response did not contain any text content")
    })?;
    Ok(ToolUseResponse::Final(CompletionResponse {
        provider: resolved.provider_id().to_string(),
        model: resolved.model().to_string(),
        output_text,
        stop_reason,
        usage,
    }))
}

fn parse_openai_stream_response<F>(
    resolved: &ResolvedProviderExecution,
    reader: Box<dyn Read + Send>,
    on_text_delta: &mut F,
) -> Result<CompletionResponse>
where
    F: FnMut(&str) -> Result<()>,
{
    let mut output_text = String::new();
    let mut stop_reason = None;
    let mut usage = TokenUsage::default();
    consume_sse(BufReader::new(reader), |_, data| {
        if data == "[DONE]" {
            return Ok(false);
        }
        let json: Value = serde_json::from_str(data)?;
        if let Some(text) = extract_openai_text(json.pointer("/choices/0/delta/content"))
            .filter(|text| !text.is_empty())
        {
            output_text.push_str(&text);
            on_text_delta(&text)?;
        }
        if let Some(choice) = json.pointer("/choices/0") {
            if stop_reason.is_none() {
                stop_reason = choice
                    .get("finish_reason")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
            }
        }
        if let Some(chunk_usage) = json.get("usage") {
            usage = parse_openai_usage(Some(chunk_usage));
        }
        Ok(true)
    })?;

    if output_text.trim().is_empty() {
        return Err(WonderError::validation(
            "OpenAI streaming response did not contain any text content",
        ));
    }

    Ok(CompletionResponse {
        provider: resolved.provider_id().to_string(),
        model: resolved.model().to_string(),
        output_text,
        stop_reason,
        usage,
    })
}

fn parse_anthropic_stream_response<F>(
    resolved: &ResolvedProviderExecution,
    reader: Box<dyn Read + Send>,
    on_text_delta: &mut F,
) -> Result<CompletionResponse>
where
    F: FnMut(&str) -> Result<()>,
{
    let mut output_text = String::new();
    let mut stop_reason = None;
    let mut usage = TokenUsage::default();
    consume_sse(BufReader::new(reader), |event, data| {
        let json: Value = serde_json::from_str(data)?;
        let event_type = json
            .get("type")
            .and_then(Value::as_str)
            .or(event)
            .unwrap_or_default();
        match event_type {
            "ping" | "content_block_start" | "message_stop" => {}
            "error" => {
                return Err(WonderError::validation(format!(
                    "Anthropic streaming request failed: {}",
                    provider_error_message(data)
                )));
            }
            "message_start" => {
                merge_usage(
                    &mut usage,
                    parse_anthropic_usage(json.pointer("/message/usage")),
                );
            }
            "content_block_delta" => {
                if let Some(text) = json
                    .pointer("/delta/text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                {
                    output_text.push_str(text);
                    on_text_delta(text)?;
                }
            }
            "message_delta" => {
                if stop_reason.is_none() {
                    stop_reason = json
                        .pointer("/delta/stop_reason")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                }
                merge_usage(&mut usage, parse_anthropic_usage(json.get("usage")));
            }
            _ => {}
        }
        Ok(true)
    })?;

    if output_text.trim().is_empty() {
        return Err(WonderError::validation(
            "Anthropic streaming response did not contain any text content",
        ));
    }

    Ok(CompletionResponse {
        provider: resolved.provider_id().to_string(),
        model: resolved.model().to_string(),
        output_text,
        stop_reason,
        usage,
    })
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

fn extract_openai_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let joined = parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .or_else(|| {
                            part.get("content")
                                .and_then(Value::as_str)
                                .map(ToString::to_string)
                        })
                })
                .collect::<Vec<_>>()
                .join("");
            (!joined.is_empty()).then_some(joined)
        }
        _ => None,
    }
}

fn parse_openai_tool_call(value: &Value) -> Result<ProviderToolCall> {
    let call_id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WonderError::validation("OpenAI tool call missing id"))?;
    let tool_name = value
        .pointer("/function/name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WonderError::validation("OpenAI tool call missing function name"))?;
    let raw_arguments = value
        .pointer("/function/arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let arguments = if raw_arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(raw_arguments).map_err(|error| {
            WonderError::validation(format!(
                "OpenAI tool call `{tool_name}` returned invalid JSON arguments: {error}"
            ))
        })?
    };

    Ok(ProviderToolCall {
        call_id: call_id.to_string(),
        tool_name: tool_name.to_string(),
        arguments,
    })
}

fn parse_anthropic_tool_call(value: &Value) -> Result<ProviderToolCall> {
    let call_id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WonderError::validation("Anthropic tool call missing id"))?;
    let tool_name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WonderError::validation("Anthropic tool call missing name"))?;
    let arguments = value.get("input").cloned().unwrap_or_else(|| json!({}));

    Ok(ProviderToolCall {
        call_id: call_id.to_string(),
        tool_name: tool_name.to_string(),
        arguments,
    })
}

fn extract_anthropic_text(content: &[Value]) -> String {
    content
        .iter()
        .filter(|part| {
            part.get("type")
                .and_then(Value::as_str)
                .is_none_or(|kind| kind == "text")
        })
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn parse_openai_usage(usage: Option<&Value>) -> TokenUsage {
    TokenUsage {
        input_tokens: usage
            .and_then(|usage| usage.get("prompt_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        output_tokens: usage
            .and_then(|usage| usage.get("completion_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        cache_creation_tokens: 0,
        cache_read_tokens: usage
            .and_then(|usage| usage.pointer("/prompt_tokens_details/cached_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    }
}

fn parse_anthropic_usage(usage: Option<&Value>) -> TokenUsage {
    TokenUsage {
        input_tokens: usage
            .and_then(|usage| usage.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        output_tokens: usage
            .and_then(|usage| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        cache_creation_tokens: usage
            .and_then(|usage| usage.get("cache_creation_input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        cache_read_tokens: usage
            .and_then(|usage| usage.get("cache_read_input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    }
}

fn merge_usage(current: &mut TokenUsage, next: TokenUsage) {
    if next.input_tokens > 0 {
        current.input_tokens = next.input_tokens;
    }
    if next.output_tokens > 0 {
        current.output_tokens = next.output_tokens;
    }
    if next.cache_creation_tokens > 0 {
        current.cache_creation_tokens = next.cache_creation_tokens;
    }
    if next.cache_read_tokens > 0 {
        current.cache_read_tokens = next.cache_read_tokens;
    }
}

fn is_anthropic_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("claude")
}

fn is_high_effort(level: Option<&str>) -> bool {
    matches!(level, Some("high") | Some("max"))
}

fn copilot_oauth_should_refresh(resolved: &ResolvedProviderExecution) -> bool {
    resolved.oauth_expires_at().is_some_and(|expires_at| {
        expires_at
            <= OffsetDateTime::now_utc()
                + time::Duration::seconds(COPILOT_OAUTH_REFRESH_SKEW_SECONDS)
    })
}

fn join_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Mutex};

    use wonder_of_u_test_support::unique_test_dir;

    use crate::{AgentSettings, AuthMaterial, CredentialStore, SettingsStore, StoredCredentials};

    use super::*;

    #[derive(Default)]
    struct RecordingTransport {
        requests: Mutex<Vec<HttpRequest>>,
        responses: Mutex<Vec<HttpResponse>>,
        stream_bodies: Mutex<Vec<String>>,
        force_error: Mutex<Option<String>>,
    }

    impl RecordingTransport {
        fn with_json_body(body: Value) -> Arc<Self> {
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

        fn with_json_responses(bodies: Vec<Value>) -> Arc<Self> {
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
            json_bodies: Vec<Value>,
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

        fn with_http_error(status: u16, body: Value) -> Arc<Self> {
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
        let transport = RecordingTransport::with_json_responses(vec![json!({
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
        let transport = RecordingTransport::with_json_body(json!({
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
        let body: Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(
            recorded.headers.get("authorization").map(String::as_str),
            Some("Bearer secret-key")
        );
        assert_eq!(
            body.pointer("/model").and_then(Value::as_str),
            Some("gpt-4.1")
        );
        assert_eq!(
            body.pointer("/messages/0/role").and_then(Value::as_str),
            Some("system")
        );
        assert_eq!(
            body.pointer("/messages/0/content").and_then(Value::as_str),
            Some("Be concise")
        );
        assert_eq!(
            body.pointer("/messages/1/content").and_then(Value::as_str),
            Some("Say hi")
        );
        assert_eq!(
            body.pointer("/max_completion_tokens")
                .and_then(Value::as_u64),
            Some(64)
        );
        let temperature = body
            .pointer("/temperature")
            .and_then(Value::as_f64)
            .expect("temperature");
        assert!((temperature - 0.2).abs() < 1e-6);
        assert_eq!(response.output_text, "Hello back");
        assert_eq!(response.stop_reason.as_deref(), Some("stop"));
        assert_eq!(response.usage.input_tokens, 11);
        assert_eq!(response.usage.output_tokens, 7);
        assert_eq!(response.usage.cache_read_tokens, 2);
    }

    #[test]
    fn openai_tool_use_runtime_builds_tools_request_with_prior_rounds() {
        let transport = RecordingTransport::with_json_body(json!({
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
                        input_schema: json!({
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
                            arguments: json!({"path": "src/main.rs"}),
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
        let body: Value = serde_json::from_str(&recorded.body).expect("request json");

        assert!(matches!(response, ToolUseResponse::Final(_)));
        assert_eq!(
            body.pointer("/tool_choice").and_then(Value::as_str),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/parallel_tool_calls")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            body.pointer("/tools/0/function/name")
                .and_then(Value::as_str),
            Some("file_read")
        );
        assert_eq!(
            body.pointer("/messages/0/role").and_then(Value::as_str),
            Some("system")
        );
        assert_eq!(
            body.pointer("/messages/1/content").and_then(Value::as_str),
            Some("Summarize the file")
        );
        assert_eq!(
            body.pointer("/messages/2/tool_calls/0/id")
                .and_then(Value::as_str),
            Some("call_123")
        );
        assert_eq!(
            body.pointer("/messages/2/tool_calls/0/function/arguments")
                .and_then(Value::as_str),
            Some("{\"path\":\"src/main.rs\"}")
        );
        assert_eq!(
            body.pointer("/messages/3/role").and_then(Value::as_str),
            Some("tool")
        );
        assert_eq!(
            body.pointer("/messages/3/tool_call_id")
                .and_then(Value::as_str),
            Some("call_123")
        );
    }

    #[test]
    fn openai_tool_use_runtime_parses_structured_tool_calls() {
        let transport = RecordingTransport::with_json_body(json!({
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
                assert_eq!(batch.calls[0].arguments, json!({"pattern": "src/**/*.rs"}));
                assert_eq!(batch.stop_reason.as_deref(), Some("tool_calls"));
                assert_eq!(batch.usage.input_tokens, 18);
                assert_eq!(batch.usage.output_tokens, 3);
            }
            other => panic!("expected tool call response, got {other:?}"),
        }
    }

    #[test]
    fn anthropic_runtime_builds_messages_request() {
        let transport = RecordingTransport::with_json_body(json!({
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
        let resolved = resolved_provider("anthropic", Some("claude-3-7-sonnet-latest"));
        let request = CompletionRequest::new("Explain the slice");

        let response = runtime
            .complete(&resolved, &request)
            .expect("anthropic response");
        let recorded = transport.take_request();
        let body: Value = serde_json::from_str(&recorded.body).expect("request json");

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
            body.pointer("/model").and_then(Value::as_str),
            Some("claude-3-7-sonnet-latest")
        );
        assert_eq!(
            body.pointer("/messages/0/content").and_then(Value::as_str),
            Some("Explain the slice")
        );
        assert_eq!(
            body.pointer("/max_tokens").and_then(Value::as_u64),
            Some(u64::from(DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS))
        );
        assert_eq!(response.output_text, "Part one. Part two.");
        assert_eq!(response.stop_reason.as_deref(), Some("end_turn"));
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
        let body: Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(body.pointer("/stream").and_then(Value::as_bool), Some(true));
        assert_eq!(
            body.pointer("/stream_options/include_usage")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(streamed, "Hello back");
        assert_eq!(response.output_text, "Hello back");
        assert_eq!(response.stop_reason.as_deref(), Some("stop"));
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
        let resolved = resolved_provider("anthropic", Some("claude-3-7-sonnet-latest"));
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
        let body: Value = serde_json::from_str(&recorded.body).expect("request json");

        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(body.pointer("/stream").and_then(Value::as_bool), Some(true));
        assert_eq!(streamed, "Part one. Part two.");
        assert_eq!(response.output_text, "Part one. Part two.");
        assert_eq!(response.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(response.usage.input_tokens, 13);
        assert_eq!(response.usage.output_tokens, 5);
        assert_eq!(response.usage.cache_creation_tokens, 4);
        assert_eq!(response.usage.cache_read_tokens, 1);
    }

    #[test]
    fn copilot_runtime_exchanges_oauth_token_and_builds_openai_request() {
        let transport = RecordingTransport::with_json_responses(vec![
            json!({
                "token": "copilot-bearer",
                "expires_at": 1_750_000_000,
                "refresh_in": 900,
                "endpoints": {"api": "https://api.githubcopilot.com"}
            }),
            json!({
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
        let completion_body: Value =
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
            completion_body.pointer("/model").and_then(Value::as_str),
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
        assert!(
            runtime.supports_tool_use_for(&resolved_oauth_provider(
                "copilot",
                Some("claude-sonnet-4")
            ))
        );
    }

    #[test]
    fn anthropic_runtime_builds_tool_use_request_and_parses_tool_calls() {
        let transport = RecordingTransport::with_json_responses(vec![json!({
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
                        input_schema: json!({
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
                            arguments: json!({"pattern": "src/**/*.rs"}),
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
        let body: Value = serde_json::from_str(&completion.body).expect("tool-use request json");

        assert_eq!(completion.method, "POST");
        assert_eq!(completion.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(
            completion.headers.get("x-api-key").map(String::as_str),
            Some("secret-key")
        );
        assert_eq!(
            body.pointer("/tool_choice/type").and_then(Value::as_str),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/messages/0/content/0/text")
                .and_then(Value::as_str),
            Some("Inspect the workspace")
        );
        assert_eq!(
            body.pointer("/messages/1/content/1/type")
                .and_then(Value::as_str),
            Some("tool_use")
        );
        assert_eq!(
            body.pointer("/messages/2/content/0/tool_use_id")
                .and_then(Value::as_str),
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
                assert_eq!(batch.calls[0].arguments, json!({"pattern": "Cargo.toml"}));
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
            json!({
                "token": "copilot-bearer",
                "expires_at": 1_750_000_000,
                "refresh_in": 900,
                "endpoints": {"api": "https://api.githubcopilot.com"}
            }),
            json!({
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
                        input_schema: json!({
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
        let body: Value = serde_json::from_str(&completion.body).expect("tool-use request json");

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
            body.pointer("/tool_choice").and_then(Value::as_str),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/tools/0/function/name")
                .and_then(Value::as_str),
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
                assert_eq!(batch.calls[0].arguments, json!({"pattern": "Cargo.toml"}));
            }
            other => panic!("expected copilot tool-call response, got {other:?}"),
        }
    }

    #[test]
    fn copilot_runtime_streams_anthropic_models_after_token_exchange() {
        let transport = RecordingTransport::with_json_and_stream_bodies(
            vec![json!({
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
        let resolved = resolved_oauth_provider("copilot", Some("claude-sonnet-4"));
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
        let body: Value = serde_json::from_str(&stream_request.body).expect("request json");

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
            body.pointer("/model").and_then(Value::as_str),
            Some("claude-sonnet-4")
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
            json!({
                "token": "copilot-bearer",
                "expires_at": 1_750_000_000,
                "refresh_in": 900,
                "endpoints": {"api": "https://api.githubcopilot.com"}
            }),
            json!({
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
        let resolved = resolved_oauth_provider("copilot", Some("claude-sonnet-4"));

        let response = runtime
            .complete_with_tool_use(
                &resolved,
                &ToolUseRequest {
                    prompt: "Inspect the workspace".into(),
                    tools: vec![ProviderToolSpec {
                        name: "glob".into(),
                        description: "Find matching files".into(),
                        input_schema: json!({
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
        let body: Value = serde_json::from_str(&completion.body).expect("tool-use request json");

        assert_eq!(completion.url, "https://api.githubcopilot.com/v1/messages");
        assert_eq!(
            completion.headers.get("authorization").map(String::as_str),
            Some("Bearer copilot-bearer")
        );
        assert_eq!(
            body.pointer("/tool_choice/type").and_then(Value::as_str),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/tools/0/name").and_then(Value::as_str),
            Some("glob")
        );

        match response {
            ToolUseResponse::ToolCalls(batch) => {
                assert_eq!(batch.calls.len(), 1);
                assert_eq!(batch.calls[0].call_id, "toolu_copilot_1");
                assert_eq!(batch.calls[0].tool_name, "glob");
                assert_eq!(batch.calls[0].arguments, json!({"pattern": "Cargo.toml"}));
                assert_eq!(batch.stop_reason.as_deref(), Some("tool_use"));
            }
            other => panic!("expected copilot claude tool-call response, got {other:?}"),
        }
    }

    #[test]
    fn openai_request_includes_reasoning_effort_for_high() {
        let transport = RecordingTransport::with_json_body(json!({
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
        let body: Value = serde_json::from_str(&recorded.body).expect("body json");

        assert_eq!(
            body.pointer("/reasoning_effort").and_then(Value::as_str),
            Some("high"),
            "reasoning_effort should be 'high' for high effort level"
        );
    }

    #[test]
    fn anthropic_request_includes_thinking_for_high_effort_claude_model() {
        let transport = RecordingTransport::with_json_body(json!({
            "id": "msg_think_1",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "deep thought"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 5, "output_tokens": 2}
        }));
        let runtime = ProviderRuntime::with_transport(transport.clone() as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("anthropic", Some("claude-3-7-sonnet-latest"));
        let request = CompletionRequest {
            prompt: "think deeply".into(),
            effort_level: Some("max".into()),
            ..CompletionRequest::default()
        };

        runtime.complete(&resolved, &request).expect("complete");
        let recorded = transport.take_request();
        let body: Value = serde_json::from_str(&recorded.body).expect("body json");

        assert_eq!(
            body.pointer("/thinking/type").and_then(Value::as_str),
            Some("enabled"),
            "thinking type should be 'enabled' for high effort with claude model"
        );
        assert_eq!(
            body.pointer("/thinking/budget_tokens")
                .and_then(Value::as_u64),
            Some(10000),
            "thinking budget_tokens should be 10000"
        );
    }

    #[test]
    fn copilot_openai_request_includes_reasoning_effort_header_for_max_effort() {
        let transport = RecordingTransport::with_json_responses(vec![
            json!({
                "token": "copilot-bearer",
                "expires_at": 1_750_000_000,
                "refresh_in": 900,
                "endpoints": {"api": "https://api.githubcopilot.com"}
            }),
            json!({
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
            json!({"error": {"message": "Invalid API key"}}),
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
            json!({"error": {"message": "Rate limit exceeded. Please retry after 60 seconds."}}),
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
            json!({"error": {"message": "Internal server error"}}),
        );
        let runtime = ProviderRuntime::with_transport(transport as Arc<dyn HttpTransport>);
        let resolved = resolved_provider("anthropic", Some("claude-3-7-sonnet-latest"));
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
}
