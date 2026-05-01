//! Web fetch and search tools.

use std::env;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonder_of_u_core::{
    FeatureFlag, Result, Tool, ToolContext, ToolKind, ToolResult, ToolSchema, ToolSpec, ToolUseId,
    WonderError,
};

use crate::{base_spec, parse_input, require_non_empty_text};

const DEFAULT_WEB_SEARCH_RESULTS: u8 = 5;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebFetchInput {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
}

impl WebFetchInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("web_fetch", "url", &self.url)?;
        if self.max_length == Some(0) {
            return Err(WonderError::validation(
                "web_fetch max_length must be greater than zero",
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
}

impl WebSearchInput {
    fn validate(&self) -> Result<()> {
        require_non_empty_text("web_search", "query", &self.query)?;
        if self.num_results == Some(0) {
            return Err(WonderError::validation(
                "web_search num_results must be greater than zero",
            ));
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
                    .required("url"),
            );
        spec.read_only = true;
        spec.concurrency_safe = true;
        spec.required_features.insert(FeatureFlag::WebTools);
        spec
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
                .required("query"),
        );
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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
    fn web_search_validation_rejects_zero_results() {
        let tool = WebSearchTool;
        let error = tool
            .validate_input(&json!({ "query": "rust", "num_results": 0 }))
            .expect_err("zero results");

        assert!(error.to_string().contains("num_results"));
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
