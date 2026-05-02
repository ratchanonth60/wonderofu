//! Renderer-agnostic permission request summaries for the TUI.
//!
//! These views intentionally keep only safe, structured fields that are useful
//! for terminal rendering. Permission decisions stay in core and CLI layers.

use std::{
    collections::VecDeque,
    path::{Component, Path, PathBuf},
};

use serde_json::Value;
use wonder_of_u_core::PermissionRequest;

use crate::dialog::{DialogActionView, DialogView};

const MAX_PATH_CHARS: usize = 48;
const MAX_TEXT_CHARS: usize = 72;

/// Describes the overall risk level of a permission request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionAccessKind {
    /// Represents read only
    ReadOnly,
    /// Represents standard
    Standard,
    /// Represents destructive
    Destructive,
}

impl PermissionAccessKind {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Standard => "changes allowed",
            Self::Destructive => "destructive",
        }
    }
}

/// A single safe detail extracted from a permission request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionDetailView {
    /// Stores the label
    pub label: String,
    /// Stores the value
    pub value: String,
}

impl PermissionDetailView {
    /// Creates a new value
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// A structured, renderer-neutral permission request summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionSummaryView {
    /// Stores the title
    pub title: String,
    /// Stores the prompt
    pub prompt: String,
    /// Stores the access
    pub access: PermissionAccessKind,
    /// Stores the details
    pub details: Vec<PermissionDetailView>,
    /// Stores the actions
    pub actions: Vec<DialogActionView>,
}

impl PermissionSummaryView {
    /// Builds a safe summary from a permission request and raw tool input.
    #[must_use]
    pub fn from_request(request: &PermissionRequest, input: &Value) -> Self {
        let tool_name = request.tool_name.trim();
        let normalized = tool_name.to_ascii_lowercase();
        let (title, prompt) = tool_copy(&normalized, tool_name);
        let access = access_kind(request);
        let details = detail_lines(&normalized, request, input);

        Self {
            title,
            prompt,
            access,
            details,
            actions: vec![
                DialogActionView::new("Allow", true),
                DialogActionView::new("Deny", false),
            ],
        }
    }
    /// Handles action hint
    #[must_use]
    pub fn action_hint(&self) -> String {
        self.actions
            .iter()
            .map(|action| {
                if action.primary {
                    format!("[{}]", action.label)
                } else {
                    action.label.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
    }
    /// Handles to dialog view
    #[must_use]
    pub fn to_dialog_view(&self) -> DialogView {
        let mut body = Vec::with_capacity(self.details.len() + 3);
        body.push(self.prompt.clone());
        body.push(format!("Access: {}", self.access.label()));
        body.extend(
            self.details
                .iter()
                .map(|detail| format!("{}: {}", detail.label, detail.value)),
        );
        body.push("Allow to continue, or deny to continue without running it.".into());

        DialogView {
            title: self.title.clone(),
            body,
            actions: self.actions.clone(),
        }
    }
}

fn access_kind(request: &PermissionRequest) -> PermissionAccessKind {
    if request.destructive {
        PermissionAccessKind::Destructive
    } else if request.read_only {
        PermissionAccessKind::ReadOnly
    } else {
        PermissionAccessKind::Standard
    }
}

fn tool_copy(normalized: &str, tool_name: &str) -> (String, String) {
    match normalized {
        "bash" => (
            "Run shell command".into(),
            "Allow Claude to run this shell command?".into(),
        ),
        "powershell" => (
            "Run PowerShell command".into(),
            "Allow Claude to run this PowerShell command?".into(),
        ),
        "file_read" => ("Read file".into(), "Allow Claude to read this file?".into()),
        "file_write" => (
            "Write file".into(),
            "Allow Claude to write this file?".into(),
        ),
        "file_edit" => ("Edit file".into(), "Allow Claude to edit this file?".into()),
        "notebook_edit" => (
            "Edit notebook".into(),
            "Allow Claude to edit this notebook?".into(),
        ),
        "web_fetch" => (
            "Fetch web content".into(),
            "Allow Claude to fetch this content?".into(),
        ),
        "web_search" => (
            "Search the web".into(),
            "Allow Claude to run this web search?".into(),
        ),
        "ask_user" => (
            "Ask user question".into(),
            "Allow Claude to ask the user this question?".into(),
        ),
        "skill" => ("Run skill".into(), "Allow Claude to run this skill?".into()),
        "plan_read" => (
            "Read plan file".into(),
            "Allow Claude to read the plan file?".into(),
        ),
        "plan_write" => (
            "Write plan file".into(),
            "Allow Claude to update the plan file?".into(),
        ),
        "enter_plan_mode" => (
            "Enter plan mode".into(),
            "Allow Claude to enter plan mode?".into(),
        ),
        "mcp_resource_list" => (
            "List MCP resources".into(),
            "Allow Claude to list MCP resources?".into(),
        ),
        "mcp_resource_read" => (
            "Read MCP resource".into(),
            "Allow Claude to read this MCP resource?".into(),
        ),
        "sandbox" => (
            "Network request outside sandbox".into(),
            "Allow this network request outside the sandbox?".into(),
        ),
        _ => (
            format!("Use tool `{tool_name}`"),
            format!("Allow Claude to use `{tool_name}`?"),
        ),
    }
}

fn detail_lines(
    normalized: &str,
    request: &PermissionRequest,
    input: &Value,
) -> Vec<PermissionDetailView> {
    let mut details = Vec::new();

    match normalized {
        "bash" | "powershell" => {
            if let Some(command) = shell_command(request, input) {
                details.push(PermissionDetailView::new("Command", command));
            }
            if let Some(cwd) = first_path(input).or_else(|| request.paths.first().cloned()) {
                details.push(PermissionDetailView::new(
                    "Directory",
                    truncate_path(&cwd, MAX_PATH_CHARS),
                ));
            }
        }
        "file_read" | "file_write" | "file_edit" | "notebook_edit" | "plan_read" | "plan_write" => {
            push_path_details(&mut details, input_paths(input, request));
        }
        "web_fetch" => {
            if let Some(url) = string_field(input, &["url"]) {
                details.push(PermissionDetailView::new(
                    "URL",
                    sanitize_inline_text(&url, MAX_TEXT_CHARS),
                ));
            }
        }
        "web_search" => {
            if let Some(query) = string_field(input, &["query"]) {
                details.push(PermissionDetailView::new(
                    "Query",
                    sanitize_inline_text(&query, MAX_TEXT_CHARS),
                ));
            }
        }
        "ask_user" => {
            if let Some(question) = string_field(input, &["question"]) {
                details.push(PermissionDetailView::new(
                    "Question",
                    sanitize_inline_text(&question, MAX_TEXT_CHARS),
                ));
            }
            if let Some(options) = string_array_field(input, &["options"]) {
                details.push(PermissionDetailView::new(
                    "Options",
                    format!("{} option{}", options.len(), plural_suffix(options.len())),
                ));
            }
        }
        "skill" => {
            if let Some(skill) = string_field(input, &["skill", "name", "command"]) {
                details.push(PermissionDetailView::new(
                    "Skill",
                    sanitize_inline_text(&skill, MAX_TEXT_CHARS),
                ));
            }
        }
        "mcp_resource_list" => {
            if let Some(server) = string_field(input, &["server"]) {
                details.push(PermissionDetailView::new(
                    "Server",
                    sanitize_inline_text(&server, MAX_TEXT_CHARS),
                ));
            }
        }
        "mcp_resource_read" => {
            if let Some(resource) = string_field(input, &["resource_name"]) {
                details.push(PermissionDetailView::new(
                    "Resource",
                    sanitize_inline_text(&resource, MAX_TEXT_CHARS),
                ));
            }
        }
        "sandbox" => {
            if let Some(host) = string_field(input, &["host", "hostname", "domain"]) {
                details.push(PermissionDetailView::new(
                    "Host",
                    sanitize_inline_text(&host, MAX_TEXT_CHARS),
                ));
            }
        }
        _ => {
            if let Some(command) = shell_command(request, input) {
                details.push(PermissionDetailView::new("Command", command));
            } else {
                push_path_details(&mut details, input_paths(input, request));
            }
        }
    }

    details
}

fn push_path_details(details: &mut Vec<PermissionDetailView>, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }

    let label = if paths.len() == 1 { "Path" } else { "Paths" };
    details.push(PermissionDetailView::new(label, format_path_list(&paths)));
}

fn shell_command(request: &PermissionRequest, input: &Value) -> Option<String> {
    request
        .shell_command
        .clone()
        .or_else(|| string_field(input, &["command", "cmd", "script"]))
        .map(|command| sanitize_inline_text(&command, MAX_TEXT_CHARS))
        .filter(|command| !command.is_empty())
}

fn input_paths(input: &Value, request: &PermissionRequest) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for field in [
        "path",
        "paths",
        "file_path",
        "file_paths",
        "directory",
        "directories",
        "cwd",
        "target",
    ] {
        if let Some(value) = value_field(input, field) {
            paths.extend(value_to_paths(value));
        }
    }

    if paths.is_empty() {
        request.paths.clone()
    } else {
        paths
    }
}

fn first_path(input: &Value) -> Option<PathBuf> {
    let empty_request = PermissionRequest::new("");
    input_paths(input, &empty_request).into_iter().next()
}

fn value_field<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
    value.as_object()?.get(field)
}

fn value_to_paths(value: &Value) -> Vec<PathBuf> {
    match value {
        Value::String(path) => vec![PathBuf::from(path)],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .collect(),
        _ => Vec::new(),
    }
}

fn string_field(input: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| {
        value_field(input, field)
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn string_array_field(input: &Value, fields: &[&str]) -> Option<Vec<String>> {
    fields.iter().find_map(|field| {
        let value = value_field(input, field)?;
        let Value::Array(values) = value else {
            return None;
        };

        Some(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>(),
        )
    })
}

fn format_path_list(paths: &[PathBuf]) -> String {
    let rendered = paths
        .iter()
        .take(2)
        .map(|path| truncate_path(path, MAX_PATH_CHARS))
        .collect::<Vec<_>>();
    match paths.len() {
        0 => String::new(),
        1 => rendered[0].clone(),
        2 => rendered.join(", "),
        len => format!("{}, +{} more", rendered.join(", "), len - 2),
    }
}

fn truncate_path(path: &Path, max_chars: usize) -> String {
    let display = path.display().to_string();
    if display.chars().count() <= max_chars {
        return display;
    }
    if max_chars <= 1 {
        return "…".into();
    }

    let separator = std::path::MAIN_SEPARATOR;
    let mut kept = VecDeque::new();
    let mut used = 1usize;
    for component in path.components().rev() {
        let part = component_text(component);
        if part.is_empty() {
            continue;
        }
        let extra = part.chars().count() + usize::from(!kept.is_empty());
        if used + extra > max_chars {
            break;
        }
        kept.push_front(part);
        used += extra;
    }

    if kept.is_empty() {
        return truncate_text(&display, max_chars);
    }

    let joined = kept
        .into_iter()
        .collect::<Vec<_>>()
        .join(&separator.to_string());
    format!("…{separator}{joined}")
}

fn component_text(component: Component<'_>) -> String {
    match component {
        Component::Prefix(prefix) => prefix.as_os_str().to_string_lossy().into_owned(),
        Component::RootDir => String::new(),
        Component::CurDir => ".".into(),
        Component::ParentDir => "..".into(),
        Component::Normal(part) => part.to_string_lossy().into_owned(),
    }
}

fn sanitize_inline_text(text: &str, max_chars: usize) -> String {
    let mut sanitized = String::new();
    let mut last_was_space = false;

    for ch in text.chars() {
        let mapped = if ch.is_control() { ' ' } else { ch };
        if mapped.is_whitespace() {
            if !last_was_space {
                sanitized.push(' ');
                last_was_space = true;
            }
        } else {
            sanitized.push(mapped);
            last_was_space = false;
        }
    }

    truncate_text(sanitized.trim(), max_chars)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    if max_chars <= 1 {
        return "…".into();
    }

    let mut truncated = text.chars().take(max_chars - 1).collect::<String>();
    truncated.push('…');
    truncated
}

const fn plural_suffix(len: usize) -> &'static str {
    if len == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use wonder_of_u_core::PermissionRequest;

    use super::*;

    #[test]
    fn access_labels_follow_request_flags() {
        let read_only = PermissionSummaryView::from_request(
            &PermissionRequest::new("file_read").read_only(true),
            &json!({}),
        );
        let destructive = PermissionSummaryView::from_request(
            &PermissionRequest::new("bash").destructive(true),
            &json!({}),
        );

        assert_eq!(read_only.access, PermissionAccessKind::ReadOnly);
        assert_eq!(read_only.access.label(), "read-only");
        assert_eq!(destructive.access, PermissionAccessKind::Destructive);
        assert_eq!(destructive.access.label(), "destructive");
    }

    #[test]
    fn path_details_truncate_long_prefixes() {
        let path = PathBuf::from("/workspace/projects/alpha/src/components/permissions/dialog.rs");
        let summary = PermissionSummaryView::from_request(
            &PermissionRequest::new("file_edit").with_path(path.clone()),
            &json!({ "path": path.display().to_string() }),
        );

        assert_eq!(summary.details.len(), 1);
        assert_eq!(summary.details[0].label, "Path");
        assert!(summary.details[0].value.starts_with("…/"));
        assert!(
            summary.details[0]
                .value
                .ends_with("src/components/permissions/dialog.rs")
        );
        assert!(summary.details[0].value.chars().count() <= MAX_PATH_CHARS + 2);
    }

    #[test]
    fn command_details_are_sanitized_and_truncated() {
        let summary = PermissionSummaryView::from_request(
            &PermissionRequest::new("bash"),
            &json!({
                "command": "printf 'hello'\n\t&& echo ok \u{1b}[31mXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX"
            }),
        );

        assert_eq!(summary.details.len(), 1);
        assert_eq!(summary.details[0].label, "Command");
        assert!(!summary.details[0].value.contains('\n'));
        assert!(!summary.details[0].value.contains('\t'));
        assert!(!summary.details[0].value.contains('\u{1b}'));
        assert!(summary.details[0].value.ends_with('…'));
    }

    #[test]
    fn permission_actions_use_allow_and_deny_hints() {
        let summary = PermissionSummaryView::from_request(
            &PermissionRequest::new("web_fetch").read_only(true),
            &json!({}),
        );

        assert_eq!(summary.actions[0], DialogActionView::new("Allow", true));
        assert_eq!(summary.actions[1], DialogActionView::new("Deny", false));
        assert_eq!(summary.action_hint(), "[Allow]  Deny");
        assert_eq!(summary.to_dialog_view().action_hint(), "[Allow]  Deny");
    }

    #[test]
    fn unknown_tools_fall_back_to_generic_copy() {
        let summary =
            PermissionSummaryView::from_request(&PermissionRequest::new("demo_tool"), &json!({}));

        assert_eq!(summary.title, "Use tool `demo_tool`");
        assert_eq!(summary.prompt, "Allow Claude to use `demo_tool`?");
        assert!(summary.details.is_empty());
    }
}
