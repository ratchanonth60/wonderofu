use std::{collections::BTreeMap, env, thread, time::Duration};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use wonder_of_u_core::{Result, WonderError};

pub const DEFAULT_COPILOT_API_BASE: &str = "https://api.githubcopilot.com";
pub const DEFAULT_COPILOT_DEVICE_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
pub const DEFAULT_COPILOT_DEVICE_SCOPE: &str = "read:user";
pub const DEFAULT_COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
pub const DEFAULT_GITHUB_DEVICE_ACCESS_TOKEN_URL: &str =
    "https://github.com/login/oauth/access_token";
pub const DEFAULT_GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
pub const COPILOT_EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.26.7";
pub const COPILOT_EDITOR_VERSION: &str = "vscode/1.99.3";
pub const COPILOT_INTEGRATION_ID: &str = "vscode-chat";
pub const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.26.7";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthMaterial {
    None,
    ApiKey {
        key: String,
    },
    OAuth {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        #[serde(default, with = "time::serde::rfc3339::option")]
        expires_at: Option<OffsetDateTime>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StoredCredentials {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, AuthMaterial>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CopilotDeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CopilotOAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<OffsetDateTime>,
}

pub fn copilot_device_flow_client_id() -> String {
    env::var("GITHUB_DEVICE_FLOW_CLIENT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_COPILOT_DEVICE_CLIENT_ID.to_string())
}

pub fn github_device_code_url() -> String {
    env::var("WONDER_OF_U_GITHUB_DEVICE_CODE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_DEVICE_CODE_URL.to_string())
}

pub fn github_device_access_token_url() -> String {
    env::var("WONDER_OF_U_GITHUB_DEVICE_ACCESS_TOKEN_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_DEVICE_ACCESS_TOKEN_URL.to_string())
}

pub fn copilot_token_url() -> String {
    env::var("WONDER_OF_U_COPILOT_TOKEN_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_COPILOT_TOKEN_URL.to_string())
}

pub fn copilot_standard_headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "copilot-integration-id".into(),
            COPILOT_INTEGRATION_ID.into(),
        ),
        (
            "editor-plugin-version".into(),
            COPILOT_EDITOR_PLUGIN_VERSION.into(),
        ),
        ("editor-version".into(), COPILOT_EDITOR_VERSION.into()),
        ("user-agent".into(), COPILOT_USER_AGENT.into()),
    ])
}

pub fn request_copilot_device_code() -> Result<CopilotDeviceCode> {
    let client_id = copilot_device_flow_client_id();
    let response = match ureq::post(github_device_code_url().as_str())
        .set("accept", "application/json")
        .send_form(&[
            ("client_id", client_id.as_str()),
            ("scope", DEFAULT_COPILOT_DEVICE_SCOPE),
        ]) {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(WonderError::validation(format!(
                "GitHub device code request failed with status {status}: {}",
                describe_auth_error(&body)
            )));
        }
        Err(ureq::Error::Transport(error)) => {
            return Err(WonderError::validation(format!(
                "GitHub device code request failed: {error}"
            )));
        }
    };
    let response_body = response.into_string().map_err(|error| {
        WonderError::validation(format!("invalid device code response body: {error}"))
    })?;
    let json = serde_json::from_str::<serde_json::Value>(&response_body).map_err(|error| {
        WonderError::validation(format!("invalid device code response: {error}"))
    })?;
    Ok(CopilotDeviceCode {
        device_code: required_string(&json, "/device_code", "device code response")?,
        user_code: required_string(&json, "/user_code", "device code response")?,
        verification_uri: required_string(&json, "/verification_uri", "device code response")?,
        expires_in: json
            .get("expires_in")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(900),
        interval: json
            .get("interval")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5)
            .max(1),
    })
}

pub fn poll_copilot_access_token(
    device_code: &str,
    initial_interval: u64,
    timeout: Duration,
) -> Result<CopilotOAuthToken> {
    let client_id = copilot_device_flow_client_id();
    let mut interval = initial_interval.max(1);
    let started = std::time::Instant::now();

    while started.elapsed() < timeout {
        let response = match ureq::post(github_device_access_token_url().as_str())
            .set("accept", "application/json")
            .send_form(&[
                ("client_id", client_id.as_str()),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]) {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                return Err(WonderError::validation(format!(
                    "GitHub oauth token request failed with status {status}: {}",
                    describe_auth_error(&body)
                )));
            }
            Err(ureq::Error::Transport(error)) => {
                return Err(WonderError::validation(format!(
                    "GitHub oauth token request failed: {error}"
                )));
            }
        };
        let response_body = response.into_string().map_err(|error| {
            WonderError::validation(format!("invalid oauth access token response body: {error}"))
        })?;
        let json = serde_json::from_str::<serde_json::Value>(&response_body).map_err(|error| {
            WonderError::validation(format!("invalid oauth access token response: {error}"))
        })?;
        match json
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
        {
            "" => {
                let token = parse_copilot_oauth_token_response(&json)?;
                if !token.access_token.trim().is_empty() {
                    return Ok(token);
                }
                return Err(WonderError::validation(
                    "oauth access token response did not contain an access token",
                ));
            }
            "authorization_pending" => thread::sleep(Duration::from_secs(interval)),
            "slow_down" => {
                interval = json
                    .get("interval")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(interval + 5)
                    .max(1);
                thread::sleep(Duration::from_secs(interval));
            }
            "expired_token" => {
                return Err(WonderError::validation(
                    "device code expired before authorization completed",
                ));
            }
            "access_denied" => {
                return Err(WonderError::validation(
                    "device authorization was denied or cancelled",
                ));
            }
            other => {
                return Err(WonderError::validation(format!(
                    "GitHub oauth device flow failed: {other}"
                )));
            }
        }
    }

    Err(WonderError::validation(
        "timed out waiting for GitHub device authorization",
    ))
}

pub fn refresh_copilot_access_token(refresh_token: &str) -> Result<CopilotOAuthToken> {
    if refresh_token.trim().is_empty() {
        return Err(WonderError::validation(
            "copilot oauth refresh token cannot be empty",
        ));
    }
    let client_id = copilot_device_flow_client_id();
    let response = match ureq::post(github_device_access_token_url().as_str())
        .set("accept", "application/json")
        .send_form(&[
            ("client_id", client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ]) {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(WonderError::validation(format!(
                "GitHub oauth refresh request failed with status {status}: {}",
                describe_auth_error(&body)
            )));
        }
        Err(ureq::Error::Transport(error)) => {
            return Err(WonderError::validation(format!(
                "GitHub oauth refresh request failed: {error}"
            )));
        }
    };
    let response_body = response.into_string().map_err(|error| {
        WonderError::validation(format!("invalid oauth refresh response body: {error}"))
    })?;
    let json = serde_json::from_str::<serde_json::Value>(&response_body).map_err(|error| {
        WonderError::validation(format!("invalid oauth refresh response: {error}"))
    })?;
    match json
        .get("error")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
    {
        "" => parse_copilot_oauth_token_response(&json),
        other => Err(WonderError::validation(format!(
            "GitHub oauth refresh failed: {other}"
        ))),
    }
}

pub(crate) fn parse_copilot_oauth_token_response(
    json: &serde_json::Value,
) -> Result<CopilotOAuthToken> {
    let access_token = required_string(json, "/access_token", "oauth access token response")?;
    let refresh_token = json
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let expires_at = oauth_expires_at_from_json(json)?;
    Ok(CopilotOAuthToken {
        access_token,
        refresh_token,
        expires_at,
    })
}

fn oauth_expires_at_from_json(json: &serde_json::Value) -> Result<Option<OffsetDateTime>> {
    if let Some(expires_at) = json
        .get("expires_at")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return OffsetDateTime::parse(expires_at, &time::format_description::well_known::Rfc3339)
            .map(Some)
            .map_err(|error| {
                WonderError::validation(format!("invalid oauth expiration timestamp: {error}"))
            });
    }
    Ok(json
        .get("expires_in")
        .and_then(serde_json::Value::as_i64)
        .filter(|value| *value > 0)
        .map(|seconds| OffsetDateTime::now_utc() + time::Duration::seconds(seconds)))
}

fn describe_auth_error(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "empty response body".into()
    } else {
        trimmed.to_string()
    }
}

fn required_string(json: &serde_json::Value, pointer: &str, context: &str) -> Result<String> {
    json.pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| WonderError::validation(format!("{context} missing {pointer}")))
}
