//! OpenAI-compatible wire-protocol: request builders, response parsers, and
//! streaming helpers for the `/chat/completions` endpoint family.

use std::{collections::BTreeMap, io::BufReader};

use serde_json::{Value, json};
use wonder_of_u_core::{Result, TokenUsage, WonderError};

use crate::ResolvedProviderExecution;

use super::{
    CompletionRequest, CompletionResponse, HttpRequest, ProviderToolCall, StreamingHttpResponse,
    ToolCallBatchResponse, ToolConversationRound, ToolUseRequest, ToolUseResponse, consume_sse,
    context_window_for_model, is_high_effort, join_url,
};

// ─── Request builders ─────────────────────────────────────────────────────────

pub(super) fn build_openai_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_openai_request_with_mode(resolved, request, false)
}

pub(super) fn build_openai_stream_request(
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
    // Wire reasoning effort for OpenAI reasoning models.
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

pub(super) fn build_openai_tool_use_request(
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
        append_openai_round(&mut messages, round)?;
    }

    let tools = build_openai_tools(&request.tools);

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
    // Wire reasoning effort for OpenAI reasoning models.
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

// ─── Response parsers ─────────────────────────────────────────────────────────

pub(super) fn parse_openai_response(
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
        context_window_size: Some(context_window_for_model(resolved.model())),
        output_text,
        stop_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        usage,
    })
}

pub(super) fn parse_openai_tool_use_response(
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
            context_window_size: Some(context_window_for_model(resolved.model())),
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
        context_window_size: Some(context_window_for_model(resolved.model())),
        output_text,
        stop_reason,
        usage,
    }))
}

pub(super) fn parse_openai_stream_response<F>(
    resolved: &ResolvedProviderExecution,
    response: StreamingHttpResponse,
    on_text_delta: &mut F,
) -> Result<CompletionResponse>
where
    F: FnMut(&str) -> Result<()>,
{
    let mut output_text = String::new();
    let mut stop_reason = None;
    let mut usage = TokenUsage::default();
    consume_sse(BufReader::new(response.reader), |_, data| {
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
        context_window_size: Some(context_window_for_model(resolved.model())),
        output_text,
        stop_reason,
        usage,
    })
}

// ─── Shared utilities ─────────────────────────────────────────────────────────

pub(super) fn parse_openai_usage(usage: Option<&Value>) -> TokenUsage {
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

pub(super) fn extract_openai_text(content: Option<&Value>) -> Option<String> {
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

pub(super) fn parse_openai_tool_call(value: &Value) -> Result<ProviderToolCall> {
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

// ─── Private helpers ──────────────────────────────────────────────────────────

fn append_openai_round(messages: &mut Vec<Value>, round: &ToolConversationRound) -> Result<()> {
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
    Ok(())
}

fn build_openai_tools(tools: &[super::ProviderToolSpec]) -> Vec<Value> {
    tools
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
        .collect()
}
