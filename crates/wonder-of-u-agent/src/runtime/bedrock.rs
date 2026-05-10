//! Amazon Bedrock wire-protocol: AWS SigV4 request signing and Bedrock
//! `InvokeModel` / `InvokeModelWithResponseStream` request builders.
//!
//! Bedrock wraps the Anthropic Messages payload inside the Bedrock envelope
//! (the body field `anthropic_version` is required instead of the
//! `anthropic-version` HTTP header).  Response parsing is delegated to the
//! Anthropic module because the payload shape is identical.

use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use wonder_of_u_core::Result;

use crate::{ResolvedProviderExecution, auth::AwsCredentials};

use super::{CompletionRequest, DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS, HttpRequest, ToolUseRequest};

// ─── SigV4 signing ────────────────────────────────────────────────────────────

/// Computes an HMAC-SHA256 digest and returns the raw bytes.
pub(super) fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Computes a hex-encoded SHA-256 hash of the given bytes.
pub(super) fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Derives the SigV4 signing key from the secret access key, date, region,
/// and service name.
fn derive_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// Adds the AWS SigV4 `Authorization`, `x-amz-date`, `x-amz-content-sha256`,
/// and optional `x-amz-security-token` headers to `headers` in place.
///
/// `datetime` must be in the SigV4 format `YYYYMMDDTHHMMSSZ`.
pub(super) fn sign_request_headers(
    headers: &mut BTreeMap<String, String>,
    method: &str,
    url: &str,
    body_bytes: &[u8],
    credentials: &AwsCredentials,
    datetime: &str,
) -> Result<()> {
    let date = &datetime[..8]; // YYYYMMDD
    let service = "bedrock";

    let url_no_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let (host_and_path, query_str) = url_no_scheme.split_once('?').unwrap_or((url_no_scheme, ""));
    let host = host_and_path
        .split_once('/')
        .map(|(h, _)| h)
        .unwrap_or(host_and_path);
    let path = host_and_path
        .split_once('/')
        .map(|(_, p)| format!("/{p}"))
        .unwrap_or_else(|| "/".to_string());

    let payload_hash = sha256_hex(body_bytes);

    headers.insert("host".into(), host.to_string());
    headers.insert("x-amz-date".into(), datetime.to_string());
    headers.insert("x-amz-content-sha256".into(), payload_hash.clone());
    if let Some(token) = &credentials.session_token {
        headers.insert("x-amz-security-token".into(), token.clone());
    }

    let mut signed_header_names: Vec<String> =
        headers.keys().map(|k| k.to_ascii_lowercase()).collect();
    signed_header_names.sort();
    let signed_headers_str = signed_header_names.join(";");

    let canonical_headers_str: String = signed_header_names
        .iter()
        .map(|name| {
            let value = headers
                .iter()
                .find(|(k, _)| k.to_ascii_lowercase() == *name)
                .map(|(_, v)| v.trim())
                .unwrap_or_default();
            format!("{name}:{value}\n")
        })
        .collect();

    let canonical_request = [
        method,
        path.as_str(),
        query_str,
        canonical_headers_str.as_str(),
        signed_headers_str.as_str(),
        payload_hash.as_str(),
    ]
    .join("\n");

    let credential_scope = format!("{date}/{}/{service}/aws4_request", credentials.region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{datetime}\n{credential_scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    let signing_key = derive_signing_key(
        &credentials.secret_access_key,
        date,
        &credentials.region,
        service,
    );
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    headers.insert(
        "authorization".into(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers_str}, Signature={signature}",
            credentials.access_key_id,
        ),
    );

    Ok(())
}

/// Formats an [`OffsetDateTime`] as the SigV4 datetime string
/// `YYYYMMDDTHHMMSSZ`.
pub(super) fn sigv4_datetime(now: OffsetDateTime) -> String {
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    )
}

// ─── Request builders ─────────────────────────────────────────────────────────

pub(super) fn build_bedrock_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_bedrock_request_with_mode(resolved, request, false)
}

pub(super) fn build_bedrock_stream_request(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
) -> Result<HttpRequest> {
    build_bedrock_request_with_mode(resolved, request, true)
}

fn build_bedrock_request_with_mode(
    resolved: &ResolvedProviderExecution,
    request: &CompletionRequest,
    stream: bool,
) -> Result<HttpRequest> {
    let credentials = resolved.aws_credentials()?;

    let path_suffix = if stream {
        "invoke-with-response-stream"
    } else {
        "invoke"
    };
    let url = format!(
        "{}/model/{}/{}",
        resolved.api_base().trim_end_matches('/'),
        resolved.model(),
        path_suffix,
    );

    let mut body = json!({
        "anthropic_version": "bedrock-2023-05-31",
        "max_tokens": request.max_output_tokens.unwrap_or(DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS),
        "messages": [{"role": "user", "content": request.prompt}],
    });
    let body_map = body
        .as_object_mut()
        .expect("bedrock request body is an object");
    if let Some(system_prompt) = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        body_map.insert("system".into(), json!(system_prompt));
    }
    if let Some(temperature) = request.temperature {
        body_map.insert("temperature".into(), json!(temperature));
    }

    let body_str = serde_json::to_string(&body)?;
    let mut headers = BTreeMap::from([
        ("accept".into(), "application/json".into()),
        ("content-type".into(), "application/json".into()),
    ]);
    let datetime = sigv4_datetime(OffsetDateTime::now_utc());
    sign_request_headers(
        &mut headers,
        "POST",
        &url,
        body_str.as_bytes(),
        credentials,
        &datetime,
    )?;

    Ok(HttpRequest {
        method: "POST".into(),
        url,
        headers,
        body: body_str,
    })
}

pub(super) fn build_bedrock_tool_use_request(
    resolved: &ResolvedProviderExecution,
    request: &ToolUseRequest,
) -> Result<HttpRequest> {
    let credentials = resolved.aws_credentials()?;
    let url = format!(
        "{}/model/{}/invoke",
        resolved.api_base().trim_end_matches('/'),
        resolved.model(),
    );

    let mut messages = vec![json!({
        "role": "user",
        "content": [{"type": "text", "text": request.prompt.as_str()}],
    })];
    for round in &request.rounds {
        append_bedrock_round(&mut messages, round);
    }

    let mut body = json!({
        "anthropic_version": "bedrock-2023-05-31",
        "max_tokens": request.max_output_tokens.unwrap_or(DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS),
        "messages": messages,
    });
    let body_map = body
        .as_object_mut()
        .expect("bedrock tool-use body is an object");
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
        body_map.insert("tool_choice".into(), json!({"type": "auto"}));
    }
    if let Some(system_prompt) = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        body_map.insert("system".into(), json!(system_prompt));
    }
    if let Some(temperature) = request.temperature {
        body_map.insert("temperature".into(), json!(temperature));
    }

    let body_str = serde_json::to_string(&body)?;
    let mut headers = BTreeMap::from([
        ("accept".into(), "application/json".into()),
        ("content-type".into(), "application/json".into()),
    ]);
    let datetime = sigv4_datetime(OffsetDateTime::now_utc());
    sign_request_headers(
        &mut headers,
        "POST",
        &url,
        body_str.as_bytes(),
        credentials,
        &datetime,
    )?;

    Ok(HttpRequest {
        method: "POST".into(),
        url,
        headers,
        body: body_str,
    })
}

// ─── Private helpers ──────────────────────────────────────────────────────────

fn append_bedrock_round(messages: &mut Vec<Value>, round: &super::ToolConversationRound) {
    let mut assistant_content = Vec::new();
    if let Some(text) = round
        .assistant_text
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        assistant_content.push(json!({"type": "text", "text": text}));
    }
    assistant_content.extend(round.calls.iter().map(|call| {
        json!({
            "type": "tool_use",
            "id": call.call_id.as_str(),
            "name": call.tool_name.as_str(),
            "input": call.arguments.clone(),
        })
    }));
    messages.push(json!({"role": "assistant", "content": assistant_content}));
    messages.push(json!({
        "role": "user",
        "content": round.results.iter().map(|r| json!({
            "type": "tool_result",
            "tool_use_id": r.call_id.as_str(),
            "content": r.content.as_str(),
        })).collect::<Vec<_>>(),
    }));
}
