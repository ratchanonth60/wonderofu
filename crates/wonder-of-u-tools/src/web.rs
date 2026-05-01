//! Web fetch and search tools.

use std::env;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    FeatureFlag, PermissionDecision, PermissionDecisionReason, PermissionRequest, Result, Tool,
    ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId, WonderError,
    evaluate_permission,
};

use crate::{base_spec, parse_input, require_non_empty_text};

const DEFAULT_WEB_SEARCH_RESULTS: u8 = 5;
const PREAPPROVED_HOSTS: &[&str] = &[
    "platform.claude.com",
    "code.claude.com",
    "modelcontextprotocol.io",
    "agentskills.io",
    "docs.python.org",
    "en.cppreference.com",
    "docs.oracle.com",
    "learn.microsoft.com",
    "developer.mozilla.org",
    "go.dev",
    "pkg.go.dev",
    "doc.rust-lang.org",
    "react.dev",
    "nodejs.org",
    "docs.djangoproject.com",
    "jupyter.org",
    "docs.spring.io",
    "dotnet.microsoft.com",
    "developer.apple.com",
    "developer.android.com",
    "huggingface.co",
    "www.kaggle.com",
    "graphql.org",
    "docs.aws.amazon.com",
    "kubernetes.io",
    "docs.unity.com",
    "git-scm.com",
    "nginx.org",
];
const PREAPPROVED_HOST_PATHS: &[(&str, &str)] = &[("github.com", "/anthropics")];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebFetchInput {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

impl WebFetchInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("web_fetch", "url", &self.url)?;
        if self.max_length == Some(0) {
            return Err(WonderError::validation(
                "web_fetch max_length must be greater than zero",
            ));
        }
        if self.prompt.is_some() {
            return Err(WonderError::validation(
                "web_fetch prompt processing is not supported in wonder-of-u-tools",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchInput {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_results: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_domains: Option<Vec<String>>,
}

impl WebSearchInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("web_search", "query", &self.query)?;
        if self.num_results == Some(0) {
            return Err(WonderError::validation(
                "web_search num_results must be greater than zero",
            ));
        }
        if let Some(domains) = &self.allowed_domains {
            for domain in domains {
                require_non_empty_text("web_search", "allowed_domains", domain)?;
            }
            if !domains.is_empty() {
                return Err(WonderError::validation(
                    "web_search allowed_domains is not supported in wonder-of-u-tools",
                ));
            }
        }
        if let Some(domains) = &self.blocked_domains {
            for domain in domains {
                require_non_empty_text("web_search", "blocked_domains", domain)?;
            }
            if !domains.is_empty() {
                return Err(WonderError::validation(
                    "web_search blocked_domains is not supported in wonder-of-u-tools",
                ));
            }
        }
        Ok(())
    }

    fn num_results(&self) -> u8 {
        self.num_results.unwrap_or(DEFAULT_WEB_SEARCH_RESULTS)
    }
}

#[derive(Debug, Default)]
pub struct WebFetchTool;

#[derive(Debug, Default)]
pub struct WebSearchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("web_fetch", "Fetch a web page as plain text", ToolKind::Web)
            .with_input_schema(
            ToolSchema::object()
                .property("url", ToolSchema::string("url to fetch"))
                .property(
                    "max_length",
                    ToolSchema::integer("optional maximum number of characters to return"),
                )
                .property(
                    "prompt",
                    ToolSchema::string(
                        "source-compatible prompt field; currently unsupported in the Rust runtime",
                    ),
                )
                .required("url"),
        );
        spec.aliases.push("WebFetch".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::WebTools);
        spec
    }

    fn permission_decision(&self, context: &ToolContext, input: &Value) -> PermissionDecision {
        let spec = self.spec();
        let request = PermissionRequest::new(spec.name)
            .with_aliases(spec.aliases)
            .read_only(spec.read_only)
            .destructive(spec.destructive);
        let decision = evaluate_permission(&context.permission_context(), &request);

        match decision {
            PermissionDecision::Allow {
                reason: PermissionDecisionReason::Mode { mode, detail },
            } => preapproved_web_host(input).map_or_else(
                || PermissionDecision::allow(PermissionDecisionReason::Mode { mode, detail }),
                |host| {
                    PermissionDecision::allow(PermissionDecisionReason::Mode {
                        mode: context.permission_mode,
                        detail: format!("preapproved web host `{host}`"),
                    })
                },
            ),
            _ => decision,
        }
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<WebFetchInput>("web_fetch", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<WebFetchInput>("web_fetch", &input)?;
        input.validate()?;

        match fetch_text(&input) {
            Ok(content) => Ok(ToolResult::success(use_id, content)),
            Err(error) => Ok(ToolResult::failure(
                use_id,
                format!("fetch failed: {error}"),
            )),
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = base_spec("web_search", "Search the web", ToolKind::Web).with_input_schema(
            ToolSchema::object()
                .property("query", ToolSchema::string("search query"))
                .property(
                    "num_results",
                    ToolSchema::integer("optional maximum number of results to return"),
                )
                .property(
                    "allowed_domains",
                    json!({
                        "type": "array",
                        "description": "source-compatible domain allowlist; currently unsupported in the Rust runtime",
                        "items": { "type": "string" }
                    }),
                )
                .property(
                    "blocked_domains",
                    json!({
                        "type": "array",
                        "description": "source-compatible domain denylist; currently unsupported in the Rust runtime",
                        "items": { "type": "string" }
                    }),
                )
                .required("query"),
        );
        spec.aliases.push("WebSearch".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::WebTools);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<WebSearchInput>("web_search", input)?.validate()
    }

    async fn execute(
        &self,
        _context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<WebSearchInput>("web_search", &input)?;
        input.validate()?;

        let content = match (
            env::var("SERPER_API_KEY")
                .ok()
                .filter(|key| !key.trim().is_empty()),
            env::var("BRAVE_API_KEY")
                .ok()
                .filter(|key| !key.trim().is_empty()),
        ) {
            (Some(key), _) => match serper_search(&key, &input) {
                Ok(results) => results,
                Err(error) => {
                    return Ok(ToolResult::failure(
                        use_id,
                        format!("web search failed: {error}"),
                    ));
                }
            },
            (None, Some(key)) => match brave_search(&key, &input) {
                Ok(results) => results,
                Err(error) => {
                    return Ok(ToolResult::failure(
                        use_id,
                        format!("web search failed: {error}"),
                    ));
                }
            },
            (None, None) => not_configured_message(),
        };

        Ok(ToolResult::success(use_id, content))
    }
}

fn fetch_text(input: &WebFetchInput) -> Result<String> {
    let response = ureq::get(&input.url).call().map_err(map_ureq_error)?;
    let is_html = response
        .header("content-type")
        .is_some_and(|value| value.contains("html"));
    let body = response
        .into_string()
        .map_err(|error| WonderError::validation(format!("invalid response body: {error}")))?;
    let text = if is_html { strip_html(&body) } else { body };
    Ok(truncate_text(text, input.max_length))
}

fn serper_search(api_key: &str, input: &WebSearchInput) -> Result<String> {
    let body = json!({
        "q": input.query,
        "num": input.num_results(),
    })
    .to_string();
    let response = ureq::post("https://google.serper.dev/search")
        .set("X-API-KEY", api_key)
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(map_ureq_error)?;
    let body = response.into_string().map_err(|error| {
        WonderError::validation(format!("invalid serper response body: {error}"))
    })?;
    format_serper_results(&body, input.num_results())
}

fn brave_search(api_key: &str, input: &WebSearchInput) -> Result<String> {
    let response = ureq::get("https://api.search.brave.com/res/v1/web/search")
        .set("X-Subscription-Token", api_key)
        .query("q", &input.query)
        .query("count", &input.num_results().to_string())
        .call()
        .map_err(map_ureq_error)?;
    let body = response.into_string().map_err(|error| {
        WonderError::validation(format!("invalid brave response body: {error}"))
    })?;
    format_brave_results(&body, input.num_results())
}

fn format_serper_results(body: &str, limit: u8) -> Result<String> {
    let value: Value = serde_json::from_str(body)?;
    let mut lines = Vec::new();
    for (index, result) in value
        .get("organic")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(limit as usize)
        .enumerate()
    {
        let title = result
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("untitled");
        let link = result.get("link").and_then(Value::as_str).unwrap_or("-");
        let snippet = result.get("snippet").and_then(Value::as_str).unwrap_or("");
        lines.push(format!("{}. {}\n{}\n{}", index + 1, title, link, snippet));
    }
    Ok(render_search_results(lines))
}

fn format_brave_results(body: &str, limit: u8) -> Result<String> {
    let value: Value = serde_json::from_str(body)?;
    let mut lines = Vec::new();
    for (index, result) in value
        .get("web")
        .and_then(|value| value.get("results"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(limit as usize)
        .enumerate()
    {
        let title = result
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("untitled");
        let link = result.get("url").and_then(Value::as_str).unwrap_or("-");
        let snippet = result
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        lines.push(format!("{}. {}\n{}\n{}", index + 1, title, link, snippet));
    }
    Ok(render_search_results(lines))
}

fn render_search_results(lines: Vec<String>) -> String {
    if lines.is_empty() {
        "No results found.".into()
    } else {
        lines.join("\n\n")
    }
}

fn strip_html(html: &str) -> String {
    let script_re =
        Regex::new(r"(?is)<(script|style)[^>]*>.*?</(script|style)>").expect("valid script regex");
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("valid tag regex");
    let whitespace_re = Regex::new(r"[ \t\r\f\v]+").expect("valid whitespace regex");

    let without_scripts = script_re.replace_all(html, " ");
    let without_tags = tag_re.replace_all(&without_scripts, " ");
    let decoded = without_tags
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");

    whitespace_re
        .replace_all(&decoded, " ")
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_text(text: String, max_length: Option<u32>) -> String {
    match max_length {
        Some(max_length) => text.chars().take(max_length as usize).collect(),
        None => text,
    }
}

fn map_ureq_error(error: ureq::Error) -> WonderError {
    match error {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            WonderError::validation(format!("status {status}: {body}"))
        }
        ureq::Error::Transport(error) => WonderError::validation(error.to_string()),
    }
}

fn not_configured_message() -> String {
    "web search not configured — set SERPER_API_KEY or BRAVE_API_KEY".into()
}

fn preapproved_web_host(input: &Value) -> Option<String> {
    let url = input.get("url")?.as_str()?;
    let (host, path) = parse_url_host_path(url)?;
    is_preapproved_host(&host, &path).then_some(host)
}

fn parse_url_host_path(url: &str) -> Option<(String, String)> {
    let (_, remainder) = url.split_once("://")?;
    let split_at = remainder
        .find(|ch| ['/', '?', '#'].contains(&ch))
        .unwrap_or(remainder.len());
    let (authority, tail) = remainder.split_at(split_at);
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let authority = authority.trim();
    if authority.is_empty() {
        return None;
    }

    let host = if authority.starts_with('[') {
        authority
            .split_once(']')
            .map(|(host, _)| host.trim_start_matches('[').to_ascii_lowercase())?
    } else {
        authority
            .split(':')
            .next()
            .filter(|host| !host.is_empty())?
            .to_ascii_lowercase()
    };
    let path = if tail.is_empty() {
        "/".to_string()
    } else if tail.starts_with('/') {
        tail.to_string()
    } else {
        "/".to_string()
    };
    Some((host, path))
}

fn is_preapproved_host(hostname: &str, pathname: &str) -> bool {
    PREAPPROVED_HOSTS.contains(&hostname)
        || PREAPPROVED_HOST_PATHS.iter().any(|(host, prefix)| {
            *host == hostname
                && (pathname == *prefix || pathname.starts_with(&format!("{prefix}/")))
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use wonder_of_u_core::{
        FeatureSet, PermissionDecision, PermissionMode, PermissionRule, PermissionRuleBehavior,
        PermissionRuleSource, SessionId, ToolContext,
    };

    use super::*;

    fn tool_context(cwd: PathBuf) -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
        }
    }

    #[test]
    fn web_fetch_validation_rejects_empty_url() {
        let tool = WebFetchTool;
        let error = tool
            .validate_input(&json!({ "url": "   " }))
            .expect_err("empty url");

        assert!(error.to_string().contains("non-empty `url`"));
    }

    #[test]
    fn web_fetch_strips_html_and_truncates() {
        let text = strip_html("<html><body><h1>Hello</h1><p>Rust &amp; tools</p></body></html>");
        let truncated = truncate_text(text, Some(10));

        assert_eq!(truncated, "Hello Rust");
    }

    #[test]
    fn web_fetch_rejects_source_prompt_parameter() {
        let tool = WebFetchTool;
        let error = tool
            .validate_input(&json!({
                "url": "https://example.com",
                "prompt": "Summarize this page",
            }))
            .expect_err("unsupported prompt");

        assert!(error.to_string().contains("prompt"));
    }

    #[test]
    fn web_fetch_marks_preapproved_hosts_in_permission_reason() {
        let tool = WebFetchTool;
        let decision = tool.permission_decision(
            &tool_context(PathBuf::from("/workspace")),
            &json!({ "url": "https://docs.python.org/3/library/pathlib.html" }),
        );

        assert!(matches!(decision, PermissionDecision::Allow { .. }));
        assert!(decision.reason().to_string().contains("preapproved"));
    }

    #[test]
    fn web_fetch_preapproved_path_scopes_respect_segment_boundaries() {
        assert!(is_preapproved_host("github.com", "/anthropics/claude-code"));
        assert!(!is_preapproved_host(
            "github.com",
            "/anthropics-evil/claude-code"
        ));
    }

    #[test]
    fn web_fetch_permission_rules_match_aliases() {
        let tool = WebFetchTool;
        let mut ask_context = tool_context(PathBuf::from("/workspace"));
        ask_context.permission_rules.push(PermissionRule::new(
            "WebFetch",
            PermissionRuleBehavior::Ask,
            PermissionRuleSource::CliArg,
        ));
        let ask =
            tool.permission_decision(&ask_context, &json!({ "url": "https://example.com/docs" }));
        assert!(matches!(ask, PermissionDecision::Ask { .. }));

        let mut deny_context = tool_context(PathBuf::from("/workspace"));
        deny_context.permission_rules.push(PermissionRule::new(
            "WebFetch",
            PermissionRuleBehavior::Deny,
            PermissionRuleSource::CliArg,
        ));
        let deny =
            tool.permission_decision(&deny_context, &json!({ "url": "https://example.com/docs" }));
        assert!(matches!(deny, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn web_search_validation_rejects_zero_results() {
        let tool = WebSearchTool;
        let error = tool
            .validate_input(&json!({ "query": "rust", "num_results": 0 }))
            .expect_err("zero results");

        assert!(error.to_string().contains("num_results"));
    }

    #[test]
    fn web_search_rejects_source_domain_filters() {
        let tool = WebSearchTool;
        let error = tool
            .validate_input(&json!({
                "query": "rust",
                "allowed_domains": ["rust-lang.org"],
            }))
            .expect_err("unsupported domain filters");

        assert!(error.to_string().contains("allowed_domains"));
    }

    #[test]
    fn web_search_returns_stub_when_unconfigured() {
        assert!(not_configured_message().contains("SERPER_API_KEY"));
    }

    #[test]
    fn web_search_formats_serper_results() {
        let formatted = format_serper_results(
            r#"{"organic":[{"title":"Rust","link":"https://www.rust-lang.org","snippet":"Fast and reliable."}]}"#,
            5,
        )
        .expect("format serper");

        assert!(formatted.contains("Rust"));
        assert!(formatted.contains("https://www.rust-lang.org"));
    }
}
