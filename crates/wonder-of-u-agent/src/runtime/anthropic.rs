//! Anthropic Messages API wire-protocol: request builders, response parsers,
//! and SSE streaming helpers for the `/v1/messages` endpoint family.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use wonder_of_u_core::{Result, TokenUsage, WonderError};

use crate::ResolvedProviderExecution;

use super::{
    CompletionRequest, CompletionResponse, DEFAULT_ANTHROPIC_API_VERSION,
    DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS, HttpRequest, ProviderToolCall, StreamingHttpResponse,
    ToolCallBatchResponse, ToolUseRequest, ToolUseResponse, consume_sse, context_window_for_model,
    is_anthropic_model, is_high_effort, join_url,
};

// ─── Request builders ─────────────────────────────────────────────────────────

pub(super) fn build_anthropic_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_anthropic_request_with_mode(resolved, request, false)
}

pub(super) fn build_anthropic_stream_request(
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
    // Wire extended thinking for high-effort Anthropic Claude models.
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

pub(super) fn build_anthropic_tool_use_request(
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

/// Builds an Anthropic tool-use request body with caller-supplied headers and
/// `api_base`.  This entry point is shared with the Copilot and Bedrock paths,
/// which inject their own auth headers and endpoints.
pub(super) fn build_anthropic_tool_use_request_with_headers(
    model: &str,
    api_base: &str,
    headers: BTreeMap<String, String>,
    request: &ToolUseRequest,
) -> Result<HttpRequest> {
    let messages = build_anthropic_messages(request);

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
    // Wire extended thinking for high-effort Claude models.
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

// ─── Response parsers ─────────────────────────────────────────────────────────

pub(super) fn parse_anthropic_response(
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
        context_window_size: Some(context_window_for_model(resolved.model())),
        output_text,
        stop_reason: json
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        usage: parse_anthropic_usage(json.get("usage")),
    })
}

pub(super) fn parse_anthropic_tool_use_response(
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
            context_window_size: Some(context_window_for_model(resolved.model())),
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
        context_window_size: Some(context_window_for_model(resolved.model())),
        output_text,
        stop_reason,
        usage,
    }))
}

pub(super) fn parse_anthropic_stream_response<F>(
    resolved: &ResolvedProviderExecution,
    response: StreamingHttpResponse,
    on_text_delta: &mut F,
) -> Result<CompletionResponse>
where
    F: FnMut(&str) -> Result<()>,
{
    use std::io::BufReader;
    use wonder_of_u_core::WonderError;

    use super::provider_error_message;

    let mut output_text = String::new();
    let mut stop_reason = None;
    let mut usage = TokenUsage::default();
    consume_sse(BufReader::new(response.reader), |event, data| {
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
        context_window_size: Some(context_window_for_model(resolved.model())),
        output_text,
        stop_reason,
        usage,
    })
}

// ─── Shared utilities ─────────────────────────────────────────────────────────

pub(super) fn parse_anthropic_usage(usage: Option<&Value>) -> TokenUsage {
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

pub(super) fn merge_usage(current: &mut TokenUsage, next: TokenUsage) {
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

pub(super) fn extract_anthropic_text(content: &[Value]) -> String {
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

pub(super) fn parse_anthropic_tool_call(value: &Value) -> Result<ProviderToolCall> {
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

// ─── Private helpers ──────────────────────────────────────────────────────────

fn build_anthropic_messages(request: &ToolUseRequest) -> Vec<Value> {
    let mut messages = vec![json!({
        "role": "user",
        "content": [{
            "type": "text",
            "text": request.prompt.as_str(),
        }],
    })];
    for round in &request.rounds {
        append_anthropic_round(&mut messages, round);
    }
    messages
}

fn append_anthropic_round(messages: &mut Vec<Value>, round: &super::ToolConversationRound) {
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
