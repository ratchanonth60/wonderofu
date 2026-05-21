//! GitHub Copilot wire-protocol: OAuth token exchange, session management, and
//! request builders for both OpenAI-compat and Anthropic sub-paths under the
//! Copilot gateway.
//!
//! Copilot uses a two-step auth flow:
//! 1. Exchange the stored GitHub OAuth access token for a short-lived Copilot
//!    bearer token via `GET /copilot_internal/v2/token`.
//! 2. Send the actual completion request to the Copilot API endpoint returned
//!    in the token exchange response, using the bearer token.
//!
//! The endpoint shape (OpenAI-compat vs. Anthropic Messages) is determined by
//! model name: models whose names contain "claude" use the Anthropic path;
//! all others use the OpenAI-compat path.

use std::collections::BTreeMap;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use time::OffsetDateTime;
use wonder_of_u_core::{Result, WonderError};

use crate::{
    ResolvedProviderExecution,
    auth::{
        copilot_device_flow_client_id, copilot_standard_headers, copilot_token_url,
        github_device_access_token_url, parse_copilot_oauth_token_response,
    },
};

use super::{
    COPILOT_OAUTH_REFRESH_SKEW_SECONDS, CompletionRequest, DEFAULT_ANTHROPIC_API_VERSION,
    DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS, HttpRequest, ToolConversationRound, ToolUseRequest,
    is_anthropic_model, is_high_effort, join_url,
};

/// Short-lived Copilot API session produced by the token-exchange step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CopilotSession {
    /// The Copilot API base URL returned by the token exchange.
    pub(super) api_base: String,
    /// The short-lived bearer token for the Copilot API.
    pub(super) bearer_token: String,
}

// ─── Token exchange ───────────────────────────────────────────────────────────

pub(super) fn build_copilot_token_exchange_request(
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

pub(super) fn parse_copilot_token_exchange_response(
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

// ─── OAuth refresh ────────────────────────────────────────────────────────────

pub(super) fn build_copilot_oauth_refresh_request(refresh_token: &str) -> HttpRequest {
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

pub(super) fn parse_copilot_oauth_refresh_response(body: &str) -> Result<crate::CopilotOAuthToken> {
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

/// Returns `true` when the Copilot OAuth access token is close enough to its
/// expiry that a proactive refresh should be attempted.
pub(super) fn copilot_oauth_should_refresh(resolved: &ResolvedProviderExecution) -> bool {
    resolved.oauth_expires_at().is_some_and(|expires_at| {
        expires_at
            <= OffsetDateTime::now_utc()
                + time::Duration::seconds(COPILOT_OAUTH_REFRESH_SKEW_SECONDS)
    })
}

// ─── Model-sniff helper ───────────────────────────────────────────────────────

/// Returns `true` when the Copilot session should use the Anthropic Messages
/// sub-path rather than the OpenAI-compat sub-path.
///
/// Copilot routes based on the model name: Claude models use the Anthropic
/// Messages API shape, everything else uses OpenAI-compat.
#[inline]
pub(super) fn copilot_model_uses_anthropic_path(model: &str) -> bool {
    is_anthropic_model(model)
}

// ─── OpenAI-compat sub-path ───────────────────────────────────────────────────

/// Builds a Copilot completion request using the OpenAI-compat endpoint.
pub(super) fn build_copilot_openai_request(
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
    // Copilot passes reasoning effort via a request header rather than a body
    // field, because the Copilot gateway handles model routing before forwarding.
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

/// Builds a Copilot tool-use request using the OpenAI-compat endpoint.
pub(super) fn build_copilot_openai_tool_use_request(
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
        append_copilot_openai_round(&mut messages, round)?;
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
    // Copilot passes reasoning effort via a request header rather than a body
    // field, because the Copilot gateway handles model routing before forwarding.
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

fn append_copilot_openai_round(
    messages: &mut Vec<Value>,
    round: &ToolConversationRound,
) -> Result<()> {
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

// ─── Anthropic sub-path ───────────────────────────────────────────────────────

/// Builds a Copilot completion request using the Anthropic Messages endpoint.
pub(super) fn build_copilot_anthropic_request(
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
    // Wire extended thinking for high-effort Anthropic Claude models.
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

/// Builds a Copilot tool-use request using the Anthropic Messages endpoint.
pub(super) fn build_copilot_anthropic_tool_use_request(
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
    super::anthropic::build_anthropic_tool_use_request_with_headers(
        resolved.model(),
        api_base,
        headers,
        request,
    )
}
