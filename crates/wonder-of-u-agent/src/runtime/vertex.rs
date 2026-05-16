//! Google Vertex AI wire-protocol for Gemini models.
//!
//! Implements two endpoints against the Vertex AI REST API:
//!
//! * **Non-streaming** – `…/models/{model}:generateContent`
//! * **Streaming** – `…/models/{model}:streamGenerateContent`
//!
//! Authentication uses a GCP OAuth2 bearer token.  Provide `GOOGLE_BEARER_TOKEN`
//! in the environment for a pre-obtained token, or configure
//! `GOOGLE_APPLICATION_CREDENTIALS` for key-file-based exchange (full token
//! exchange from service account key is not yet implemented in-process).
//!
//! Tool-use is not yet supported; callers receive a clear validation error.

use std::collections::BTreeMap;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use wonder_of_u_core::{Result, WonderError};

use crate::ResolvedProviderExecution;

use super::{CompletionRequest, CompletionResponse, HttpRequest, StreamingHttpResponse, gemini};

// ─── Request builders ─────────────────────────────────────────────────────────

/// Builds a non-streaming `generateContent` request for a Vertex AI endpoint.
///
/// The endpoint URL is derived from the `project` and `location` stored in the
/// resolved GCP OAuth2 auth material.  The bearer token is required; if only a
/// key-file path was configured (no `GOOGLE_BEARER_TOKEN`), an error is
/// returned.
pub(super) fn build_vertex_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_vertex_request_internal(resolved, request, false)
}

/// Builds a streaming `streamGenerateContent` request for a Vertex AI endpoint.
///
/// The body is identical to the non-streaming request; only the action segment
/// of the URL changes to `streamGenerateContent`.
pub(super) fn build_vertex_stream_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_vertex_request_internal(resolved, request, true)
}

fn build_vertex_request_internal(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
    stream: bool,
) -> Result<HttpRequest> {
    let (project, location) = resolved.gcp_project_location()?;
    let bearer = resolved.gcp_access_token().ok_or_else(|| {
        WonderError::validation(
            "Vertex AI requires a GCP bearer token; set GOOGLE_BEARER_TOKEN to a \
             pre-obtained OAuth2 access token, or implement service-account key-file \
             exchange for production use",
        )
    })?;

    let model_encoded = utf8_percent_encode(resolved.model(), NON_ALPHANUMERIC).to_string();
    let action = if stream {
        "streamGenerateContent"
    } else {
        "generateContent"
    };
    let url = format!(
        "https://{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model_encoded}:{action}",
    );

    let mut contents = vec![serde_json::json!({
        "role": "user",
        "parts": [{"text": request.prompt.as_str()}],
    })];
    let body = gemini::build_generate_content_body(request, &mut contents);

    Ok(HttpRequest {
        method: "POST".into(),
        url,
        headers: BTreeMap::from([
            ("accept".into(), "application/json".into()),
            ("authorization".into(), format!("Bearer {bearer}")),
            ("content-type".into(), "application/json".into()),
        ]),
        body: serde_json::to_string(&body)?,
    })
}

// ─── Response parsers ─────────────────────────────────────────────────────────

/// Parses a Vertex AI `generateContent` response.
///
/// Vertex returns the same payload shape as the Gemini REST API, so this
/// delegates directly to [`gemini::parse_gemini_response`].
pub(super) fn parse_vertex_response(
    resolved: &ResolvedProviderExecution,
    body: &str,
) -> Result<CompletionResponse> {
    gemini::parse_gemini_response(resolved, body)
}

/// Parses a Vertex AI `streamGenerateContent` SSE response.
///
/// Vertex streams the same SSE event shape as the Gemini REST API, so this
/// delegates to [`gemini::parse_gemini_stream_response`].
pub(super) fn parse_vertex_stream_response<F>(
    resolved: &ResolvedProviderExecution,
    response: StreamingHttpResponse,
    on_text_delta: &mut F,
) -> Result<CompletionResponse>
where
    F: FnMut(&str) -> Result<()>,
{
    gemini::parse_gemini_stream_response(resolved, response, on_text_delta)
}

/// Tool-use is not yet supported for the Vertex Gemini protocol.
///
/// Returns a descriptive validation error so callers can surface a clear
/// message.  Use a provider with `open_ai_compat` or `anthropic_compat` for
/// tool use.
pub(super) fn vertex_tool_use_unsupported() -> WonderError {
    WonderError::validation(
        "tool-use is not yet supported for the `vertex_gemini` wire protocol; \
         use a provider with `open_ai_compat` or `anthropic_compat` for tool use",
    )
}
