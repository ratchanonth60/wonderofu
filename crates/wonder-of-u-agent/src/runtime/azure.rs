//! Azure OpenAI Service wire-protocol.
//!
//! Azure uses the same request/response payload shape as the OpenAI
//! Chat Completions API, with two differences:
//!
//! 1. The endpoint URL encodes the deployment name as the "model" segment:
//!    `{endpoint}/openai/deployments/{deployment}/chat/completions?api-version={version}`
//! 2. Authentication uses an `api-key` header instead of
//!    `Authorization: Bearer`.
//!
//! Streaming and tool-use are fully supported by delegating to the OpenAI
//! sub-module parsers.

use std::collections::BTreeMap;

use wonder_of_u_core::{Result, WonderError};

use crate::ResolvedProviderExecution;

use super::{
    CompletionRequest, CompletionResponse, HttpRequest, StreamingHttpResponse, ToolUseRequest,
    ToolUseResponse, openai,
};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Default Azure OpenAI API version used when `AZURE_OPENAI_API_VERSION` is
/// not set in the environment.
pub(super) const DEFAULT_AZURE_API_VERSION: &str = "2025-01-01-preview";

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn azure_api_version() -> String {
    std::env::var("AZURE_OPENAI_API_VERSION")
        .unwrap_or_else(|_| DEFAULT_AZURE_API_VERSION.to_string())
}

/// Builds the Azure chat-completions URL.
///
/// Shape: `{endpoint}/openai/deployments/{deployment}/chat/completions?api-version={version}`
fn azure_chat_completions_url(resolved: &ResolvedProviderExecution) -> String {
    format!(
        "{}/openai/deployments/{}/chat/completions?api-version={}",
        resolved.api_base().trim_end_matches('/'),
        resolved.model(),
        azure_api_version(),
    )
}

fn azure_headers(resolved: &ResolvedProviderExecution) -> Result<BTreeMap<String, String>> {
    let api_key = resolved.api_key()?;
    Ok(BTreeMap::from([
        ("accept".into(), "application/json".into()),
        ("api-key".into(), api_key.to_string()),
        ("content-type".into(), "application/json".into()),
    ]))
}

// ─── Request builders ─────────────────────────────────────────────────────────

/// Builds a non-streaming Azure chat-completions request.
///
/// The body is identical to the OpenAI `chat/completions` payload; only the
/// URL and `api-key` header differ.
pub(super) fn build_azure_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    // Build an OpenAI-compat body (minus the `model` field—Azure encodes the
    // deployment in the URL, so the field is redundant; we include it for
    // compatibility since Azure ignores extras).
    let openai_req = openai::build_openai_request(resolved, request)?;
    Ok(HttpRequest {
        method: "POST".into(),
        url: azure_chat_completions_url(resolved),
        headers: azure_headers(resolved)?,
        body: openai_req.body,
    })
}

/// Builds a streaming Azure chat-completions request.
pub(super) fn build_azure_stream_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    let openai_req = openai::build_openai_stream_request(resolved, request)?;
    Ok(HttpRequest {
        method: "POST".into(),
        url: azure_chat_completions_url(resolved),
        headers: azure_headers(resolved)?,
        body: openai_req.body,
    })
}

/// Builds an Azure chat-completions tool-use request.
pub(super) fn build_azure_tool_use_request(
    resolved: &ResolvedProviderExecution,
    request: &ToolUseRequest,
) -> Result<HttpRequest> {
    let openai_req = openai::build_openai_tool_use_request(resolved, request)?;
    Ok(HttpRequest {
        method: "POST".into(),
        url: azure_chat_completions_url(resolved),
        headers: azure_headers(resolved)?,
        body: openai_req.body,
    })
}

// ─── Response parsers ─────────────────────────────────────────────────────────

/// Parses an Azure chat-completions non-streaming response.
///
/// Azure returns the standard OpenAI `choices[0].message.content` payload,
/// so this delegates to [`openai::parse_openai_response`].
pub(super) fn parse_azure_response(
    resolved: &ResolvedProviderExecution,
    body: &str,
) -> Result<CompletionResponse> {
    openai::parse_openai_response(resolved, body)
}

/// Parses an Azure streaming response (SSE).
pub(super) fn parse_azure_stream_response<F>(
    resolved: &ResolvedProviderExecution,
    response: StreamingHttpResponse,
    on_text_delta: &mut F,
) -> Result<CompletionResponse>
where
    F: FnMut(&str) -> Result<()>,
{
    openai::parse_openai_stream_response(resolved, response, on_text_delta)
}

/// Parses an Azure tool-use response.
pub(super) fn parse_azure_tool_use_response(
    resolved: &ResolvedProviderExecution,
    body: &str,
) -> Result<ToolUseResponse> {
    openai::parse_openai_tool_use_response(resolved, body)
}

/// Returns a validation error explaining that the request failed due to a
/// missing Azure api-key.
#[allow(dead_code)]
pub(super) fn azure_missing_api_key_error(provider_id: &str) -> WonderError {
    WonderError::validation(format!(
        "provider `{provider_id}` requires Azure auth; set AZURE_OPENAI_API_KEY and AZURE_OPENAI_API_ENDPOINT"
    ))
}
