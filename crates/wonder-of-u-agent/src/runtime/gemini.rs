//! Google Gemini native REST API wire-protocol.
//!
//! Implements two endpoints on the Gemini REST API:
//!
//! * **Non-streaming** – `…/models/{model}:generateContent`
//! * **Streaming** – `…/models/{model}:streamGenerateContent?alt=sse`
//!
//! The API key is passed as the `key` query parameter.

use std::{collections::BTreeMap, io::BufReader};

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use wonder_of_u_core::{Result, TokenUsage, WonderError};

use crate::ResolvedProviderExecution;

use super::{
    CompletionRequest, CompletionResponse, HttpRequest, ProviderToolCall, ProviderToolSpec,
    StreamingHttpResponse, ToolCallBatchResponse, ToolUseRequest, ToolUseResponse, consume_sse,
    context_window_for_model, provider_input_schema,
};

// ─── Constants ────────────────────────────────────────────────────────────────

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1024;

// ─── Request builders ─────────────────────────────────────────────────────────

/// Builds a non-streaming `generateContent` request for the Gemini REST API.
pub(super) fn build_gemini_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_gemini_request_internal(resolved, request, false)
}

/// Builds a streaming `streamGenerateContent` request for the Gemini REST API.
///
/// The body is identical to the non-streaming request; only the endpoint
/// action changes (`streamGenerateContent` instead of `generateContent`) and
/// `alt=sse` is appended so the server returns Server-Sent Events rather than
/// a newline-delimited JSON array.
pub(super) fn build_gemini_stream_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_gemini_request_internal(resolved, request, true)
}

/// Builds a non-streaming tool-use `generateContent` request for Gemini.
pub(super) fn build_gemini_tool_use_request(
    resolved: &ResolvedProviderExecution,
    request: &ToolUseRequest,
) -> Result<HttpRequest> {
    let api_key = resolved.api_key()?;
    let model_encoded = utf8_percent_encode(resolved.model(), NON_ALPHANUMERIC).to_string();
    let url = format!(
        "{}/v1beta/models/{}:generateContent?key={}",
        resolved.api_base().trim_end_matches('/'),
        model_encoded,
        api_key,
    );
    let body = build_gemini_tool_use_body(request);

    Ok(HttpRequest {
        method: "POST".into(),
        url,
        headers: BTreeMap::from([
            ("accept".into(), "application/json".into()),
            ("content-type".into(), "application/json".into()),
        ]),
        body: serde_json::to_string(&body)?,
    })
}

fn build_gemini_request_internal(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
    stream: bool,
) -> Result<HttpRequest> {
    let api_key = resolved.api_key()?;
    // Percent-encode the model name so `/` in model IDs doesn't break routing.
    let model_encoded = utf8_percent_encode(resolved.model(), NON_ALPHANUMERIC).to_string();
    let (action, extra) = if stream {
        ("streamGenerateContent", "&alt=sse")
    } else {
        ("generateContent", "")
    };
    let url = format!(
        "{}/v1beta/models/{}:{}?key={}{}",
        resolved.api_base().trim_end_matches('/'),
        model_encoded,
        action,
        api_key,
        extra,
    );

    let mut contents = vec![json!({
        "role": "user",
        "parts": [{"text": request.prompt.as_str()}],
    })];
    // Gemini encodes system prompt as the first `"user"` turn when using the
    // v1beta endpoint; the dedicated `systemInstruction` field is preferred.
    let body = build_generate_content_body(request, &mut contents);

    Ok(HttpRequest {
        method: "POST".into(),
        url,
        headers: BTreeMap::from([
            ("accept".into(), "application/json".into()),
            ("content-type".into(), "application/json".into()),
        ]),
        body: serde_json::to_string(&body)?,
    })
}

/// Shared body builder for both Gemini and Vertex (same payload shape).
pub(super) fn build_generate_content_body(
    request: &CompletionRequest,
    contents: &mut Vec<Value>,
) -> Value {
    let mut body = json!({ "contents": contents });
    let body_map = body
        .as_object_mut()
        .expect("generate_content body is an object");

    if let Some(system) = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        body_map.insert(
            "systemInstruction".into(),
            json!({"parts": [{"text": system}]}),
        );
    }

    let mut gen_config = serde_json::Map::new();
    gen_config.insert(
        "maxOutputTokens".into(),
        json!(
            request
                .max_output_tokens
                .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
        ),
    );
    if let Some(temp) = request.temperature {
        gen_config.insert("temperature".into(), json!(temp));
    }
    body_map.insert("generationConfig".into(), Value::Object(gen_config));

    body
}

pub(super) fn build_gemini_tool_use_body(request: &ToolUseRequest) -> Value {
    let mut contents = build_gemini_tool_use_contents(request);
    let completion = CompletionRequest {
        prompt: request.prompt.clone(),
        system_prompt: request.system_prompt.clone(),
        max_output_tokens: request.max_output_tokens,
        temperature: request.temperature,
        effort_level: request.effort_level.clone(),
    };
    let mut body = build_generate_content_body(&completion, &mut contents);
    let body_map = body
        .as_object_mut()
        .expect("generate_content body is an object");
    if !request.tools.is_empty() {
        body_map.insert(
            "tools".into(),
            Value::Array(vec![build_gemini_function_declarations(&request.tools)]),
        );
        body_map.insert(
            "tool_config".into(),
            json!({"function_calling_config": {"mode": "AUTO"}}),
        );
    }
    body
}

pub(super) fn build_gemini_tool_use_contents(request: &ToolUseRequest) -> Vec<Value> {
    let mut contents = vec![json!({
        "role": "user",
        "parts": [{"text": request.prompt.as_str()}],
    })];

    for round in &request.rounds {
        let mut model_parts = Vec::new();
        if let Some(text) = round
            .assistant_text
            .as_deref()
            .filter(|text| !text.is_empty())
        {
            model_parts.push(json!({"text": text}));
        }
        for call in &round.calls {
            model_parts.push(json!({
                "functionCall": {
                    "name": call.tool_name,
                    "args": call.arguments,
                }
            }));
        }
        if !model_parts.is_empty() {
            contents.push(json!({"role": "model", "parts": model_parts}));
        }

        let response_parts: Vec<Value> = round
            .results
            .iter()
            .filter_map(|result| {
                let call = round
                    .calls
                    .iter()
                    .find(|call| call.call_id == result.call_id)?;
                Some(json!({
                    "functionResponse": {
                        "name": call.tool_name,
                        "response": {"output": result.content},
                    }
                }))
            })
            .collect();
        if !response_parts.is_empty() {
            contents.push(json!({"role": "user", "parts": response_parts}));
        }
    }

    contents
}

pub(super) fn build_gemini_function_declarations(tools: &[ProviderToolSpec]) -> Value {
    json!({
        "functionDeclarations": tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": provider_input_schema(&tool.input_schema),
                })
            })
            .collect::<Vec<_>>()
    })
}

// ─── Response parsers ─────────────────────────────────────────────────────────

/// Parses a `generateContent` JSON response into a [`CompletionResponse`].
///
/// Concatenates all `text` parts from the first candidate's content, then
/// extracts `usageMetadata` token counts.
pub(super) fn parse_gemini_response(
    resolved: &ResolvedProviderExecution,
    body: &str,
) -> Result<CompletionResponse> {
    let json: Value = serde_json::from_str(body)
        .map_err(|e| WonderError::validation(format!("invalid Gemini response JSON: {e}")))?;

    // Check for an API-level error object.
    if let Some(err_msg) = json.pointer("/error/message").and_then(Value::as_str) {
        return Err(WonderError::validation(format!(
            "Gemini API error: {err_msg}"
        )));
    }

    // Concatenate all text parts from the first candidate.
    let output_text = json
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.pointer("/text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default();

    let stop_reason = json
        .pointer("/candidates/0/finishReason")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    // Mirror the streaming path: an empty response is always an error.  This
    // surfaces SAFETY blocks, empty-candidate replies, and other provider-side
    // anomalies rather than returning a successful empty string to the caller.
    if output_text.trim().is_empty() {
        let finish_info = stop_reason
            .as_deref()
            .map(|r| format!(" (finishReason: {r})"))
            .unwrap_or_default();
        return Err(WonderError::validation(format!(
            "Gemini response did not contain any text content{finish_info}"
        )));
    }

    let input_tokens = json
        .pointer("/usageMetadata/promptTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = json
        .pointer("/usageMetadata/candidatesTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    Ok(CompletionResponse {
        provider: resolved.provider_id().to_string(),
        model: resolved.model().to_string(),
        context_window_size: Some(context_window_for_model(resolved.model())),
        output_text,
        stop_reason,
        usage: TokenUsage {
            input_tokens,
            output_tokens,
            ..TokenUsage::default()
        },
    })
}

/// Parses a Gemini function-calling response.
pub(super) fn parse_gemini_tool_use_response(
    resolved: &ResolvedProviderExecution,
    body: &str,
) -> Result<ToolUseResponse> {
    let json: Value = serde_json::from_str(body)
        .map_err(|e| WonderError::validation(format!("invalid Gemini response JSON: {e}")))?;
    if let Some(err_msg) = json.pointer("/error/message").and_then(Value::as_str) {
        return Err(WonderError::validation(format!(
            "Gemini API error: {err_msg}"
        )));
    }

    let parts = json
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let output_text = parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<String>();
    let calls: Vec<ProviderToolCall> = parts
        .iter()
        .filter_map(|part| part.get("functionCall"))
        .enumerate()
        .map(|(index, call)| ProviderToolCall {
            call_id: format!("gemini-call-{index}"),
            tool_name: call
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            arguments: call.get("args").cloned().unwrap_or_else(|| json!({})),
            thought_signature: None,
        })
        .filter(|call| !call.tool_name.is_empty())
        .collect();

    let stop_reason = json
        .pointer("/candidates/0/finishReason")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let usage = TokenUsage {
        input_tokens: json
            .pointer("/usageMetadata/promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: json
            .pointer("/usageMetadata/candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        ..TokenUsage::default()
    };

    if calls.is_empty() {
        // No function calls: this must be the model's final text answer.
        // If the text is also empty the response is unusable (e.g. SAFETY
        // block or an unexpected empty candidate); return an error so the
        // caller is never handed a successful but blank result.
        if output_text.trim().is_empty() {
            let finish_info = stop_reason
                .as_deref()
                .map(|r| format!(" (finishReason: {r})"))
                .unwrap_or_default();
            return Err(WonderError::validation(format!(
                "Gemini tool-use response contained no function calls and no text content{finish_info}"
            )));
        }
        return Ok(ToolUseResponse::Final(CompletionResponse {
            provider: resolved.provider_id().to_string(),
            model: resolved.model().to_string(),
            context_window_size: Some(context_window_for_model(resolved.model())),
            output_text,
            stop_reason,
            usage,
        }));
    }

    Ok(ToolUseResponse::ToolCalls(ToolCallBatchResponse {
        assistant_text: (!output_text.is_empty()).then_some(output_text),
        calls,
        context_window_size: Some(context_window_for_model(resolved.model())),
        stop_reason,
        usage,
    }))
}

/// Parses a `streamGenerateContent` SSE response into a [`CompletionResponse`].
///
/// Each SSE data event carries a partial `generateContent`-shaped JSON object.
/// Text is extracted from every chunk's `candidates[0].content.parts[*].text`
/// fields.  `finishReason` and `usageMetadata` are taken from the last chunk
/// that supplies them (the terminal event from the Gemini API).
///
/// The `on_text_delta` callback is invoked for each non-empty text chunk so
/// callers can display incremental output.
pub(super) fn parse_gemini_stream_response<F>(
    resolved: &ResolvedProviderExecution,
    response: StreamingHttpResponse,
    on_text_delta: &mut F,
) -> Result<CompletionResponse>
where
    F: FnMut(&str) -> Result<()>,
{
    let mut output_text = String::new();
    let mut stop_reason: Option<String> = None;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;

    consume_sse(BufReader::new(response.reader), |_, data| {
        let json: Value = serde_json::from_str(data)
            .map_err(|e| WonderError::validation(format!("invalid Gemini stream chunk: {e}")))?;

        // Each SSE chunk has the same shape as a non-streaming generateContent
        // response; text lives in candidates[0].content.parts[*].text.
        let chunk_text = json
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p.pointer("/text").and_then(Value::as_str))
                    .collect::<String>()
            })
            .unwrap_or_default();

        if !chunk_text.is_empty() {
            output_text.push_str(&chunk_text);
            on_text_delta(&chunk_text)?;
        }

        // The terminal chunk carries finishReason; earlier chunks may omit it.
        if let Some(reason) = json
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty() && *r != "FINISH_REASON_UNSPECIFIED")
        {
            stop_reason = Some(reason.to_string());
        }

        // usageMetadata is only present on the final chunk.
        if let Some(n) = json
            .pointer("/usageMetadata/promptTokenCount")
            .and_then(Value::as_u64)
        {
            input_tokens = n;
        }
        if let Some(n) = json
            .pointer("/usageMetadata/candidatesTokenCount")
            .and_then(Value::as_u64)
        {
            output_tokens = n;
        }

        Ok(true)
    })?;

    if output_text.trim().is_empty() {
        return Err(WonderError::validation(
            "Gemini streaming response did not contain any text content",
        ));
    }

    Ok(CompletionResponse {
        provider: resolved.provider_id().to_string(),
        model: resolved.model().to_string(),
        context_window_size: Some(context_window_for_model(resolved.model())),
        output_text,
        stop_reason,
        usage: TokenUsage {
            input_tokens,
            output_tokens,
            ..TokenUsage::default()
        },
    })
}
