//! SDK-style `<task-notification>` XML payload builder.
//!
//! This module provides a focused builder for constructing well-formed XML
//! payloads that describe the terminal state of a background task.  The
//! payload is suitable for embedding in tool-result transcripts or shipping
//! as a structured side-channel message.
//!
//! # Examples
//!
//! ```
//! use wonder_of_u_core::task_notification::TaskNotificationPayload;
//!
//! let xml = TaskNotificationPayload::new("task-123", "completed")
//!     .summary("Built and tested the workspace")
//!     .render_xml();
//!
//! assert!(xml.contains("<task-id>task-123</task-id>"));
//! assert!(xml.contains("<status>completed</status>"));
//! ```

use std::path::Path;

use crate::{TaskId, TokenUsage, ToolUseId};

// Minimal XML character escaping for text content and attribute values.
// We handle the five characters that XML requires.
fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Builder for a `<task-notification>` XML payload.
///
/// All setter methods follow a consuming-builder pattern and return `Self` so
/// calls can be chained.  Call [`render_xml`](Self::render_xml) to produce the
/// final string.
///
/// # Required fields
/// - `task_id` – opaque identifier for the task (e.g. a UUID string or a
///   `todo-*` id).
/// - `status` – terminal status label such as `"completed"` or `"failed"`.
///
/// # Optional fields
/// Any field not set is simply omitted from the rendered XML, keeping payloads
/// compact.
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct TaskNotificationPayload {
    task_id: String,
    tool_use_id: Option<String>,
    output_file: Option<String>,
    status: String,
    summary: Option<String>,
    result: Option<String>,
    usage: Option<TokenUsage>,
    worktree_path: Option<String>,
    worktree_branch: Option<String>,
}

impl TaskNotificationPayload {
    /// Creates a minimal payload with the two required fields.
    pub fn new(task_id: impl Into<String>, status: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            status: status.into(),
            ..Default::default()
        }
    }

    /// Convenience constructor that accepts typed [`TaskId`] and status string.
    pub fn from_task_id(id: TaskId, status: impl Into<String>) -> Self {
        Self::new(id.to_string(), status)
    }

    /// Attaches a [`ToolUseId`] that correlates this notification back to the
    /// tool-use request that spawned the task.
    pub fn tool_use_id(mut self, id: ToolUseId) -> Self {
        self.tool_use_id = Some(id.to_string());
        self
    }

    /// Sets the tool use id from a raw string (useful when the id is already
    /// serialised, e.g. from a deserialized transcript).
    pub fn tool_use_id_str(mut self, id: impl Into<String>) -> Self {
        self.tool_use_id = Some(id.into());
        self
    }

    /// Path of the output / log file produced by the task.
    pub fn output_file(mut self, path: &Path) -> Self {
        self.output_file = Some(path.display().to_string());
        self
    }

    /// Path of the output / log file from a raw string.
    pub fn output_file_str(mut self, path: impl Into<String>) -> Self {
        self.output_file = Some(path.into());
        self
    }

    /// Human-readable summary of what the task did (e.g. description or
    /// status message).
    pub fn summary(mut self, text: impl Into<String>) -> Self {
        self.summary = Some(text.into());
        self
    }

    /// Final result detail – e.g. exit code text or a short outcome sentence.
    pub fn result(mut self, text: impl Into<String>) -> Self {
        self.result = Some(text.into());
        self
    }

    /// Token usage counters recorded for this task.
    pub fn usage(mut self, usage: TokenUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Absolute path of the git worktree the task ran inside.
    pub fn worktree_path(mut self, path: &Path) -> Self {
        self.worktree_path = Some(path.display().to_string());
        self
    }

    /// Absolute path of the git worktree from a raw string.
    pub fn worktree_path_str(mut self, path: impl Into<String>) -> Self {
        self.worktree_path = Some(path.into());
        self
    }

    /// Branch the task's worktree was checked out on.
    pub fn worktree_branch(mut self, branch: impl Into<String>) -> Self {
        self.worktree_branch = Some(branch.into());
        self
    }

    /// Renders the payload as a self-contained `<task-notification>` XML
    /// string.  All text content is XML-escaped.
    ///
    /// # Output shape
    ///
    /// ```xml
    /// <task-notification>
    ///   <task-id>…</task-id>
    ///   <!-- optional elements omitted when not set -->
    ///   <tool-use-id>…</tool-use-id>
    ///   <output-file>…</output-file>
    ///   <status>…</status>
    ///   <summary>…</summary>
    ///   <result>…</result>
    ///   <usage input="…" output="…" cache-creation="…" cache-read="…"/>
    ///   <worktree>
    ///     <path>…</path>
    ///     <branch>…</branch>
    ///   </worktree>
    /// </task-notification>
    /// ```
    #[must_use]
    pub fn render_xml(&self) -> String {
        let mut buf = String::from("<task-notification>\n");

        push_elem(&mut buf, "task-id", &self.task_id);

        if let Some(id) = &self.tool_use_id {
            push_elem(&mut buf, "tool-use-id", id);
        }
        if let Some(path) = &self.output_file {
            push_elem(&mut buf, "output-file", path);
        }

        push_elem(&mut buf, "status", &self.status);

        if let Some(summary) = &self.summary {
            push_elem(&mut buf, "summary", summary);
        }
        if let Some(result) = &self.result {
            push_elem(&mut buf, "result", result);
        }

        if let Some(usage) = &self.usage {
            buf.push_str(&format!(
                "  <usage input=\"{}\" output=\"{}\" cache-creation=\"{}\" cache-read=\"{}\"/>\n",
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_creation_tokens,
                usage.cache_read_tokens,
            ));
        }

        let has_worktree = self.worktree_path.is_some() || self.worktree_branch.is_some();
        if has_worktree {
            buf.push_str("  <worktree>\n");
            if let Some(path) = &self.worktree_path {
                push_nested_elem(&mut buf, "path", path);
            }
            if let Some(branch) = &self.worktree_branch {
                push_nested_elem(&mut buf, "branch", branch);
            }
            buf.push_str("  </worktree>\n");
        }

        buf.push_str("</task-notification>");
        buf
    }
}

// Appends `  <tag>escaped_content</tag>\n`.
fn push_elem(buf: &mut String, tag: &str, content: &str) {
    buf.push_str(&format!("  <{tag}>{}</{tag}>\n", xml_escape(content)));
}

// Appends `    <tag>escaped_content</tag>\n` (extra indent for nested elements).
fn push_nested_elem(buf: &mut String, tag: &str, content: &str) {
    buf.push_str(&format!("    <{tag}>{}</{tag}>\n", xml_escape(content)));
}

/// Builds a [`TaskNotificationPayload`] from a [`crate::TaskState`] reference.
///
/// Fields are populated from the task where available:
/// - `task_id` from `task.id`
/// - `status` from the task status label
/// - `summary` from `task.description` + optional `task.status_message`
/// - `result` from `task.exit_code` if present
/// - `output_file` from `task.output_log` if present
/// - `worktree_branch` from `task.worktree_branch` if present
pub fn payload_from_task_state(task: &crate::TaskState) -> TaskNotificationPayload {
    let status = task_status_label(task.status);

    let summary = match &task.status_message {
        Some(msg) if !msg.is_empty() => {
            format!("{} — {}", task.description, msg)
        }
        _ => task.description.clone(),
    };

    let mut payload = TaskNotificationPayload::new(task.id.to_string(), status).summary(summary);

    if let Some(exit_code) = task.exit_code {
        payload = payload.result(format!("exit code: {exit_code}"));
    }

    if let Some(log_path) = &task.output_log {
        payload = payload.output_file(log_path);
    }

    if let Some(branch) = &task.worktree_branch {
        payload = payload.worktree_branch(branch.clone());
    }

    payload
}

fn task_status_label(status: crate::TaskStatus) -> &'static str {
    use crate::TaskStatus;
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Killed => "killed",
        TaskStatus::Cancelled => "cancelled",
    }
}

/// Returns the plain-text lines that the TUI notification dialog displays for
/// a task, alongside the XML payload for structured consumers.
///
/// The plain-text lines are identical to what the TUI already shows; the XML
/// is an additive companion.  This helper is intentionally kept pure so it can
/// be called from both TUI code and tests without pulling in ratatui.
pub fn task_notification_both(task: &crate::TaskState) -> (Vec<String>, TaskNotificationPayload) {
    let mut lines = vec![format!(
        "[{}] {}",
        task_status_label(task.status),
        task.description
    )];
    if let Some(msg) = task.status_message.as_deref().filter(|v| !v.is_empty()) {
        lines.push(msg.to_string());
    }
    if let Some(code) = task.exit_code {
        lines.push(format!("exit code: {code}"));
    }
    let payload = payload_from_task_state(task);
    (lines, payload)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{TaskId, TaskState, TaskStatus, TokenUsage, ToolUseId};

    // ── xml_escape ─────────────────────────────────────────────────────────

    #[test]
    fn xml_escape_handles_all_five_special_chars() {
        assert_eq!(
            xml_escape(r#"<tag attr="x" key='y'>a & b</tag>"#),
            "&lt;tag attr=&quot;x&quot; key=&apos;y&apos;&gt;a &amp; b&lt;/tag&gt;"
        );
    }

    #[test]
    fn xml_escape_passthrough_for_plain_text() {
        let plain = "Hello, world! 1+2=3";
        assert_eq!(xml_escape(plain), plain);
    }

    #[test]
    fn xml_escape_empty_string() {
        assert_eq!(xml_escape(""), "");
    }

    // ── required fields only ───────────────────────────────────────────────

    #[test]
    fn render_xml_minimal_contains_task_id_and_status() {
        let xml = TaskNotificationPayload::new("task-abc", "completed").render_xml();
        assert!(
            xml.contains("<task-id>task-abc</task-id>"),
            "task-id missing"
        );
        assert!(xml.contains("<status>completed</status>"), "status missing");
        assert!(xml.starts_with("<task-notification>"), "wrong root element");
        assert!(xml.ends_with("</task-notification>"), "unclosed root");
    }

    #[test]
    fn render_xml_minimal_omits_optional_elements() {
        let xml = TaskNotificationPayload::new("t1", "failed").render_xml();
        assert!(
            !xml.contains("<tool-use-id>"),
            "tool-use-id should be absent"
        );
        assert!(
            !xml.contains("<output-file>"),
            "output-file should be absent"
        );
        assert!(!xml.contains("<summary>"), "summary should be absent");
        assert!(!xml.contains("<result>"), "result should be absent");
        assert!(!xml.contains("<usage"), "usage should be absent");
        assert!(!xml.contains("<worktree>"), "worktree should be absent");
    }

    // ── optional fields ────────────────────────────────────────────────────

    #[test]
    fn render_xml_with_tool_use_id() {
        let id = ToolUseId::new();
        let xml = TaskNotificationPayload::new("t1", "completed")
            .tool_use_id(id)
            .render_xml();
        assert!(
            xml.contains(&format!("<tool-use-id>{id}</tool-use-id>")),
            "tool-use-id tag missing or wrong"
        );
    }

    #[test]
    fn render_xml_with_output_file() {
        let path = PathBuf::from("/workspace/task-output.log");
        let xml = TaskNotificationPayload::new("t1", "completed")
            .output_file(&path)
            .render_xml();
        assert!(
            xml.contains("<output-file>/workspace/task-output.log</output-file>"),
            "output-file tag missing"
        );
    }

    #[test]
    fn render_xml_output_file_path_is_escaped() {
        // Path containing characters that require XML escaping.
        let xml = TaskNotificationPayload::new("t1", "completed")
            .output_file_str("/tmp/out&put<file>.log")
            .render_xml();
        assert!(
            xml.contains("<output-file>/tmp/out&amp;put&lt;file&gt;.log</output-file>"),
            "output-file content not escaped: {xml}"
        );
    }

    #[test]
    fn render_xml_with_summary_and_result() {
        let xml = TaskNotificationPayload::new("t2", "failed")
            .summary("Ran cargo test")
            .result("exit code: 1")
            .render_xml();
        assert!(xml.contains("<summary>Ran cargo test</summary>"));
        assert!(xml.contains("<result>exit code: 1</result>"));
    }

    #[test]
    fn render_xml_summary_is_xml_escaped() {
        let xml = TaskNotificationPayload::new("t3", "completed")
            .summary("Fixed <bug> & improved 'tests'")
            .render_xml();
        assert!(
            xml.contains("<summary>Fixed &lt;bug&gt; &amp; improved &apos;tests&apos;</summary>"),
            "summary not escaped: {xml}"
        );
    }

    #[test]
    fn render_xml_with_usage() {
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 200,
            cache_creation_tokens: 10,
            cache_read_tokens: 50,
        };
        let xml = TaskNotificationPayload::new("t4", "completed")
            .usage(usage)
            .render_xml();
        assert!(
            xml.contains(
                r#"<usage input="100" output="200" cache-creation="10" cache-read="50"/>"#
            ),
            "usage element missing or malformed: {xml}"
        );
    }

    // ── worktree tags ──────────────────────────────────────────────────────

    #[test]
    fn render_xml_worktree_path_only() {
        let xml = TaskNotificationPayload::new("t5", "completed")
            .worktree_path_str("/tmp/wt/feat-branch")
            .render_xml();
        assert!(xml.contains("<worktree>"), "worktree open tag missing");
        assert!(
            xml.contains("<path>/tmp/wt/feat-branch</path>"),
            "path tag missing"
        );
        assert!(
            !xml.contains("<branch>"),
            "branch tag should be absent when not set"
        );
        assert!(xml.contains("</worktree>"), "worktree close tag missing");
    }

    #[test]
    fn render_xml_worktree_branch_only() {
        let xml = TaskNotificationPayload::new("t6", "completed")
            .worktree_branch("feat/my-feature")
            .render_xml();
        assert!(xml.contains("<worktree>"));
        assert!(xml.contains("<branch>feat/my-feature</branch>"));
        assert!(!xml.contains("<path>"));
    }

    #[test]
    fn render_xml_worktree_path_and_branch() {
        let path = PathBuf::from("/workspace/worktrees/feat");
        let xml = TaskNotificationPayload::new("t7", "completed")
            .worktree_path(&path)
            .worktree_branch("feat/xml-payloads")
            .render_xml();
        assert!(xml.contains("<path>/workspace/worktrees/feat</path>"));
        assert!(xml.contains("<branch>feat/xml-payloads</branch>"));
    }

    #[test]
    fn render_xml_worktree_branch_is_xml_escaped() {
        let xml = TaskNotificationPayload::new("t8", "completed")
            .worktree_branch("fix/handle-<edge>&case")
            .render_xml();
        assert!(
            xml.contains("<branch>fix/handle-&lt;edge&gt;&amp;case</branch>"),
            "branch not escaped: {xml}"
        );
    }

    // ── from_task_id constructor ───────────────────────────────────────────

    #[test]
    fn from_task_id_uses_typed_id() {
        let id = TaskId::new();
        let xml = TaskNotificationPayload::from_task_id(id, "completed").render_xml();
        assert!(
            xml.contains(&format!("<task-id>{id}</task-id>")),
            "typed TaskId not rendered"
        );
    }

    // ── payload_from_task_state ─────────────────────────────────────────────

    #[test]
    fn payload_from_task_state_minimal() {
        let task = TaskState::pending("run unit tests");
        let xml = payload_from_task_state(&task).render_xml();
        assert!(xml.contains(&format!("<task-id>{}</task-id>", task.id)));
        assert!(xml.contains("<status>pending</status>"));
        assert!(xml.contains("<summary>run unit tests</summary>"));
    }

    #[test]
    fn payload_from_task_state_with_exit_code() {
        let mut task = TaskState::pending_shell("build", "cargo build", "/workspace");
        task.status = TaskStatus::Failed;
        task.exit_code = Some(101);
        let xml = payload_from_task_state(&task).render_xml();
        assert!(xml.contains("<status>failed</status>"));
        assert!(xml.contains("<result>exit code: 101</result>"));
    }

    #[test]
    fn payload_from_task_state_with_output_log() {
        let mut task = TaskState::pending("archive logs");
        task.output_log = Some(PathBuf::from("/var/log/task-42.log"));
        let xml = payload_from_task_state(&task).render_xml();
        assert!(xml.contains("<output-file>/var/log/task-42.log</output-file>"));
    }

    #[test]
    fn payload_from_task_state_with_worktree_branch() {
        let mut task = TaskState::pending("run agent");
        task.worktree_branch = Some("feat/worktree-isolation".into());
        let xml = payload_from_task_state(&task).render_xml();
        assert!(xml.contains("<branch>feat/worktree-isolation</branch>"));
    }

    #[test]
    fn payload_from_task_state_merges_status_message_into_summary() {
        let mut task = TaskState::pending("compile project");
        task.status_message = Some("linker error in main.rs".into());
        let xml = payload_from_task_state(&task).render_xml();
        // Summary should contain both description and status message.
        assert!(xml.contains("compile project"));
        assert!(xml.contains("linker error in main.rs"));
    }

    // ── task_notification_both ─────────────────────────────────────────────

    #[test]
    fn task_notification_both_plain_lines_match_existing_behavior() {
        let mut task = TaskState::pending("deploy service");
        task.status = TaskStatus::Completed;
        task.status_message = Some("deployed to prod".into());
        task.exit_code = Some(0);

        let (lines, _xml_payload) = task_notification_both(&task);

        // Line 0: "[completed] deploy service"
        assert_eq!(lines[0], "[completed] deploy service");
        // Line 1: status message
        assert_eq!(lines[1], "deployed to prod");
        // Line 2: exit code
        assert_eq!(lines[2], "exit code: 0");
    }

    #[test]
    fn task_notification_both_xml_is_consistent_with_lines() {
        let mut task = TaskState::pending("run linter");
        task.status = TaskStatus::Failed;
        task.exit_code = Some(2);

        let (lines, payload) = task_notification_both(&task);
        let xml = payload.render_xml();

        // Status must match between plain text and XML.
        assert!(lines[0].contains("failed"));
        assert!(xml.contains("<status>failed</status>"));
        // Exit code appears in lines and in XML result tag.
        assert!(lines.iter().any(|l| l.contains("exit code: 2")));
        assert!(xml.contains("<result>exit code: 2</result>"));
    }

    #[test]
    fn task_notification_both_omits_empty_status_message_from_lines() {
        let mut task = TaskState::pending("silent task");
        task.status = TaskStatus::Completed;
        task.status_message = Some(String::new()); // empty — must be suppressed

        let (lines, _) = task_notification_both(&task);
        assert_eq!(lines.len(), 1, "empty status_message must not add a line");
    }
}
