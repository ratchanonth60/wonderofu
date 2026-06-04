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
const WEB_SEARCH_EXTENSION_LABEL: &str = "rust-external-web-search";
const WEB_SEARCH_NATIVE_STATUS: &str = "native-unavailable";
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
/// Represents web fetch input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebFetchInput {
    /// Stores the url
    pub url: String,
    /// Stores the max length
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// Stores the prompt
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
        // `prompt` is accepted at validation time and converted into a
        // provider-layer handoff in execute().
        Ok(())
    }
}
/// Represents web search input
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchInput {
    /// Stores the query
    pub query: String,
    /// Stores the num results
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_results: Option<u8>,
    /// Stores the allowed domains
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
    /// Stores the blocked domains
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
        }
        if let Some(domains) = &self.blocked_domains {
            for domain in domains {
                require_non_empty_text("web_search", "blocked_domains", domain)?;
            }
        }
        // Matches upstream: cannot specify both simultaneously.
        if self.allowed_domains.as_ref().is_some_and(|d| !d.is_empty())
            && self.blocked_domains.as_ref().is_some_and(|d| !d.is_empty())
        {
            return Err(WonderError::validation(
                "web_search cannot specify both allowed_domains and blocked_domains \
                 in the same request",
            ));
        }
        Ok(())
    }

    fn num_results(&self) -> u8 {
        self.num_results.unwrap_or(DEFAULT_WEB_SEARCH_RESULTS)
    }
}
/// Represents web fetch tool
#[derive(Debug, Default)]
pub struct WebFetchTool;
/// Represents web search tool
#[derive(Debug, Default)]
pub struct WebSearchTool;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WebSearchBackend {
    Serper,
    Brave,
    NotConfigured,
}

impl WebSearchBackend {
    const fn label(self) -> &'static str {
        match self {
            Self::Serper => "external-serper",
            Self::Brave => "external-brave",
            Self::NotConfigured => "not-configured",
        }
    }

    const fn display_name(self) -> &'static str {
        match self {
            Self::Serper => "Serper",
            Self::Brave => "Brave",
            Self::NotConfigured => "not configured",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExternalWebSearchConfig {
    backend: WebSearchBackend,
    api_key: Option<String>,
}

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
                            "optional prompt to apply to the fetched content; the tool fetches \
                         the page and returns a structured provider handoff so the model can \
                         process the prompt using the fetched content in the same tool loop",
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
            Ok(content) => match input.prompt.as_deref() {
                Some(prompt) => Ok(build_prompt_handoff_result(
                    use_id,
                    &input.url,
                    prompt,
                    input.max_length,
                    content,
                )),
                None => Ok(ToolResult::success(use_id, content)),
            },
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
        let mut spec = base_spec(
            "web_search",
            "Search the web via the Rust external web search extension when configured",
            ToolKind::Web,
        )
        .with_input_schema(
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
                        "description": "restrict results to URLs whose host matches one of these \
                                        domains (subdomains included); mutually exclusive with \
                                        blocked_domains",
                        "items": { "type": "string" }
                    }),
                )
                .property(
                    "blocked_domains",
                    json!({
                        "type": "array",
                        "description": "exclude results whose host matches any of these domains \
                                        (subdomains included); mutually exclusive with \
                                        allowed_domains",
                        "items": { "type": "string" }
                    }),
                )
                .required("query"),
        );
        spec.aliases.push("WebSearch".into());
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::WebTools);
        annotate_web_search_schema(&mut spec);
        spec
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        parse_input::<WebSearchInput>("web_search", input)?.validate()
    }

    async fn execute(
        &self,
        context: ToolContext,
        use_id: ToolUseId,
        input: Value,
    ) -> Result<ToolResult> {
        let input = parse_input::<WebSearchInput>("web_search", &input)?;
        input.validate()?;

        let config = external_web_search_config();
        let content = match (&config.backend, config.api_key.as_deref()) {
            (WebSearchBackend::Serper, Some(key)) => match serper_search(key, &input) {
                Ok(results) => results,
                Err(error) => {
                    return Ok(web_search_result(
                        use_id,
                        false,
                        format!("web search failed: {error}"),
                        &context,
                        config.backend,
                    ));
                }
            },
            (WebSearchBackend::Brave, Some(key)) => match brave_search(key, &input) {
                Ok(results) => results,
                Err(error) => {
                    return Ok(web_search_result(
                        use_id,
                        false,
                        format!("web search failed: {error}"),
                        &context,
                        config.backend,
                    ));
                }
            },
            (WebSearchBackend::NotConfigured, _) => not_configured_message(),
            _ => not_configured_message(),
        };

        Ok(web_search_result(
            use_id,
            true,
            content,
            &context,
            config.backend,
        ))
    }
}

fn fetch_text(input: &WebFetchInput) -> Result<String> {
    let mut response = ureq::get(&input.url).call().map_err(map_ureq_error)?;
    let is_html = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| value.contains("html"));
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| WonderError::validation(format!("invalid response body: {error}")))?;
    let text = if is_html { strip_html(&body) } else { body };
    Ok(truncate_text(text, input.max_length))
}

fn build_prompt_handoff_result(
    use_id: ToolUseId,
    url: &str,
    prompt: &str,
    max_length: Option<u32>,
    content: String,
) -> ToolResult {
    ToolResult::success(
        use_id,
        render_prompt_handoff(url, prompt, max_length.is_some(), &content),
    )
    .with_metadata(json!({
        "mode": "provider_prompt_handoff",
        "prompt_processed_by_tool": false,
        "url": url,
        "prompt": prompt,
    }))
}

fn render_prompt_handoff(url: &str, prompt: &str, truncated: bool, content: &str) -> String {
    let truncation_note = if truncated {
        "\nFetched content was truncated according to `max_length`; use only the content included below."
    } else {
        ""
    };
    format!(
        "Apply the requested prompt to the fetched content below. This tool fetched the page but \
         did not execute the prompt itself.{truncation_note}\n\n\
         <web_fetch_prompt_handoff>\n\
         <source_url>{url}</source_url>\n\
         <requested_prompt>\n\
         {prompt}\n\
         </requested_prompt>\n\
         <fetched_content>\n\
         {content}\n\
         </fetched_content>\n\
         </web_fetch_prompt_handoff>"
    )
}

fn annotate_web_search_schema(spec: &mut ToolSpec) {
    if let Some(object) = spec.input_schema.as_object_mut() {
        object.insert(
            "x-web-search-runtime-dispatch".into(),
            json!("rust-external-extension"),
        );
        object.insert(
            "x-web-search-extension-label".into(),
            json!(WEB_SEARCH_EXTENSION_LABEL),
        );
        object.insert(
            "x-web-search-note".into(),
            json!(
                "provider-native web search is unavailable in this runtime context; configured Serper or Brave credentials enable the Rust external web search extension"
            ),
        );
        object.insert(
            "x-web-search-result-statuses".into(),
            json!([
                WEB_SEARCH_NATIVE_STATUS,
                WebSearchBackend::Serper.label(),
                WebSearchBackend::Brave.label(),
                WebSearchBackend::NotConfigured.label(),
            ]),
        );
    }
}

pub(crate) fn should_expose_web_search(context: &ToolContext) -> bool {
    should_expose_web_search_with_backend(context, external_web_search_config().backend)
}

pub(crate) fn should_expose_web_search_with_backend(
    context: &ToolContext,
    backend: WebSearchBackend,
) -> bool {
    if context.provider.is_none() && context.model.is_none() {
        return true;
    }

    !matches!(backend, WebSearchBackend::NotConfigured)
}

fn external_web_search_config() -> ExternalWebSearchConfig {
    external_web_search_config_with_env(|name| env::var(name).ok())
}

fn external_web_search_config_with_env(
    getenv: impl Fn(&str) -> Option<String>,
) -> ExternalWebSearchConfig {
    let serper = getenv("SERPER_API_KEY").filter(|key| !key.trim().is_empty());
    let brave = getenv("BRAVE_API_KEY").filter(|key| !key.trim().is_empty());

    match (serper, brave) {
        (Some(api_key), _) => ExternalWebSearchConfig {
            backend: WebSearchBackend::Serper,
            api_key: Some(api_key),
        },
        (None, Some(api_key)) => ExternalWebSearchConfig {
            backend: WebSearchBackend::Brave,
            api_key: Some(api_key),
        },
        (None, None) => ExternalWebSearchConfig {
            backend: WebSearchBackend::NotConfigured,
            api_key: None,
        },
    }
}

fn web_search_result(
    use_id: ToolUseId,
    success: bool,
    content: impl Into<String>,
    context: &ToolContext,
    backend: WebSearchBackend,
) -> ToolResult {
    let result = if success {
        ToolResult::success(use_id, content)
    } else {
        ToolResult::failure(use_id, content)
    };

    result.with_metadata(json!({
        "tool": "web_search",
        "extension": WEB_SEARCH_EXTENSION_LABEL,
        "provider_native_status": WEB_SEARCH_NATIVE_STATUS,
        "search_backend": backend.label(),
        "selected_provider": context.provider.clone(),
        "selected_model": context.model.clone(),
    }))
}

fn serper_search(api_key: &str, input: &WebSearchInput) -> Result<String> {
    let body = json!({
        "q": input.query,
        "num": input.num_results(),
    })
    .to_string();
    let mut response = ureq::post("https://google.serper.dev/search")
        .header("X-API-KEY", api_key)
        .header("Content-Type", "application/json")
        .send(&body)
        .map_err(map_ureq_error)?;
    let body = response.body_mut().read_to_string().map_err(|error| {
        WonderError::validation(format!("invalid serper response body: {error}"))
    })?;
    format_serper_results(&body, input)
}

fn brave_search(api_key: &str, input: &WebSearchInput) -> Result<String> {
    let mut response = ureq::get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", api_key)
        .query("q", &input.query)
        .query("count", input.num_results().to_string())
        .call()
        .map_err(map_ureq_error)?;
    let body = response.body_mut().read_to_string().map_err(|error| {
        WonderError::validation(format!("invalid brave response body: {error}"))
    })?;
    format_brave_results(&body, input)
}

fn format_serper_results(body: &str, input: &WebSearchInput) -> Result<String> {
    let value: Value = serde_json::from_str(body)?;
    let mut lines = Vec::new();
    for result in value
        .get("organic")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if lines.len() >= input.num_results() as usize {
            break;
        }
        let link = result.get("link").and_then(Value::as_str).unwrap_or("-");
        if !result_url_passes_domain_filter(link, input) {
            continue;
        }
        let title = result
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("untitled");
        let snippet = result.get("snippet").and_then(Value::as_str).unwrap_or("");
        lines.push(format!(
            "{}. {}\n{}\n{}",
            lines.len() + 1,
            title,
            link,
            snippet
        ));
    }
    Ok(render_search_results(WebSearchBackend::Serper, lines))
}

fn format_brave_results(body: &str, input: &WebSearchInput) -> Result<String> {
    let value: Value = serde_json::from_str(body)?;
    let mut lines = Vec::new();
    for result in value
        .get("web")
        .and_then(|value| value.get("results"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if lines.len() >= input.num_results() as usize {
            break;
        }
        let link = result.get("url").and_then(Value::as_str).unwrap_or("-");
        if !result_url_passes_domain_filter(link, input) {
            continue;
        }
        let title = result
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("untitled");
        let snippet = result
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        lines.push(format!(
            "{}. {}\n{}\n{}",
            lines.len() + 1,
            title,
            link,
            snippet
        ));
    }
    Ok(render_search_results(WebSearchBackend::Brave, lines))
}

/// Returns `true` when `url_host` matches the `filter_domain`.
///
/// Matching is case-insensitive and strips a leading `www.` from both sides so
/// that `rust-lang.org` matches `www.rust-lang.org`, `docs.rust-lang.org`, etc.
fn domain_matches_filter(url_host: &str, filter_domain: &str) -> bool {
    let h = url_host.to_ascii_lowercase();
    let d = filter_domain.trim().to_ascii_lowercase();
    let h = h.trim_start_matches("www.");
    let d = d.trim_start_matches("www.");
    if d.is_empty() {
        return false;
    }
    h == d || h.ends_with(&format!(".{d}"))
}

/// Applies the `allowed_domains` / `blocked_domains` filter from `input` to
/// `url`.  Returns `true` if the result should be included in the output.
///
/// Semantics match the upstream WebSearchTool:
/// - If `allowed_domains` is non-empty, only URLs whose host matches are kept.
/// - If `blocked_domains` is non-empty, URLs whose host matches are removed.
/// - Empty / absent lists are a no-op (all URLs pass).
fn result_url_passes_domain_filter(url: &str, input: &WebSearchInput) -> bool {
    let Some((host, _)) = parse_url_host_path(url) else {
        // Unparseable URL: keep it to avoid silently dropping results.
        return true;
    };

    if let Some(allowed) = input.allowed_domains.as_deref().filter(|d| !d.is_empty()) {
        return allowed
            .iter()
            .any(|domain| domain_matches_filter(&host, domain));
    }

    if let Some(blocked) = input.blocked_domains.as_deref().filter(|d| !d.is_empty()) {
        return !blocked
            .iter()
            .any(|domain| domain_matches_filter(&host, domain));
    }

    true
}

fn render_search_results(backend: WebSearchBackend, lines: Vec<String>) -> String {
    let body = if lines.is_empty() {
        "No results found.".to_string()
    } else {
        lines.join("\n\n")
    };

    format!(
        "Rust external web search ({})\n\n{body}",
        backend.display_name()
    )
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
        ureq::Error::StatusCode(status) => WonderError::validation(format!("status {status}")),
        other => WonderError::validation(other.to_string()),
    }
}

fn not_configured_message() -> String {
    "Rust external web search is not configured — set SERPER_API_KEY or BRAVE_API_KEY. Provider-native web search is unavailable in this runtime context.".into()
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
    use std::{
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        thread,
    };

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
            session_worktree: None,
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            provider: None,
            model: None,
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
            progress_tx: None,
            interaction_rx: None,
            fork_context: None,
        }
    }

    fn provider_tool_context(cwd: PathBuf) -> ToolContext {
        let mut context = tool_context(cwd);
        context.provider = Some("openai".into());
        context.model = Some("gpt-4.1".into());
        context
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

    #[tokio::test]
    async fn web_fetch_without_prompt_fetches_plain_text() {
        use wonder_of_u_core::ToolUseId;

        let (url, server) = spawn_http_server(
            "text/html; charset=utf-8",
            "<html><body><h1>Hello</h1><p>Rust &amp; tools</p></body></html>",
        );
        let tool = WebFetchTool;
        let result = tool
            .execute(
                tool_context(PathBuf::from("/workspace")),
                ToolUseId::new(),
                serde_json::json!({
                    "url": url,
                    "max_length": 10,
                }),
            )
            .await
            .expect("execute should not return Err");
        server.join().expect("server thread should finish");

        assert!(result.success, "expected success result");
        assert_eq!(result.content, "Hello Rust");
        assert_eq!(result.metadata, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn web_fetch_prompt_returns_provider_handoff_result() {
        use wonder_of_u_core::ToolUseId;

        let (url, server) = spawn_http_server(
            "text/html; charset=utf-8",
            "<html><body><h1>Hello</h1><p>Rust &amp; tools</p></body></html>",
        );
        let tool = WebFetchTool;
        let result = tool
            .execute(
                tool_context(PathBuf::from("/workspace")),
                ToolUseId::new(),
                serde_json::json!({
                    "url": url,
                    "prompt": "Summarize the page in one sentence",
                }),
            )
            .await
            .expect("execute should not return Err");
        server.join().expect("server thread should finish");

        assert!(result.success, "expected success handoff result");
        assert_eq!(
            result.metadata["mode"],
            serde_json::json!("provider_prompt_handoff")
        );
        assert_eq!(
            result.metadata["prompt_processed_by_tool"],
            serde_json::json!(false)
        );
        assert!(
            result
                .content
                .contains("Summarize the page in one sentence"),
            "content should include the requested prompt"
        );
        assert!(
            result.content.contains("Hello Rust & tools"),
            "content should include fetched page text"
        );
        assert!(
            result.content.contains("<web_fetch_prompt_handoff>"),
            "content should expose the provider handoff envelope"
        );
    }

    #[test]
    fn web_fetch_validation_accepts_prompt_field() {
        // prompt remains valid input because execution handles it via a
        // provider-layer handoff instead of rejecting it locally.
        let tool = WebFetchTool;
        tool.validate_input(&serde_json::json!({
            "url": "https://example.com",
            "prompt": "Summarize this page",
        }))
        .expect("prompt should pass validation");
    }

    fn spawn_http_server(content_type: &str, body: &str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("local addr");
        let content_type = content_type.to_string();
        let body = body.to_string();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        });
        (format!("http://{address}"), handle)
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
    fn web_search_rejects_both_domain_filters_simultaneously() {
        // Matches upstream: cannot specify both allowed_domains and blocked_domains.
        let tool = WebSearchTool;
        let error = tool
            .validate_input(&json!({
                "query": "rust",
                "allowed_domains": ["rust-lang.org"],
                "blocked_domains": ["example.com"],
            }))
            .expect_err("both domain filters should be rejected");

        assert!(
            error.to_string().contains("allowed_domains")
                && error.to_string().contains("blocked_domains"),
            "error should mention both fields: {error}"
        );
    }

    #[test]
    fn web_search_allowed_domains_filters_serper_results() {
        let input = WebSearchInput {
            query: "rust".into(),
            num_results: Some(5),
            allowed_domains: Some(vec!["rust-lang.org".into()]),
            blocked_domains: None,
        };
        let body = r#"{"organic":[
            {"title":"Rust","link":"https://www.rust-lang.org","snippet":"fast"},
            {"title":"Other","link":"https://www.example.com","snippet":"unrelated"}
        ]}"#;
        let formatted = format_serper_results(body, &input).expect("format serper");
        assert!(
            formatted.contains("rust-lang.org"),
            "allowed domain should appear"
        );
        assert!(
            !formatted.contains("example.com"),
            "non-allowed domain should be filtered out"
        );
    }

    #[test]
    fn web_search_blocked_domains_filters_serper_results() {
        let input = WebSearchInput {
            query: "rust".into(),
            num_results: Some(5),
            allowed_domains: None,
            blocked_domains: Some(vec!["example.com".into()]),
        };
        let body = r#"{"organic":[
            {"title":"Rust","link":"https://www.rust-lang.org","snippet":"fast"},
            {"title":"Other","link":"https://www.example.com","snippet":"unrelated"}
        ]}"#;
        let formatted = format_serper_results(body, &input).expect("format serper");
        assert!(
            formatted.contains("rust-lang.org"),
            "non-blocked domain should appear"
        );
        assert!(
            !formatted.contains("example.com"),
            "blocked domain should be filtered out"
        );
    }

    #[test]
    fn web_search_subdomain_matches_allowed_domain() {
        assert!(domain_matches_filter("docs.rust-lang.org", "rust-lang.org"));
        assert!(domain_matches_filter("www.rust-lang.org", "rust-lang.org"));
        assert!(!domain_matches_filter("norust-lang.org", "rust-lang.org"));
    }

    #[test]
    fn web_search_no_domain_filter_passes_all_results() {
        let input = WebSearchInput {
            query: "rust".into(),
            num_results: Some(5),
            allowed_domains: None,
            blocked_domains: None,
        };
        let body = r#"{"organic":[
            {"title":"Rust","link":"https://www.rust-lang.org","snippet":"fast"},
            {"title":"Other","link":"https://www.example.com","snippet":"other"}
        ]}"#;
        let formatted = format_serper_results(body, &input).expect("format serper");
        assert!(formatted.contains("rust-lang.org"));
        assert!(formatted.contains("example.com"));
    }

    #[test]
    fn web_search_config_is_not_configured_without_keys() {
        let config = external_web_search_config_with_env(|_| None);

        assert_eq!(
            config,
            ExternalWebSearchConfig {
                backend: WebSearchBackend::NotConfigured,
                api_key: None,
            }
        );
        assert!(not_configured_message().contains("Rust external web search"));
        assert!(not_configured_message().contains("SERPER_API_KEY"));
    }

    #[test]
    fn web_search_config_prefers_serper_when_available() {
        let config = external_web_search_config_with_env(|name| match name {
            "SERPER_API_KEY" => Some("serper-key".into()),
            "BRAVE_API_KEY" => Some("brave-key".into()),
            _ => None,
        });

        assert_eq!(
            config,
            ExternalWebSearchConfig {
                backend: WebSearchBackend::Serper,
                api_key: Some("serper-key".into()),
            }
        );
    }

    #[test]
    fn web_search_config_uses_brave_when_serper_is_absent() {
        let config = external_web_search_config_with_env(|name| match name {
            "BRAVE_API_KEY" => Some("brave-key".into()),
            _ => None,
        });

        assert_eq!(
            config,
            ExternalWebSearchConfig {
                backend: WebSearchBackend::Brave,
                api_key: Some("brave-key".into()),
            }
        );
    }

    #[test]
    fn web_search_schema_marks_external_extension_metadata() {
        let spec = WebSearchTool.spec();

        assert!(
            spec.description
                .contains("Rust external web search extension")
        );
        assert_eq!(
            spec.input_schema["x-web-search-runtime-dispatch"],
            json!("rust-external-extension")
        );
        assert_eq!(
            spec.input_schema["x-web-search-extension-label"],
            json!(WEB_SEARCH_EXTENSION_LABEL)
        );
        assert_eq!(
            spec.input_schema["x-web-search-result-statuses"],
            json!([
                WEB_SEARCH_NATIVE_STATUS,
                "external-serper",
                "external-brave",
                "not-configured",
            ])
        );
    }

    #[test]
    fn web_search_exposure_requires_external_backend_when_provider_is_selected() {
        let plain_context = tool_context(PathBuf::from("/workspace"));
        let provider_context = provider_tool_context(PathBuf::from("/workspace"));

        assert!(should_expose_web_search_with_backend(
            &plain_context,
            WebSearchBackend::NotConfigured
        ));
        assert!(!should_expose_web_search_with_backend(
            &provider_context,
            WebSearchBackend::NotConfigured
        ));
        assert!(should_expose_web_search_with_backend(
            &provider_context,
            WebSearchBackend::Serper
        ));
    }

    #[tokio::test]
    async fn web_search_unconfigured_result_metadata_is_explicit() {
        use wonder_of_u_core::ToolUseId;

        let tool = WebSearchTool;
        let result = tool
            .execute(
                provider_tool_context(PathBuf::from("/workspace")),
                ToolUseId::new(),
                json!({ "query": "rust" }),
            )
            .await
            .expect("execute should not return Err");

        assert!(result.success);
        assert_eq!(
            result.metadata["extension"],
            json!(WEB_SEARCH_EXTENSION_LABEL)
        );
        assert_eq!(
            result.metadata["provider_native_status"],
            json!(WEB_SEARCH_NATIVE_STATUS)
        );
        assert_eq!(result.metadata["search_backend"], json!("not-configured"));
        assert_eq!(result.metadata["selected_provider"], json!("openai"));
        assert_eq!(result.metadata["selected_model"], json!("gpt-4.1"));
        assert!(result.content.contains("Rust external web search"));
    }

    #[test]
    fn web_search_formats_serper_results() {
        let input = WebSearchInput {
            query: "rust".into(),
            num_results: Some(5),
            allowed_domains: None,
            blocked_domains: None,
        };
        let formatted = format_serper_results(
            r#"{"organic":[{"title":"Rust","link":"https://www.rust-lang.org","snippet":"Fast and reliable."}]}"#,
            &input,
        )
        .expect("format serper");

        assert!(formatted.contains("Rust external web search (Serper)"));
        assert!(formatted.contains("Rust"));
        assert!(formatted.contains("https://www.rust-lang.org"));
    }
}
