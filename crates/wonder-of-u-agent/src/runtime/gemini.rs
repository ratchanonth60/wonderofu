//! Google Gemini native REST API wire-protocol.
//!
//! Implements the `generateContent` endpoint at
//! `https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent`.
//!
//! The API key is passed as the `key` query parameter.  Streaming and tool-use
//! are not yet supported through this module; callers receive a clear
//! `validation` error that explains the limitation.

use std::collections::BTreeMap;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use wonder_of_u_core::{Result, TokenUsage, WonderError};

use crate::ResolvedProviderExecution;

use super::{CompletionRequest, CompletionResponse, HttpRequest, context_window_for_model};

// ─── Constants ────────────────────────────────────────────────────────────────

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1024;

// ─── Request builders ─────────────────────────────────────────────────────────

/// Builds a non-streaming `generateContent` request for the Gemini REST API.
pub(super) fn build_gemini_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    let api_key = resolved.api_key()?;
    // Percent-encode the model name so `/` in model IDs doesn't break routing.
    let model_encoded = utf8_percent_encode(resolved.model(), NON_ALPHANUMERIC).to_string();
    let url = format!(
        "{}/v1beta/models/{}:generateContent?key={}",
        resolved.api_base().trim_end_matches('/'),
        model_encoded,
        api_key,
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

/// Streaming is not yet supported for the Gemini native protocol.
///
/// Returns a descriptive validation error so callers can surface a clear
/// message rather than a generic panic or silent failure.
pub(super) fn gemini_streaming_unsupported() -> WonderError {
    WonderError::validation(
        "streaming is not yet supported for the `gemini_native` wire protocol; \
         use a provider with `open_ai_compat` or `anthropic_compat` for streaming",
    )
}

/// Tool-use is not yet supported for the Gemini native protocol.
pub(super) fn gemini_tool_use_unsupported() -> WonderError {
    WonderError::validation(
        "tool-use is not yet supported for the `gemini_native` wire protocol; \
         use a provider with `open_ai_compat` or `anthropic_compat` for tool use",
    )
}
