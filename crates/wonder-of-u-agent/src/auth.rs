use std::{collections::BTreeMap, env, thread, time::Duration};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use wonder_of_u_core::{Result, WonderError};

/// Default copilot api base value
pub const DEFAULT_COPILOT_API_BASE: &str = "https://api.githubcopilot.com";
/// Default copilot device client id value
pub const DEFAULT_COPILOT_DEVICE_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
/// Default copilot device scope value
pub const DEFAULT_COPILOT_DEVICE_SCOPE: &str = "read:user";
/// Default copilot token url value
pub const DEFAULT_COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
/// Default github device access token url value
pub const DEFAULT_GITHUB_DEVICE_ACCESS_TOKEN_URL: &str =
    "https://github.com/login/oauth/access_token";
/// Default github device code url value
pub const DEFAULT_GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
/// Version string for copilot editor plugin
pub const COPILOT_EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.26.7";
/// Version string for copilot editor
pub const COPILOT_EDITOR_VERSION: &str = "vscode/1.99.3";
/// Constant copilot integration id
pub const COPILOT_INTEGRATION_ID: &str = "vscode-chat";
/// Constant copilot user agent
pub const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.26.7";
/// Stores resolved AWS credentials for SigV4 signing
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AwsCredentials {
    /// AWS access key ID
    pub access_key_id: String,
    /// AWS secret access key
    pub secret_access_key: String,
    /// Optional session token (for temporary credentials)
    pub session_token: Option<String>,
    /// AWS region (e.g. `us-east-1`)
    pub region: String,
}

/// Enumerates auth material
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthMaterial {
    /// Represents none
    None,
    /// Represents api key
    ApiKey {
        /// Stores the key
        key: String,
    },
    /// Represents o auth
    OAuth {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        /// Stores the access token
        access_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        /// Stores the refresh token
        refresh_token: Option<String>,
        #[serde(default, with = "time::serde::rfc3339::option")]
        /// Stores the expires at
        expires_at: Option<OffsetDateTime>,
    },
}
/// Represents stored credentials
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StoredCredentials {
    /// Stores the providers
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, AuthMaterial>,
}
/// Represents copilot device code
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CopilotDeviceCode {
    /// Stores the device code
    pub device_code: String,
    /// Stores the user code
    pub user_code: String,
    /// Stores the verification uri
    pub verification_uri: String,
    /// Stores the expires in
    pub expires_in: u64,
    /// Stores the interval
    pub interval: u64,
}
/// Represents copilot o auth token
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CopilotOAuthToken {
    /// Stores the access token
    pub access_token: String,
    /// Stores the refresh token
    pub refresh_token: Option<String>,
    /// Stores the expires at
    pub expires_at: Option<OffsetDateTime>,
}

/// Handles copilot device flow client id
pub fn copilot_device_flow_client_id() -> String {
    env::var("GITHUB_DEVICE_FLOW_CLIENT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_COPILOT_DEVICE_CLIENT_ID.to_string())
}

/// Handles github device code url
pub fn github_device_code_url() -> String {
    env::var("WONDER_OF_U_GITHUB_DEVICE_CODE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_DEVICE_CODE_URL.to_string())
}

/// Handles github device access token url
pub fn github_device_access_token_url() -> String {
    env::var("WONDER_OF_U_GITHUB_DEVICE_ACCESS_TOKEN_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_DEVICE_ACCESS_TOKEN_URL.to_string())
}

/// Handles copilot token url
pub fn copilot_token_url() -> String {
    env::var("WONDER_OF_U_COPILOT_TOKEN_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_COPILOT_TOKEN_URL.to_string())
}

/// Handles copilot standard headers
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

/// Requests copilot device code
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

/// Polls copilot access token
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

/// Refreshes copilot access token
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

/// Resolves AWS credentials from the standard environment variables.
///
/// Reads `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`
/// (optional), and `AWS_REGION` / `AWS_DEFAULT_REGION`.  Returns `None` when
/// the mandatory variables are absent or empty.
pub fn resolve_aws_credentials_from_env() -> Option<AwsCredentials> {
    let access_key_id = env::var("AWS_ACCESS_KEY_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let secret_access_key = env::var("AWS_SECRET_ACCESS_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let session_token = env::var("AWS_SESSION_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let region = env::var("AWS_REGION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var("AWS_DEFAULT_REGION")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "us-east-1".to_string());

    Some(AwsCredentials {
        access_key_id,
        secret_access_key,
        session_token,
        region,
    })
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
