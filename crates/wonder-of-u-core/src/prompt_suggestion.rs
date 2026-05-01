use serde::{Deserialize, Serialize};

use crate::{AppState, MessagePayload, QueryPhase, QueryState, TaskStatus};

/// Local prompt suggestion categories derived from session metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSuggestionKind {
    Confirm,
    Verify,
    Review,
    Commit,
    Try,
}

impl PromptSuggestionKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Verify => "verify",
            Self::Review => "review",
            Self::Commit => "commit",
            Self::Try => "try",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PromptSuggestion {
    pub text: String,
    pub kind: PromptSuggestionKind,
    pub score: u16,
    pub rationale: String,
}

impl PromptSuggestion {
    #[must_use]
    pub fn new(
        text: impl Into<String>,
        kind: PromptSuggestionKind,
        score: u16,
        rationale: impl Into<String>,
    ) -> Self {
        Self {
            text: text.into(),
            kind,
            score,
            rationale: rationale.into(),
        }
    }
}

#[must_use]
pub fn best_prompt_suggestion(app: &AppState, query: &QueryState) -> Option<PromptSuggestion> {
    rank_prompt_suggestions(app, query).into_iter().next()
}

#[must_use]
pub fn rank_prompt_suggestions(app: &AppState, query: &QueryState) -> Vec<PromptSuggestion> {
    if query.phase != QueryPhase::Completed
        || app.pending_tool_approval.is_some()
        || matches!(app.permission_mode, crate::PermissionMode::Plan)
    {
        return Vec::new();
    }

    let Some(last_assistant) = last_assistant_text(app) else {
        return Vec::new();
    };

    if has_recent_error(app) || sounds_like_error(last_assistant) {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    let lower = last_assistant.to_ascii_lowercase();
    let has_running_tasks = app
        .background_tasks
        .values()
        .any(|task| task.status == TaskStatus::Running);
    let has_completed_tasks = app
        .background_tasks
        .values()
        .any(|task| task.status == TaskStatus::Completed);
    let has_git_branch = app.session.git_branch.is_some();
    let changed_something = mentions_any(
        &lower,
        &[
            "fix",
            "fixed",
            "updated",
            "changed",
            "implemented",
            "added",
            "patched",
        ],
    ) || query.tool_calls > 0;

    if asks_for_confirmation(&lower) {
        candidates.push(PromptSuggestion::new(
            "yes",
            PromptSuggestionKind::Confirm,
            100,
            "assistant asked for confirmation",
        ));
    }

    if changed_something
        && mentions_any(
            &lower,
            &[
                "test",
                "tests",
                "verify",
                "verification",
                "haven't run",
                "did not run",
            ],
        )
    {
        candidates.push(PromptSuggestion::new(
            "run the tests",
            PromptSuggestionKind::Verify,
            92,
            "assistant described changes that still need verification",
        ));
    }

    if mentions_any(
        &lower,
        &[
            "diff",
            "patch",
            "changes",
            "changed files",
            "walk you through",
        ],
    ) || (changed_something && query.tool_roundtrips > 0)
    {
        candidates.push(PromptSuggestion::new(
            "show me the diff",
            PromptSuggestionKind::Review,
            78,
            "assistant referenced the patch or recent file changes",
        ));
    }

    if has_git_branch
        && !has_running_tasks
        && sounds_complete(&lower)
        && (changed_something || has_completed_tasks)
    {
        candidates.push(PromptSuggestion::new(
            "commit this",
            PromptSuggestionKind::Commit,
            if has_completed_tasks { 74 } else { 70 },
            "session looks ready for a local commit",
        ));
    }

    if changed_something {
        candidates.push(PromptSuggestion::new(
            "try it out",
            PromptSuggestionKind::Try,
            64,
            "assistant described an implementation that the user may want to exercise",
        ));
    }

    candidates.retain(|candidate| suggestion_filter_reason(&candidate.text).is_none());
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.text.cmp(&right.text))
    });
    candidates.dedup_by(|left, right| left.text == right.text);
    candidates
}

#[must_use]
pub fn suggestion_filter_reason(text: &str) -> Option<&'static str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some("empty");
    }

    let lower = trimmed.to_ascii_lowercase();
    let word_count = trimmed.split_whitespace().count();
    if lower == "done" {
        return Some("done");
    }
    if lower == "nothing found"
        || lower == "nothing found."
        || lower.starts_with("nothing to suggest")
        || lower.starts_with("no suggestion")
        || lower.contains("stay silent")
        || lower.contains("silence is")
    {
        return Some("meta_text");
    }
    if (trimmed.starts_with('(') && trimmed.ends_with(')'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        return Some("meta_wrapped");
    }
    if lower.starts_with("api error:")
        || lower.starts_with("prompt is too long")
        || lower.starts_with("request timed out")
        || lower.starts_with("invalid api key")
        || lower.starts_with("image was too large")
    {
        return Some("error_message");
    }
    if trimmed
        .split_once(':')
        .is_some_and(|(prefix, _)| prefix.chars().all(|ch| ch.is_alphanumeric() || ch == '_'))
    {
        return Some("prefixed_label");
    }
    if word_count < 2
        && !trimmed.starts_with('/')
        && !matches!(
            lower.as_str(),
            "yes"
                | "yeah"
                | "yep"
                | "yea"
                | "yup"
                | "sure"
                | "ok"
                | "okay"
                | "push"
                | "commit"
                | "deploy"
                | "stop"
                | "continue"
                | "check"
                | "exit"
                | "quit"
                | "no"
        )
    {
        return Some("too_few_words");
    }
    if word_count > 12 {
        return Some("too_many_words");
    }
    if trimmed.chars().count() >= 100 {
        return Some("too_long");
    }
    if has_multiple_sentences(trimmed) {
        return Some("multiple_sentences");
    }
    if trimmed.contains('\n') || trimmed.contains('*') {
        return Some("has_formatting");
    }
    if mentions_any(
        &lower,
        &[
            "thanks",
            "thank you",
            "looks good",
            "sounds good",
            "that works",
            "that worked",
            "that's all",
            "nice",
            "great",
            "perfect",
            "makes sense",
            "awesome",
            "excellent",
        ],
    ) {
        return Some("evaluative");
    }
    if starts_with_any(
        trimmed,
        &[
            "let me",
            "i'll",
            "i've",
            "i'm",
            "i can",
            "i would",
            "i think",
            "i notice",
            "here's",
            "here is",
            "here are",
            "that's",
            "this is",
            "this will",
            "you can",
            "you should",
            "you could",
            "sure,",
            "of course",
            "certainly",
        ],
    ) {
        return Some("claude_voice");
    }
    None
}

fn last_assistant_text(app: &AppState) -> Option<&str> {
    app.messages
        .iter()
        .rev()
        .find_map(|message| match &message.payload {
            MessagePayload::AssistantText { content } => Some(content.as_str()),
            _ => None,
        })
}

fn has_recent_error(app: &AppState) -> bool {
    app.messages
        .iter()
        .rev()
        .take(4)
        .any(|message| match &message.payload {
            MessagePayload::ToolResult {
                success, content, ..
            } => !success || sounds_like_error(content),
            MessagePayload::AssistantText { content } => sounds_like_error(content),
            _ => false,
        })
}

fn sounds_like_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    mentions_any(
        &lower,
        &[
            "error",
            "failed",
            "failure",
            "couldn't",
            "could not",
            "didn't work",
            "did not work",
            "timed out",
            "permission denied",
        ],
    )
}

fn asks_for_confirmation(text: &str) -> bool {
    mentions_any(
        text,
        &[
            "should i",
            "want me to",
            "would you like me to",
            "shall i",
            "go ahead?",
            "continue?",
        ],
    )
}

fn sounds_complete(text: &str) -> bool {
    mentions_any(
        text,
        &[
            "done",
            "complete",
            "completed",
            "finished",
            "ready",
            "all set",
            "fixed",
            "implemented",
        ],
    )
}

fn mentions_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn starts_with_any(text: &str, prefixes: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    prefixes.iter().any(|prefix| lower.starts_with(prefix))
}

fn has_multiple_sentences(text: &str) -> bool {
    let mut saw_terminal = false;
    for ch in text.chars() {
        if matches!(ch, '.' | '!' | '?') {
            saw_terminal = true;
            continue;
        }
        if saw_terminal && ch.is_ascii_uppercase() {
            return true;
        }
        if !ch.is_whitespace() {
            saw_terminal = false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{AppState, MessageEnvelope, MessagePayload, PermissionMode, QueryState, TaskState};

    use super::*;

    #[test]
    fn rank_prompt_suggestions_prefers_verification_after_fixing_work() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.session.git_branch = Some("feat/fix".into());
        app.push_message(MessageEnvelope::new(
            app.session.id,
            MessagePayload::UserText {
                content: "fix the failing tests".into(),
            },
        ))
        .expect("push user message");
        app.push_message(MessageEnvelope::new(
            app.session.id,
            MessagePayload::AssistantText {
                content: "I fixed the bug and updated the tests, but I haven't run them yet."
                    .into(),
            },
        ))
        .expect("push assistant message");

        let mut query = QueryState::start("fix the failing tests", crate::CoordinatorMode::Direct);
        query.record_tool_batch(["file_read", "file_write"]);
        query.complete();

        let suggestions = rank_prompt_suggestions(&app, &query);
        assert_eq!(suggestions[0].text, "run the tests");
        assert_eq!(suggestions[0].kind, PromptSuggestionKind::Verify);
        assert!(
            suggestions
                .iter()
                .any(|candidate| candidate.text == "show me the diff")
        );
        assert!(
            suggestions
                .iter()
                .any(|candidate| candidate.text == "commit this")
        );
    }

    #[test]
    fn prompt_suggestions_are_suppressed_for_plan_mode_and_recent_errors() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.permission_mode = PermissionMode::Plan;
        app.push_message(MessageEnvelope::new(
            app.session.id,
            MessagePayload::AssistantText {
                content: "I fixed it.".into(),
            },
        ))
        .expect("push assistant message");

        let mut query = QueryState::start("fix it", crate::CoordinatorMode::Direct);
        query.complete();
        assert!(rank_prompt_suggestions(&app, &query).is_empty());

        app.permission_mode = PermissionMode::Default;
        app.messages.clear();
        app.push_message(MessageEnvelope::new(
            app.session.id,
            MessagePayload::ToolResult {
                tool: "bash".into(),
                use_id: crate::ToolUseId::new(),
                success: false,
                content: "ERROR: tests failed".into(),
            },
        ))
        .expect("push tool result");
        app.push_message(MessageEnvelope::new(
            app.session.id,
            MessagePayload::AssistantText {
                content: "The test run failed.".into(),
            },
        ))
        .expect("push assistant message");
        assert!(rank_prompt_suggestions(&app, &query).is_empty());
    }

    #[test]
    fn suggestion_filter_reason_rejects_meta_and_evaluative_text() {
        assert_eq!(suggestion_filter_reason("looks good"), Some("evaluative"));
        assert_eq!(suggestion_filter_reason("(silence)"), Some("meta_wrapped"));
        assert_eq!(suggestion_filter_reason("done"), Some("done"));
        assert_eq!(suggestion_filter_reason("/help"), None);
        assert_eq!(suggestion_filter_reason("run the tests"), None);
    }

    #[test]
    fn completed_tasks_boost_commit_suggestions() {
        let mut app = AppState::new(PathBuf::from("/workspace"));
        app.session.git_branch = Some("feat/ready".into());
        let mut task = TaskState::pending("verify");
        task.mark_finished(TaskStatus::Completed, Some(0), Some("done".into()));
        app.upsert_task(task);
        app.push_message(MessageEnvelope::new(
            app.session.id,
            MessagePayload::AssistantText {
                content: "The implementation is complete and ready.".into(),
            },
        ))
        .expect("push assistant message");

        let mut query = QueryState::start("ship it", crate::CoordinatorMode::Direct);
        query.complete();

        let suggestions = rank_prompt_suggestions(&app, &query);
        assert!(suggestions.iter().any(|candidate| {
            candidate.text == "commit this" && candidate.kind == PromptSuggestionKind::Commit
        }));
    }
}
