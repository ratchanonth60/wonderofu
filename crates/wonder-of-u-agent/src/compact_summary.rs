//! LLM-driven conversation summarization for `/compact`.
//!
//! Renders the messages selected for compaction into a plain-text transcript,
//! sends it through the configured provider runtime with a structured summary
//! prompt (ported from the original `services/compact/prompt.ts`), and returns
//! the model's summary for use as the `CompactBoundary` content. Callers fall
//! back to a mechanical summary when no provider is reachable.

use std::path::Path;

use wonder_of_u_core::{MessageEnvelope, MessagePayload, Result, WonderError};

use crate::{CompletionRequest, ProviderRuntime, ProviderSelection};

/// Maximum characters of rendered conversation sent to the summarizer. The
/// excerpt is tail-biased: when over budget the oldest messages are dropped
/// first, since the most recent context matters most for continuation.
const MAX_CONVERSATION_CHARS: usize = 96_000;

/// Per-message excerpt cap. Tool results and bash output dominate transcript
/// size but their full content is re-derivable, so they get a tighter cap.
const MESSAGE_EXCERPT_CHARS: usize = 1_500;
const TOOL_OUTPUT_EXCERPT_CHARS: usize = 600;

/// Maximum output tokens for the summary response.
const MAX_SUMMARY_OUTPUT_TOKENS: u32 = 4_096;

/// Notice prepended to the excerpt when older messages were dropped.
const TRUNCATION_NOTICE: &str =
    "[Note: earlier messages were omitted to fit the summarization context window]";

/// Kill-switch: set to `off`/`0`/`false` to skip the provider call and force
/// the mechanical fallback summary (useful offline and in tests, where a
/// leaked `*_API_KEY` env var would otherwise trigger a real network call).
pub const COMPACT_LLM_SUMMARY_ENV: &str = "WONDER_OF_U_COMPACT_LLM_SUMMARY";

fn llm_summary_disabled() -> bool {
    std::env::var(COMPACT_LLM_SUMMARY_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        )
    })
}

/// System prompt for the compaction summarizer, ported from the original
/// TypeScript `BASE_COMPACT_PROMPT`.
const COMPACT_SUMMARY_SYSTEM_PROMPT: &str = r#"Your task is to create a detailed summary of the conversation so far, paying close attention to the user's explicit requests and your previous actions.
This summary should be thorough in capturing technical details, code patterns, and architectural decisions that would be essential for continuing development work without losing context.

Before providing your final summary, wrap your analysis in <analysis> tags to organize your thoughts and ensure you've covered all necessary points:
   - The user's explicit requests and intents
   - Key technical concepts, code patterns, function signatures, and file edits
   - Errors that you ran into and how you fixed them
   - Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.

Your summary should include the following sections:

1. Primary Request and Intent: Capture all of the user's explicit requests and intents in detail
2. Key Technical Concepts: List all important technical concepts, technologies, and frameworks discussed.
3. Files and Code Sections: Enumerate specific files and code sections examined, modified, or created. Pay special attention to the most recent messages and include full code snippets where applicable and include a summary of why this file read or edit is important.
4. Errors and fixes: List all errors that you ran into, and how you fixed them. Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.
5. Problem Solving: Document problems solved and any ongoing troubleshooting efforts.
6. All user messages: List ALL user messages that are not tool results. These are critical for understanding the users' feedback and changing intent.
7. Pending Tasks: Outline any pending tasks that you have explicitly been asked to work on.
8. Current Work: Describe in detail precisely what was being worked on immediately before this summary request, paying special attention to the most recent messages from both user and assistant. Include file names and code snippets where applicable.
9. Optional Next Step: List the next step that you will take that is related to the most recent work you were doing. IMPORTANT: ensure that this step is DIRECTLY in line with the user's most recent explicit requests, and the task you were working on immediately before this summary request. If your last task was concluded, then only list next steps if they are explicitly in line with the user's request.

Structure your output as:

<analysis>
[Your thought process, ensuring all points are covered thoroughly and accurately]
</analysis>

<summary>
[The numbered sections described above]
</summary>

If additional summarization instructions are provided in the included context, follow them when creating the summary."#;

/// Renders the messages selected for compaction into a plain-text transcript
/// suitable for the summarization prompt.
///
/// Tail-biased truncation: messages are accumulated newest-first until the
/// character budget is exhausted, then re-ordered oldest-first. Low-signal
/// progress payloads are skipped entirely.
pub fn render_conversation_for_summary(messages: &[MessageEnvelope]) -> String {
    let mut entries: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut truncated = false;

    for message in messages.iter().rev() {
        let Some(entry) = render_message(message) else {
            continue;
        };
        if used + entry.len() > MAX_CONVERSATION_CHARS {
            truncated = true;
            break;
        }
        used += entry.len();
        entries.push(entry);
    }

    if truncated {
        entries.push(TRUNCATION_NOTICE.to_string());
    }
    entries.reverse();
    entries.join("\n\n")
}

/// Renders a single message as a role-tagged excerpt, or `None` for payloads
/// that carry no summarizable signal.
fn render_message(message: &MessageEnvelope) -> Option<String> {
    match &message.payload {
        MessagePayload::UserText { content } => Some(format!(
            "[user]\n{}",
            truncate(content, MESSAGE_EXCERPT_CHARS)
        )),
        MessagePayload::AssistantText { content } => Some(format!(
            "[assistant]\n{}",
            truncate(content, MESSAGE_EXCERPT_CHARS)
        )),
        MessagePayload::AssistantToolUse { tool, input, .. } => Some(format!(
            "[tool_use {tool}]\n{}",
            truncate(&input.to_string(), TOOL_OUTPUT_EXCERPT_CHARS)
        )),
        MessagePayload::ToolResult {
            tool,
            success,
            content,
            ..
        } => Some(format!(
            "[tool_result {tool} {}]\n{}",
            if *success { "ok" } else { "error" },
            truncate(content, TOOL_OUTPUT_EXCERPT_CHARS)
        )),
        MessagePayload::BashOutput {
            stdout,
            stderr,
            exit_code,
        } => {
            let mut body = String::new();
            if !stdout.is_empty() {
                body.push_str(truncate(stdout, TOOL_OUTPUT_EXCERPT_CHARS));
            }
            if !stderr.is_empty() {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str("stderr: ");
                body.push_str(truncate(stderr, TOOL_OUTPUT_EXCERPT_CHARS));
            }
            let code = exit_code.map_or_else(String::new, |c| format!(" exit={c}"));
            Some(format!("[bash_output{code}]\n{body}"))
        }
        MessagePayload::System { content } => Some(format!(
            "[system]\n{}",
            truncate(content, MESSAGE_EXCERPT_CHARS)
        )),
        MessagePayload::Command { input, output } => {
            let mut body = format!("[command]\n{input}");
            if let Some(output) = output {
                body.push('\n');
                body.push_str(truncate(output, TOOL_OUTPUT_EXCERPT_CHARS));
            }
            Some(body)
        }
        MessagePayload::CompactBoundary { summary } => Some(format!(
            "[earlier conversation summary]\n{}",
            truncate(summary, MESSAGE_EXCERPT_CHARS)
        )),
        MessagePayload::UserAttachment { label, uri } => {
            Some(format!("[user attachment] {label} ({uri})"))
        }
        MessagePayload::PlanApproval { summary, approved } => Some(format!(
            "[plan {}]\n{}",
            if *approved { "approved" } else { "rejected" },
            truncate(summary, MESSAGE_EXCERPT_CHARS)
        )),
        MessagePayload::ProviderError { kind, message } => {
            Some(format!("[provider error {kind}] {message}"))
        }
        // Thinking is internal and often huge; progress/hook/task/permission
        // payloads are transient UI signals with no continuation value.
        MessagePayload::AssistantThinking { .. }
        | MessagePayload::UserPasteReference { .. }
        | MessagePayload::Progress { .. }
        | MessagePayload::HookResult { .. }
        | MessagePayload::HookProgress { .. }
        | MessagePayload::Task { .. }
        | MessagePayload::Permission { .. }
        | MessagePayload::TaskNotification { .. } => None,
    }
}

/// Extracts the `<summary>` block from the model response, falling back to the
/// whole trimmed text when the model did not follow the output structure.
pub fn extract_summary_block(text: &str) -> &str {
    if let Some(start) = text.find("<summary>") {
        let body = &text[start + "<summary>".len()..];
        let end = body.find("</summary>").unwrap_or(body.len());
        return body[..end].trim();
    }
    text.trim()
}

/// Generates an LLM summary of `messages` via the configured provider.
///
/// Blocking call (ureq transport); in the TUI this runs inside
/// `spawn_blocking` like every other command. Returns an error when no
/// provider/credentials resolve or the model returns an empty summary — the
/// caller should fall back to a mechanical summary in that case.
pub fn generate_compact_summary(
    storage_dir: Option<&Path>,
    provider: Option<String>,
    model: Option<String>,
    messages: &[MessageEnvelope],
    custom_instructions: Option<&str>,
) -> Result<String> {
    if llm_summary_disabled() {
        return Err(WonderError::validation(format!(
            "LLM compaction summary disabled via {COMPACT_LLM_SUMMARY_ENV}"
        )));
    }
    let conversation = render_conversation_for_summary(messages);
    if conversation.trim().is_empty() {
        return Err(WonderError::validation(
            "no summarizable content in the messages selected for compaction",
        ));
    }

    let mut prompt = format!(
        "<conversation>\n{conversation}\n</conversation>\n\nPlease provide your summary of the conversation above."
    );
    if let Some(instructions) = custom_instructions {
        prompt.push_str("\n\n## Compact Instructions\n");
        prompt.push_str(instructions);
    }

    let runtime = ProviderRuntime::new();
    let selection = ProviderSelection::new(provider, model);
    let request = CompletionRequest {
        prompt,
        system_prompt: Some(COMPACT_SUMMARY_SYSTEM_PROMPT.into()),
        max_output_tokens: Some(MAX_SUMMARY_OUTPUT_TOKENS),
        temperature: Some(0.0),
        effort_level: None,
        images: Vec::new(),
    };

    let (_, response) = runtime.complete_with_storage(storage_dir, selection, &request)?;
    let summary = extract_summary_block(&response.output_text);
    if summary.is_empty() {
        return Err(WonderError::internal(
            "provider returned an empty compaction summary",
        ));
    }
    Ok(summary.to_string())
}

fn truncate(s: &str, max_chars: usize) -> &str {
    let mut idx = max_chars;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    if idx >= s.len() { s } else { &s[..idx] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonder_of_u_core::{SessionId, ToolUseId};

    fn msg(payload: MessagePayload) -> MessageEnvelope {
        MessageEnvelope::new(SessionId::new(), payload)
    }

    #[test]
    fn render_includes_user_and_assistant_text() {
        let messages = vec![
            msg(MessagePayload::UserText {
                content: "fix the login bug".into(),
            }),
            msg(MessagePayload::AssistantText {
                content: "found the issue in auth.rs".into(),
            }),
        ];
        let rendered = render_conversation_for_summary(&messages);
        assert!(rendered.contains("[user]\nfix the login bug"));
        assert!(rendered.contains("[assistant]\nfound the issue in auth.rs"));
    }

    #[test]
    fn render_skips_thinking_and_progress() {
        let messages = vec![
            msg(MessagePayload::AssistantThinking {
                content: "secret reasoning".into(),
                collapsed: true,
            }),
            msg(MessagePayload::Progress {
                label: "spinning".into(),
                detail: None,
            }),
            msg(MessagePayload::UserText {
                content: "hello".into(),
            }),
        ];
        let rendered = render_conversation_for_summary(&messages);
        assert!(!rendered.contains("secret reasoning"));
        assert!(!rendered.contains("spinning"));
        assert!(rendered.contains("hello"));
    }

    #[test]
    fn render_truncates_large_tool_results() {
        let messages = vec![msg(MessagePayload::ToolResult {
            tool: "file_read".into(),
            use_id: ToolUseId::new(),
            success: true,
            content: "x".repeat(10_000),
        })];
        let rendered = render_conversation_for_summary(&messages);
        assert!(rendered.len() < 1_000);
        assert!(rendered.contains("[tool_result file_read ok]"));
    }

    #[test]
    fn render_drops_oldest_when_over_budget() {
        let big = "y".repeat(MESSAGE_EXCERPT_CHARS);
        let count = MAX_CONVERSATION_CHARS / MESSAGE_EXCERPT_CHARS + 8;
        let mut messages: Vec<MessageEnvelope> = (0..count)
            .map(|_| {
                msg(MessagePayload::UserText {
                    content: big.clone(),
                })
            })
            .collect();
        messages.insert(
            0,
            msg(MessagePayload::UserText {
                content: "OLDEST-MARKER".into(),
            }),
        );
        messages.push(msg(MessagePayload::UserText {
            content: "NEWEST-MARKER".into(),
        }));

        let rendered = render_conversation_for_summary(&messages);
        assert!(rendered.contains("NEWEST-MARKER"));
        assert!(!rendered.contains("OLDEST-MARKER"));
        assert!(rendered.starts_with(TRUNCATION_NOTICE));
    }

    #[test]
    fn extract_summary_block_prefers_tagged_section() {
        let text = "<analysis>thinking</analysis>\n<summary>\n1. Did things\n</summary>";
        assert_eq!(extract_summary_block(text), "1. Did things");
    }

    #[test]
    fn extract_summary_block_falls_back_to_whole_text() {
        assert_eq!(extract_summary_block("  plain summary  "), "plain summary");
    }

    #[test]
    fn extract_summary_block_tolerates_missing_close_tag() {
        assert_eq!(
            extract_summary_block("<summary>unterminated"),
            "unterminated"
        );
    }

    #[test]
    fn generate_fails_on_empty_conversation() {
        let result = generate_compact_summary(None, None, None, &[], None);
        assert!(result.is_err());
    }
}
