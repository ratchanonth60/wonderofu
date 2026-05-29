//! Tool-use summary generation.
//!
//! After a batch of tool calls completes, this module can send those tool
//! names, inputs, and outputs to a fast model (Claude Haiku) to obtain a
//! short, human-readable label such as "Read config.json" or "Fixed NPE in
//! validate.ts".  The label is used in compact task/fleet views.
//!
//! All errors are swallowed and returned as [`None`]: summary generation is a
//! best-effort, non-critical path.

use std::path::Path;

use serde_json::Value;

use crate::{CompletionRequest, ProviderRuntime, ProviderSelection};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Haiku model used for fast, cheap summarisation.
const SUMMARY_MODEL: &str = "claude-haiku-4-5-20251001";
const SUMMARY_PROVIDER: &str = "anthropic";

/// Maximum tokens in the model response.  The summary is a single short line,
/// so 100 tokens is more than sufficient.
const SUMMARY_MAX_TOKENS: u32 = 100;

/// Maximum length (bytes) of a serialised JSON argument/output before it is
/// truncated with `"..."`.
const TRUNCATE_JSON_LEN: usize = 300;

const SYSTEM_PROMPT: &str = "\
Write a short summary label describing what these tool calls accomplished. \
It appears as a single-line row in a mobile app and truncates around 30 \
characters, so think git-commit-subject, not sentence.

Keep the verb in past tense and the most distinctive noun. Drop articles, \
connectors, and long location context first.

Examples:
- Searched in auth/
- Fixed NPE in UserService
- Created signup endpoint
- Read config.json
- Ran failing tests";

// ─── Public types ─────────────────────────────────────────────────────────────

/// One entry in a batch of completed tool calls.
pub struct ToolSummaryEntry {
    /// Tool name as registered in the tool registry.
    pub name: String,
    /// Raw JSON input sent to the tool.
    pub input: Value,
    /// Text output produced by the tool.
    pub output: String,
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Generates a one-line human-readable label for a completed batch of tool
/// calls.
///
/// Sends the tool names, truncated inputs, and truncated outputs to Claude
/// Haiku and returns the trimmed response text.
///
/// Returns [`None`] when:
/// - `tools` is empty
/// - the Anthropic provider or Haiku model is not available / not configured
/// - the API call fails for any reason
///
/// # Examples
///
/// ```no_run
/// use wonder_of_u_agent::{ProviderRuntime, tool_use_summary::{ToolSummaryEntry, generate_tool_use_summary}};
///
/// let runtime = ProviderRuntime::new();
/// let entries = vec![ToolSummaryEntry {
///     name: "file_read".into(),
///     input: serde_json::json!({"path": "Cargo.toml"}),
///     output: "[package]\nname = \"wonder-of-u\"".into(),
/// }];
/// let summary = generate_tool_use_summary(&entries, &runtime, None);
/// // summary might be Some("Read Cargo.toml")
/// ```
pub fn generate_tool_use_summary(
    tools: &[ToolSummaryEntry],
    runtime: &ProviderRuntime,
    storage_dir: Option<&Path>,
) -> Option<String> {
    if tools.is_empty() {
        return None;
    }

    try_generate(tools, runtime, storage_dir)
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Wraps the fallible generation path so the public function can convert any
/// error into `None` with a single `.ok().flatten()`.
fn try_generate(
    tools: &[ToolSummaryEntry],
    runtime: &ProviderRuntime,
    storage_dir: Option<&Path>,
) -> Option<String> {
    let selection =
        ProviderSelection::new(Some(SUMMARY_PROVIDER.into()), Some(SUMMARY_MODEL.into()));

    let resolved = runtime.resolve_execution(storage_dir, selection).ok()?;

    let tool_blocks: String = tools
        .iter()
        .map(|entry| {
            let input_str = truncate_json(&entry.input);
            let output_str = truncate_str(&entry.output);
            format!(
                "Tool: {}\nInput: {}\nOutput: {}",
                entry.name, input_str, output_str
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let user_prompt = format!("Tools completed:\n\n{tool_blocks}\n\nLabel:");

    let request = CompletionRequest {
        prompt: user_prompt,
        system_prompt: Some(SYSTEM_PROMPT.into()),
        max_output_tokens: Some(SUMMARY_MAX_TOKENS),
        temperature: None,
        effort_level: None,
    };

    let response = runtime.complete(&resolved, &request).ok()?;
    let trimmed = response.output_text.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Serialises `value` to JSON and truncates to at most [`TRUNCATE_JSON_LEN`]
/// bytes, appending `"..."` when the value was longer.  Falls back to a
/// placeholder string when serialisation fails.
fn truncate_json(value: &Value) -> String {
    match serde_json::to_string(value) {
        Ok(s) => truncate_str(&s),
        Err(_) => "[unable to serialize]".into(),
    }
}

/// Truncates `s` to at most [`TRUNCATE_JSON_LEN`] bytes at a valid UTF-8
/// character boundary, then appends `"..."`.
fn truncate_str(s: &str) -> String {
    if s.len() <= TRUNCATE_JSON_LEN {
        return s.to_string();
    }
    // Walk backwards from the byte limit to find a valid char boundary.
    let mut end = TRUNCATE_JSON_LEN.saturating_sub(3);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wonder_of_u_test_support::EnvVarGuard;

    use super::*;

    #[test]
    fn empty_tool_list_returns_none() {
        let runtime = ProviderRuntime::new();
        assert!(generate_tool_use_summary(&[], &runtime, None).is_none());
    }

    #[test]
    fn truncate_json_short_value_unchanged() {
        let v = serde_json::json!({"path": "foo.rs"});
        let out = truncate_json(&v);
        assert_eq!(out, r#"{"path":"foo.rs"}"#);
        assert!(!out.ends_with("..."));
    }

    #[test]
    fn truncate_json_long_value_gets_ellipsis() {
        // Build a string value longer than TRUNCATE_JSON_LEN bytes.
        let long_str = "x".repeat(400);
        let v = serde_json::json!(long_str);
        let out = truncate_json(&v);
        assert!(out.ends_with("..."));
        assert!(out.len() <= TRUNCATE_JSON_LEN);
    }

    #[test]
    fn truncate_str_at_char_boundary() {
        // 4-byte UTF-8 codepoint repeated so the slice boundary falls mid-codepoint.
        let s = "\u{1F600}".repeat(100); // each emoji is 4 bytes
        let out = truncate_str(&s);
        // Result must be valid UTF-8 (no panic) and end with ellipsis.
        assert!(out.ends_with("..."));
        assert!(out.len() <= TRUNCATE_JSON_LEN);
        // Ensure it's still valid UTF-8 by collecting chars.
        let _ = out.chars().count();
    }

    #[test]
    fn generate_returns_none_without_provider_auth() {
        // Remove the key for the duration of this test so resolve_execution fails
        // deterministically, regardless of the developer's environment.
        let _guard = EnvVarGuard::remove("ANTHROPIC_API_KEY");

        let runtime = ProviderRuntime::new();
        let entries = [ToolSummaryEntry {
            name: "file_read".into(),
            input: serde_json::json!({"path": "Cargo.toml"}),
            output: "content here".into(),
        }];
        let result = generate_tool_use_summary(&entries, &runtime, None);
        assert!(result.is_none());
    }

    #[test]
    fn prompt_contains_tool_name_and_label_suffix() {
        // Verify the prompt shape without making any HTTP call.
        let entries = [
            ToolSummaryEntry {
                name: "bash".into(),
                input: serde_json::json!({"command": "cargo test"}),
                output: "test result: ok".into(),
            },
            ToolSummaryEntry {
                name: "file_write".into(),
                input: serde_json::json!({"path": "out.txt"}),
                output: "".into(),
            },
        ];
        let tool_blocks: String = entries
            .iter()
            .map(|e| {
                let input_str = truncate_json(&e.input);
                let output_str = truncate_str(&e.output);
                format!(
                    "Tool: {}\nInput: {}\nOutput: {}",
                    e.name, input_str, output_str
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let prompt = format!("Tools completed:\n\n{tool_blocks}\n\nLabel:");

        assert!(prompt.contains("Tool: bash"));
        assert!(prompt.contains("Tool: file_write"));
        assert!(prompt.ends_with("Label:"));
    }
}
