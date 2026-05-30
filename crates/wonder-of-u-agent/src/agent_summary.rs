//! Background agent-summary generator.
//!
//! Every 30 seconds while an agent subprocess is running, the parent process
//! reads the agent's JSONL transcript and calls the provider API to produce a
//! short (3-5 word) present-tense label describing what the agent is currently
//! doing (e.g. "Reading runAgent.ts", "Fixing null check in validate.ts").
//!
//! The result is written back to [`TaskState::agent_summary`] so the TUI can
//! surface it alongside the task list without needing a separate polling loop.

use std::path::Path;

use wonder_of_u_core::{MessagePayload, SessionId};
use wonder_of_u_storage::TranscriptStore;

use crate::{CompletionRequest, ProviderRuntime, ProviderSelection};

/// Minimum number of messages in a transcript before we bother summarising.
const MIN_MESSAGES: usize = 3;

/// Maximum number of (most-recent) messages to include in the summary prompt.
const MAX_MESSAGES: usize = 20;

/// Maximum output tokens for the summary response.
const MAX_SUMMARY_TOKENS: u32 = 50;

/// Builds the one-shot prompt that asks the model for a 3-5 word activity label.
fn build_summary_prompt(transcript_excerpt: &str, previous_summary: Option<&str>) -> String {
    let previous_line = match previous_summary {
        Some(s) if !s.is_empty() => format!(
            "\nPrevious summary: {s}\nUpdate it only if the activity has meaningfully changed."
        ),
        _ => String::new(),
    };

    format!(
        "You are summarising what an AI agent is currently doing based on its recent \
conversation transcript.\n\
Respond with ONLY a 3-5 word present-tense action phrase, no punctuation, no quotes.\n\
Examples: \"Reading runAgent.ts\", \"Fixing null check in validate.ts\", \
\"Writing unit tests for parser\"\n\
{previous_line}\n\
Transcript (most recent messages):\n\
{transcript_excerpt}"
    )
}

/// Extracts a compact text representation of the most-recent transcript messages,
/// stripping large tool-result payloads to keep the prompt small.
fn extract_transcript_excerpt(transcript: &[wonder_of_u_core::MessageEnvelope]) -> String {
    // Take up to MAX_MESSAGES from the tail, skipping ToolResult payloads
    // (they are often large and noisy).
    let relevant: Vec<&wonder_of_u_core::MessageEnvelope> = transcript
        .iter()
        .rev()
        .filter(|msg| {
            !matches!(
                &msg.payload,
                MessagePayload::ToolResult { .. } | MessagePayload::BashOutput { .. }
            )
        })
        .take(MAX_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    relevant
        .iter()
        .filter_map(|msg| match &msg.payload {
            MessagePayload::UserText { content } => {
                Some(format!("User: {}", truncate(content, 200)))
            }
            MessagePayload::AssistantText { content } => {
                Some(format!("Assistant: {}", truncate(content, 300)))
            }
            MessagePayload::AssistantToolUse { tool, input, .. } => Some(format!(
                "Tool call: {} {}",
                tool,
                truncate(&input.to_string(), 120)
            )),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate(s: &str, max_chars: usize) -> &str {
    let mut idx = max_chars;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    if idx >= s.len() { s } else { &s[..idx] }
}

/// Generates a short progress description for a running agent subprocess.
///
/// Reads the session transcript from `storage_dir`, builds a summary prompt,
/// calls the provider API (blocking), and returns the trimmed response text.
///
/// Returns `None` on any error or when the transcript is too short — the caller
/// should keep the previous summary in that case.
pub fn generate_agent_summary(
    storage_dir: &Path,
    session_id: SessionId,
    provider: Option<String>,
    model: Option<String>,
    previous_summary: Option<&str>,
) -> Option<String> {
    let store = TranscriptStore::new(storage_dir);
    let loaded = match store.load_session(session_id) {
        Ok(t) => t,
        Err(_) => return None,
    };

    if loaded.messages.len() < MIN_MESSAGES {
        return None;
    }

    let excerpt = extract_transcript_excerpt(&loaded.messages);
    if excerpt.trim().is_empty() {
        return None;
    }

    let prompt = build_summary_prompt(&excerpt, previous_summary);
    let runtime = ProviderRuntime::new();
    let selection = ProviderSelection::new(provider, model);
    let request = CompletionRequest {
        prompt,
        system_prompt: None,
        max_output_tokens: Some(MAX_SUMMARY_TOKENS),
        temperature: Some(0.0),
        effort_level: None,
    };

    match runtime.complete_with_storage(Some(storage_dir), selection, &request) {
        Ok((_, response)) => {
            let text = response.output_text.trim().to_string();
            if text.is_empty() { None } else { Some(text) }
        }
        Err(_) => None,
    }
}

/// Reads the session-link sidecar for `task_id` and returns the `SessionId`
/// it contains, or `None` if the file is absent or malformed.
pub fn read_session_link(
    storage_dir: &Path,
    task_id: wonder_of_u_core::TaskId,
) -> Option<SessionId> {
    let path = wonder_of_u_storage::StoragePaths::new(storage_dir).task_session_link_path(task_id);
    let raw = std::fs::read_to_string(&path).ok()?;
    raw.trim().parse::<SessionId>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonder_of_u_core::{MessageEnvelope, MessagePayload, SessionId, ToolUseId};

    fn make_msg(session_id: SessionId, payload: MessagePayload) -> MessageEnvelope {
        MessageEnvelope::new(session_id, payload)
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "hello world";
        assert_eq!(truncate(s, 5), "hello");
        assert_eq!(truncate(s, 100), "hello world");
    }

    #[test]
    fn extract_excerpt_filters_tool_results() {
        let session_id = SessionId::new();

        let msgs = vec![
            make_msg(
                session_id,
                MessagePayload::UserText {
                    content: "Hello".into(),
                },
            ),
            make_msg(
                session_id,
                MessagePayload::ToolResult {
                    tool: "file_read".into(),
                    use_id: ToolUseId::new(),
                    success: true,
                    content: "big tool output".into(),
                },
            ),
            make_msg(
                session_id,
                MessagePayload::AssistantText {
                    content: "Done".into(),
                },
            ),
        ];

        let excerpt = extract_transcript_excerpt(&msgs);
        assert!(excerpt.contains("Hello"), "user text missing");
        assert!(excerpt.contains("Done"), "assistant text missing");
        assert!(
            !excerpt.contains("big tool output"),
            "tool result should be filtered"
        );
    }

    #[test]
    fn build_summary_prompt_includes_previous_summary() {
        let prompt = build_summary_prompt("User: hi\nAssistant: hello", Some("Greeting user"));
        assert!(prompt.contains("Greeting user"));
    }

    #[test]
    fn build_summary_prompt_no_previous() {
        let prompt = build_summary_prompt("User: hi\nAssistant: hello", None);
        assert!(!prompt.contains("Previous summary"));
    }
}
