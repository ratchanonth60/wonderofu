use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

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

/// Short-lived bearer token for Bedrock, sourced from `AWS_BEARER_TOKEN_BEDROCK`.
///
/// No SigV4 signing is required when this credential kind is present; the
/// token is passed directly as a `Bearer` authorization header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AwsBearerCredentials {
    /// The bearer token value
    pub token: String,
    /// AWS region (defaults to `us-east-1`)
    pub region: String,
}

/// AWS credentials resolved from a named profile in `~/.aws/credentials`.
///
/// The profile name comes from `AWS_PROFILE` (default `"default"`); the file
/// path can be overridden with `AWS_SHARED_CREDENTIALS_FILE`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AwsProfileCredentials {
    /// The profile name that was resolved (e.g. `"default"`, `"prod"`)
    pub profile: String,
    /// The AWS credentials extracted from the profile section
    pub credentials: AwsCredentials,
}

/// Readiness descriptor for GCP Vertex AI credentials.
///
/// Returned when `VERTEXAI_PROJECT`, `VERTEXAI_LOCATION`, **and**
/// `GOOGLE_APPLICATION_CREDENTIALS` are all present and non-empty.
/// Actual token exchange is deferred to the runtime stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GcpReadiness {
    /// GCP project ID (`VERTEXAI_PROJECT`)
    pub project: String,
    /// Vertex AI location (e.g. `us-central1`) from `VERTEXAI_LOCATION`
    pub location: String,
    /// Path to the service-account JSON key file (`GOOGLE_APPLICATION_CREDENTIALS`)
    pub credentials_source: String,
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
    let region = resolve_aws_region_from_env();

    Some(AwsCredentials {
        access_key_id,
        secret_access_key,
        session_token,
        region,
    })
}

/// Resolves a short-lived AWS bearer token from `AWS_BEARER_TOKEN_BEDROCK`.
///
/// This is the highest-priority auth path for Amazon Bedrock: when the token
/// is present no SigV4 request signing is required.  The region defaults to
/// `us-east-1` if neither `AWS_REGION` nor `AWS_DEFAULT_REGION` is set.
///
/// Returns `None` when `AWS_BEARER_TOKEN_BEDROCK` is absent or blank.
pub fn resolve_aws_bearer_from_env() -> Option<AwsBearerCredentials> {
    let token = env::var("AWS_BEARER_TOKEN_BEDROCK")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let region = resolve_aws_region_from_env();
    Some(AwsBearerCredentials { token, region })
}

/// Resolves AWS credentials from a named profile in `~/.aws/credentials`.
///
/// Resolution order:
/// 1. Profile name: `AWS_PROFILE` env var, falling back to `"default"`.
/// 2. Credentials file: `AWS_SHARED_CREDENTIALS_FILE` env var, falling back
///    to `$HOME/.aws/credentials`.
/// 3. Region within the profile section (if present), then `AWS_REGION` /
///    `AWS_DEFAULT_REGION` env vars, then `"us-east-1"`.
///
/// Returns `None` when the credentials file cannot be read or the requested
/// profile section does not contain the mandatory keys.
///
/// # Examples
///
/// ```no_run
/// if let Some(creds) = wonder_of_u_agent::resolve_aws_profile_from_env() {
///     println!("profile={} region={}", creds.profile, creds.credentials.region);
/// }
/// ```
pub fn resolve_aws_profile_from_env() -> Option<AwsProfileCredentials> {
    let profile = env::var("AWS_PROFILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "default".to_string());

    let creds_path = resolve_aws_credentials_file_path()?;
    let content = std::fs::read_to_string(&creds_path).ok()?;
    let credentials = parse_aws_credentials_file(&content, &profile)?;

    Some(AwsProfileCredentials {
        profile,
        credentials,
    })
}

/// Resolves GCP Vertex AI readiness from environment variables.
///
/// Returns `Some(GcpReadiness)` only when **all three** of the following
/// variables are present and non-empty:
/// - `VERTEXAI_PROJECT` — the GCP project ID
/// - `VERTEXAI_LOCATION` — the Vertex AI location (e.g. `us-central1`)
/// - `GOOGLE_APPLICATION_CREDENTIALS` — filesystem path to a service-account
///   key JSON file
///
/// No network calls or `gcloud` subprocess are made.  Actual OAuth 2 token
/// exchange is deferred to a later runtime stage.
pub fn resolve_gcp_credentials_from_env() -> Option<GcpReadiness> {
    let project = env::var("VERTEXAI_PROJECT")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let location = env::var("VERTEXAI_LOCATION")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let credentials_source = env::var("GOOGLE_APPLICATION_CREDENTIALS")
        .ok()
        .filter(|v| !v.trim().is_empty())?;

    Some(GcpReadiness {
        project,
        location,
        credentials_source,
    })
}

// ── internal helpers ─────────────────────────────────────────────────────────

/// Returns the effective AWS region, checking `AWS_REGION` then
/// `AWS_DEFAULT_REGION`, defaulting to `"us-east-1"`.
fn resolve_aws_region_from_env() -> String {
    env::var("AWS_REGION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            env::var("AWS_DEFAULT_REGION")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| "us-east-1".to_string())
}

/// Returns the path to the AWS credentials file, honouring
/// `AWS_SHARED_CREDENTIALS_FILE` and then `$HOME/.aws/credentials`.
pub(crate) fn resolve_aws_credentials_file_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("AWS_SHARED_CREDENTIALS_FILE") {
        let path = path.trim().to_string();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    let home = env::var("HOME").ok().filter(|v| !v.trim().is_empty())?;
    Some(Path::new(&home).join(".aws").join("credentials"))
}

/// Parses an INI-style AWS credentials file and extracts the named profile.
///
/// Only `aws_access_key_id` and `aws_secret_access_key` are mandatory; all
/// other keys are optional.  Region within the profile takes precedence over
/// environment variables.
pub(crate) fn parse_aws_credentials_file(content: &str, profile: &str) -> Option<AwsCredentials> {
    let mut in_section = false;
    let mut access_key_id: Option<String> = None;
    let mut secret_access_key: Option<String> = None;
    let mut session_token: Option<String> = None;
    let mut region_in_file: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();

        // Section header
        if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if in_section {
                // We just left our section — stop parsing.
                break;
            }
            in_section = inner.trim() == profile;
            continue;
        }

        if !in_section || line.starts_with('#') || line.starts_with(';') || line.is_empty() {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            // Strip inline comments from value.
            let value = value
                .split_once('#')
                .map(|(v, _)| v)
                .unwrap_or(value)
                .trim()
                .to_string();

            match key {
                "aws_access_key_id" if !value.is_empty() => access_key_id = Some(value),
                "aws_secret_access_key" if !value.is_empty() => secret_access_key = Some(value),
                "aws_session_token" if !value.is_empty() => session_token = Some(value),
                "region" if !value.is_empty() => region_in_file = Some(value),
                _ => {}
            }
        }
    }

    let region = region_in_file.unwrap_or_else(resolve_aws_region_from_env);

    Some(AwsCredentials {
        access_key_id: access_key_id?,
        secret_access_key: secret_access_key?,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_aws_credentials_file ───────────────────────────────────────────

    #[test]
    fn parse_credentials_file_default_profile() {
        let content = "\
[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
region = us-west-2
";
        let creds = parse_aws_credentials_file(content, "default").expect("parse default");
        assert_eq!(creds.access_key_id, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(
            creds.secret_access_key,
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
        );
        assert_eq!(creds.region, "us-west-2");
        assert!(creds.session_token.is_none());
    }

    #[test]
    fn parse_credentials_file_named_profile() {
        let content = "\
[default]
aws_access_key_id = DEFAULTKEY
aws_secret_access_key = DEFAULTSECRET

[prod]
aws_access_key_id = PRODKEY
aws_secret_access_key = PRODSECRET
aws_session_token = PRODSESSION
region = eu-west-1
";
        let creds = parse_aws_credentials_file(content, "prod").expect("parse prod");
        assert_eq!(creds.access_key_id, "PRODKEY");
        assert_eq!(creds.secret_access_key, "PRODSECRET");
        assert_eq!(creds.session_token.as_deref(), Some("PRODSESSION"));
        assert_eq!(creds.region, "eu-west-1");
    }

    #[test]
    fn parse_credentials_file_missing_profile_returns_none() {
        let content = "[default]\naws_access_key_id = KEY\naws_secret_access_key = SECRET\n";
        assert!(parse_aws_credentials_file(content, "nonexistent").is_none());
    }

    #[test]
    fn parse_credentials_file_missing_mandatory_key_returns_none() {
        // Only access_key_id, no secret → None
        let content = "[default]\naws_access_key_id = KEY\n";
        assert!(parse_aws_credentials_file(content, "default").is_none());
    }

    #[test]
    fn parse_credentials_file_strips_inline_comments() {
        let content = "\
[default]
aws_access_key_id = MYKEY # trailing comment
aws_secret_access_key = MYSECRET
";
        let creds = parse_aws_credentials_file(content, "default").expect("parse");
        assert_eq!(creds.access_key_id, "MYKEY");
    }

    // ── resolve_aws_bearer_from_env ──────────────────────────────────────────

    #[test]
    fn bearer_env_absent_returns_none() {
        // Use a temp env scope by checking without setting the var (relies on
        // test isolation; do not call env::set_var in parallel tests).
        // We simply assert the shape when the var is not set.
        let result = {
            // Ensure the var is unset for this sub-scope.
            let _guard = EnvGuard::unset("AWS_BEARER_TOKEN_BEDROCK");
            resolve_aws_bearer_from_env()
        };
        assert!(result.is_none(), "expected None when bearer token absent");
    }

    #[test]
    fn bearer_env_present_returns_credentials() {
        let _g1 = EnvGuard::set("AWS_BEARER_TOKEN_BEDROCK", "my-bearer-token");
        let _g2 = EnvGuard::set("AWS_REGION", "ap-southeast-1");
        let result = resolve_aws_bearer_from_env().expect("bearer token present");
        assert_eq!(result.token, "my-bearer-token");
        assert_eq!(result.region, "ap-southeast-1");
    }

    #[test]
    fn bearer_env_blank_returns_none() {
        let _g = EnvGuard::set("AWS_BEARER_TOKEN_BEDROCK", "   ");
        assert!(resolve_aws_bearer_from_env().is_none());
    }

    // ── resolve_aws_profile_from_env (via AWS_SHARED_CREDENTIALS_FILE) ───────

    #[test]
    fn profile_resolver_reads_credentials_file() {
        let content = "\
[default]
aws_access_key_id = FILEKEY
aws_secret_access_key = FILESECRET
region = us-east-2
";
        // Write the credentials to a temp file path using a real tempdir approach
        // by setting AWS_SHARED_CREDENTIALS_FILE to a known path.
        let dir = std::env::temp_dir().join("wou_auth_test_profile_resolver");
        std::fs::create_dir_all(&dir).ok();
        let creds_path = dir.join("credentials");
        std::fs::write(&creds_path, content).expect("write test creds");

        let _g1 = EnvGuard::set(
            "AWS_SHARED_CREDENTIALS_FILE",
            creds_path.to_str().expect("path"),
        );
        let _g2 = EnvGuard::unset("AWS_PROFILE");

        let result = resolve_aws_profile_from_env().expect("profile resolved");
        assert_eq!(result.profile, "default");
        assert_eq!(result.credentials.access_key_id, "FILEKEY");
        assert_eq!(result.credentials.region, "us-east-2");

        std::fs::remove_file(&creds_path).ok();
    }

    #[test]
    fn profile_resolver_missing_file_returns_none() {
        let _g = EnvGuard::set(
            "AWS_SHARED_CREDENTIALS_FILE",
            "/nonexistent/path/credentials",
        );
        assert!(resolve_aws_profile_from_env().is_none());
    }

    // ── resolve_gcp_credentials_from_env ─────────────────────────────────────

    #[test]
    fn gcp_env_all_absent_returns_none() {
        let _g1 = EnvGuard::unset("VERTEXAI_PROJECT");
        let _g2 = EnvGuard::unset("VERTEXAI_LOCATION");
        let _g3 = EnvGuard::unset("GOOGLE_APPLICATION_CREDENTIALS");
        assert!(resolve_gcp_credentials_from_env().is_none());
    }

    #[test]
    fn gcp_env_partially_set_returns_none() {
        let _g1 = EnvGuard::set("VERTEXAI_PROJECT", "my-project");
        let _g2 = EnvGuard::unset("VERTEXAI_LOCATION");
        let _g3 = EnvGuard::unset("GOOGLE_APPLICATION_CREDENTIALS");
        assert!(resolve_gcp_credentials_from_env().is_none());
    }

    #[test]
    fn gcp_env_all_present_returns_readiness() {
        let _g1 = EnvGuard::set("VERTEXAI_PROJECT", "my-gcp-project");
        let _g2 = EnvGuard::set("VERTEXAI_LOCATION", "us-central1");
        let _g3 = EnvGuard::set("GOOGLE_APPLICATION_CREDENTIALS", "/sa/key.json");
        let result = resolve_gcp_credentials_from_env().expect("GCP readiness present");
        assert_eq!(result.project, "my-gcp-project");
        assert_eq!(result.location, "us-central1");
        assert_eq!(result.credentials_source, "/sa/key.json");
    }

    // ── test helpers ─────────────────────────────────────────────────────────

    /// RAII guard that sets/restores a single environment variable for the
    /// duration of a test.  Must be used with `--test-threads=1` to avoid
    /// races when multiple tests touch the same var.
    struct EnvGuard {
        key: String,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: single-threaded test context only (--test-threads=1).
            unsafe { std::env::set_var(key, value) };
            Self {
                key: key.to_string(),
                previous,
            }
        }

        fn unset(key: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: single-threaded test context only (--test-threads=1).
            unsafe { std::env::remove_var(key) };
            Self {
                key: key.to_string(),
                previous,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(val) => {
                    // SAFETY: single-threaded test context only (--test-threads=1).
                    unsafe { std::env::set_var(&self.key, val) };
                }
                None => {
                    // SAFETY: single-threaded test context only (--test-threads=1).
                    unsafe { std::env::remove_var(&self.key) };
                }
            }
        }
    }
}
