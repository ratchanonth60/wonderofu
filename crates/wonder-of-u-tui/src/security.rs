//! Renderer-agnostic security review views for workspace trust and managed settings.
//!
//! These models keep trust and managed-settings review focused on safe summaries:
//! names, sources, and action labels. Policy enforcement stays in higher layers.

use std::{
    collections::{BTreeSet, VecDeque},
    path::{Component, Path},
};

use serde_json::Value;

use crate::dialog::{DialogActionView, DialogView};

const MAX_PATH_CHARS: usize = 48;
const MAX_SOURCE_CHARS: usize = 48;
const MAX_SETTING_CHARS: usize = 48;

const DANGEROUS_SHELL_SETTINGS: &[&str] = &[
    "apiKeyHelper",
    "awsAuthRefresh",
    "awsCredentialExport",
    "gcpAuthRefresh",
    "otelHeadersHelper",
    "statusLine",
];

const SAFE_ENV_VARS: &[&str] = &[
    "ANTHROPIC_CUSTOM_HEADERS",
    "ANTHROPIC_CUSTOM_MODEL_OPTION",
    "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION",
    "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_SUPPORTED_CAPABILITIES",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_SUPPORTED_CAPABILITIES",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL_AWS_REGION",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "AWS_ACCESS_KEY_ID",
    "AWS_DEFAULT_REGION",
    "AWS_PROFILE",
    "AWS_REGION",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "BASH_DEFAULT_TIMEOUT_MS",
    "BASH_MAX_OUTPUT_LENGTH",
    "BASH_MAX_TIMEOUT_MS",
    "CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR",
    "CLAUDE_CODE_API_KEY_HELPER_TTL_MS",
    "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "CLAUDE_CODE_DISABLE_TERMINAL_TITLE",
    "CLAUDE_CODE_ENABLE_TELEMETRY",
    "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS",
    "CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL",
    "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "CLAUDE_CODE_SKIP_FOUNDRY_AUTH",
    "CLAUDE_CODE_SKIP_VERTEX_AUTH",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_VERTEX",
    "DISABLE_AUTOUPDATER",
    "DISABLE_BUG_COMMAND",
    "DISABLE_COST_WARNINGS",
    "DISABLE_ERROR_REPORTING",
    "DISABLE_FEEDBACK_COMMAND",
    "DISABLE_TELEMETRY",
    "ENABLE_TOOL_SEARCH",
    "MAX_MCP_OUTPUT_TOKENS",
    "MAX_THINKING_TOKENS",
    "MCP_TIMEOUT",
    "MCP_TOOL_TIMEOUT",
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
    "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL",
    "OTEL_EXPORTER_OTLP_METRICS_CLIENT_CERTIFICATE",
    "OTEL_EXPORTER_OTLP_METRICS_CLIENT_KEY",
    "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
    "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL",
    "OTEL_EXPORTER_OTLP_PROTOCOL",
    "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    "OTEL_LOG_TOOL_DETAILS",
    "OTEL_LOG_USER_PROMPTS",
    "OTEL_LOGS_EXPORT_INTERVAL",
    "OTEL_LOGS_EXPORTER",
    "OTEL_METRIC_EXPORT_INTERVAL",
    "OTEL_METRICS_EXPORTER",
    "OTEL_METRICS_INCLUDE_ACCOUNT_UUID",
    "OTEL_METRICS_INCLUDE_SESSION_ID",
    "OTEL_METRICS_INCLUDE_VERSION",
    "OTEL_RESOURCE_ATTRIBUTES",
    "USE_BUILTIN_RIPGREP",
    "VERTEX_REGION_CLAUDE_3_5_HAIKU",
    "VERTEX_REGION_CLAUDE_3_5_SONNET",
    "VERTEX_REGION_CLAUDE_3_7_SONNET",
    "VERTEX_REGION_CLAUDE_4_0_OPUS",
    "VERTEX_REGION_CLAUDE_4_0_SONNET",
    "VERTEX_REGION_CLAUDE_4_1_OPUS",
    "VERTEX_REGION_CLAUDE_4_5_SONNET",
    "VERTEX_REGION_CLAUDE_4_6_SONNET",
    "VERTEX_REGION_CLAUDE_HAIKU_4_5",
];

/// User-facing trust state labels for workspace review.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceTrustState {
    /// Represents trusted
    Trusted,
    /// Represents untrusted
    Untrusted,
    /// Represents review required
    ReviewRequired,
}

impl WorkspaceTrustState {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
            Self::ReviewRequired => "review required",
        }
    }
}

/// The semantic action a security dialog can expose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityActionKind {
    /// Represents allow
    Allow,
    /// Represents deny
    Deny,
    /// Represents defer
    Defer,
}

/// A renderer-neutral action option for trust and security dialogs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecurityActionView {
    /// Stores the kind
    pub kind: SecurityActionKind,
    /// Stores the label
    pub label: String,
    /// Stores the primary
    pub primary: bool,
}

impl SecurityActionView {
    /// Creates a new value
    #[must_use]
    pub fn new(kind: SecurityActionKind, label: impl Into<String>, primary: bool) -> Self {
        Self {
            kind,
            label: label.into(),
            primary,
        }
    }
    /// Handles allow
    #[must_use]
    pub fn allow(label: impl Into<String>, primary: bool) -> Self {
        Self::new(SecurityActionKind::Allow, label, primary)
    }
    /// Handles deny
    #[must_use]
    pub fn deny(label: impl Into<String>, primary: bool) -> Self {
        Self::new(SecurityActionKind::Deny, label, primary)
    }
    /// Handles defer
    #[must_use]
    pub fn defer(label: impl Into<String>, primary: bool) -> Self {
        Self::new(SecurityActionKind::Defer, label, primary)
    }
    /// Handles to dialog action
    #[must_use]
    pub fn to_dialog_action(&self) -> DialogActionView {
        DialogActionView::new(self.label.clone(), self.primary)
    }
}

/// Formats security action labels in the same compact style used by dialogs.
#[must_use]
pub fn security_action_hint(actions: &[SecurityActionView]) -> String {
    actions
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

/// High-level risky workspace capabilities shown during trust review.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceRiskKind {
    /// Represents mcp servers
    McpServers,
    /// Represents hooks
    Hooks,
    /// Represents bash permissions
    BashPermissions,
    /// Represents slash commands
    SlashCommands,
    /// Represents skills
    Skills,
    /// Represents api key helper
    ApiKeyHelper,
    /// Represents aws commands
    AwsCommands,
    /// Represents gcp commands
    GcpCommands,
    /// Represents otel headers helper
    OtelHeadersHelper,
    /// Represents dangerous env vars
    DangerousEnvVars,
    /// Represents managed settings
    ManagedSettings,
}

impl WorkspaceRiskKind {
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::McpServers => "MCP servers",
            Self::Hooks => "Hooks",
            Self::BashPermissions => "Bash permissions",
            Self::SlashCommands => "Slash commands",
            Self::Skills => "Skills or plugins",
            Self::ApiKeyHelper => "apiKeyHelper",
            Self::AwsCommands => "AWS commands",
            Self::GcpCommands => "GCP commands",
            Self::OtelHeadersHelper => "otelHeadersHelper",
            Self::DangerousEnvVars => "Environment variables",
            Self::ManagedSettings => "Managed settings",
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::McpServers => "can connect project-scoped tools and external servers",
            Self::Hooks => "can run code automatically when workspace events fire",
            Self::BashPermissions => "allow shell execution from workspace settings",
            Self::SlashCommands => "can expand into workspace-provided shell commands",
            Self::Skills => "can run workspace-provided prompts with tool access",
            Self::ApiKeyHelper => "can run an external helper before requests",
            Self::AwsCommands => "can run external AWS refresh or export commands",
            Self::GcpCommands => "can run an external GCP refresh command",
            Self::OtelHeadersHelper => "can run an external telemetry helper command",
            Self::DangerousEnvVars => "override environment variables before commands run",
            Self::ManagedSettings => "come from centrally managed settings that still need review",
        }
    }
}

/// A safe workspace warning with sanitized source names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRiskView {
    /// Stores the kind
    pub kind: WorkspaceRiskKind,
    /// Stores the label
    pub label: String,
    /// Stores the summary
    pub summary: String,
    /// Stores the sources
    pub sources: Vec<String>,
}

impl WorkspaceRiskView {
    /// Creates a new value
    #[must_use]
    pub fn new(
        kind: WorkspaceRiskKind,
        sources: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        Self {
            kind,
            label: kind.label().into(),
            summary: kind.summary().into(),
            sources: sanitize_sources(sources),
        }
    }
    /// Handles source hint
    #[must_use]
    pub fn source_hint(&self) -> Option<String> {
        (!self.sources.is_empty())
            .then(|| format!("Sources: {}", format_list_with_and(&self.sources, Some(2))))
    }
}

/// A structured trust review for an untrusted or newly opened workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustDialogView {
    /// Stores the title
    pub title: String,
    /// Stores the workspace
    pub workspace: String,
    /// Stores the trust
    pub trust: WorkspaceTrustState,
    /// Stores the warnings
    pub warnings: Vec<WorkspaceRiskView>,
    /// Stores the guide url
    pub guide_url: Option<String>,
    /// Stores the actions
    pub actions: Vec<SecurityActionView>,
}

impl TrustDialogView {
    /// Creates a new value
    #[must_use]
    pub fn new(workspace: impl AsRef<Path>, warnings: Vec<WorkspaceRiskView>) -> Self {
        Self {
            title: "Accessing workspace".into(),
            workspace: truncate_path(workspace.as_ref(), MAX_PATH_CHARS),
            trust: WorkspaceTrustState::ReviewRequired,
            warnings,
            guide_url: Some("https://code.claude.com/docs/en/security".into()),
            actions: vec![
                SecurityActionView::allow("Yes, I trust this folder", true),
                SecurityActionView::deny("No, exit", false),
                SecurityActionView::defer("Review first", false),
            ],
        }
    }
    /// Handles action hint
    #[must_use]
    pub fn action_hint(&self) -> String {
        security_action_hint(&self.actions)
    }
    /// Handles to dialog view
    #[must_use]
    pub fn to_dialog_view(&self) -> DialogView {
        let mut body = vec![
            format!("Workspace: {}", self.workspace),
            "Quick safety check: only continue if you created this project or trust where it came from."
                .into(),
            "Claude will be able to read, edit, and execute files here.".into(),
        ];
        if !self.warnings.is_empty() {
            body.push("Signals to review before trusting:".into());
            body.extend(self.warnings.iter().flat_map(|warning| {
                let mut lines = vec![format!("• {} — {}", warning.label, warning.summary)];
                if let Some(source_hint) = warning.source_hint() {
                    lines.push(format!("  {source_hint}"));
                }
                lines
            }));
        }
        if let Some(url) = &self.guide_url {
            body.push(format!("Security guide: {url}"));
        }
        body.push(format!("Trust state: {}", self.trust.label()));

        DialogView {
            title: self.title.clone(),
            body,
            actions: self
                .actions
                .iter()
                .map(SecurityActionView::to_dialog_action)
                .collect(),
            selected_action: 0,
        }
    }
}

/// Indicates whether remote-managed settings are enforced or only surfaced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedSettingsEnforcement {
    /// Represents enforced
    Enforced,
    /// Represents informational only
    InformationalOnly,
}

impl ManagedSettingsEnforcement {
    /// Constant fn
    #[must_use]
    pub const fn note(self) -> Option<&'static str> {
        match self {
            Self::Enforced => None,
            Self::InformationalOnly => Some(
                "Remote-managed settings enforcement is not active in this runtime; review is informational only.",
            ),
        }
    }
}

/// A safe summary of one managed setting that needs review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedSettingRiskView {
    /// Stores the key
    pub key: String,
    /// Stores the label
    pub label: String,
    /// Stores the summary
    pub summary: String,
    /// Stores the sources
    pub sources: Vec<String>,
}

impl ManagedSettingRiskView {
    /// Handles from key
    #[must_use]
    pub fn from_key(
        key: impl AsRef<str>,
        sources: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        let key = sanitize_inline_text(key.as_ref(), MAX_SETTING_CHARS);
        let summary = match key.as_str() {
            "apiKeyHelper" => "runs an external helper before API requests",
            "awsAuthRefresh" => "runs an external AWS authentication refresh command",
            "awsCredentialExport" => "runs an external AWS credential export command",
            "gcpAuthRefresh" => "runs an external GCP authentication refresh command",
            "otelHeadersHelper" => "runs an external telemetry headers helper command",
            "statusLine" => "runs a workspace-controlled status command",
            "hooks" => "runs configured hooks automatically",
            _ if key
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_') =>
            {
                "sets a managed environment variable"
            }
            _ => "requires review before trust is granted",
        };

        Self {
            label: key.clone(),
            key,
            summary: summary.into(),
            sources: sanitize_sources(sources),
        }
    }
    /// Handles source hint
    #[must_use]
    pub fn source_hint(&self) -> Option<String> {
        (!self.sources.is_empty())
            .then(|| format!("Sources: {}", format_list_with_and(&self.sources, Some(2))))
    }
}

/// A structured security review for centrally managed settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedSettingsSecurityDialogView {
    /// Stores the title
    pub title: String,
    /// Stores the risks
    pub risks: Vec<ManagedSettingRiskView>,
    /// Stores the enforcement
    pub enforcement: ManagedSettingsEnforcement,
    /// Stores the actions
    pub actions: Vec<SecurityActionView>,
}

impl ManagedSettingsSecurityDialogView {
    /// Handles from settings
    #[must_use]
    pub fn from_settings(
        settings: &Value,
        source: Option<&str>,
        enforcement: ManagedSettingsEnforcement,
    ) -> Self {
        let risks = dangerous_managed_settings(settings, source);
        Self {
            title: "Managed settings require approval".into(),
            risks,
            enforcement,
            actions: vec![
                SecurityActionView::allow("Yes, I trust these settings", true),
                SecurityActionView::deny("No, exit", false),
                SecurityActionView::defer("Review later", false),
            ],
        }
    }
    /// Handles action hint
    #[must_use]
    pub fn action_hint(&self) -> String {
        security_action_hint(&self.actions)
    }
    /// Handles to dialog view
    #[must_use]
    pub fn to_dialog_view(&self) -> DialogView {
        let mut body = vec![
            "Managed settings can enable code execution or intercept prompts and responses.".into(),
        ];
        if !self.risks.is_empty() {
            body.push("Settings requiring approval:".into());
            body.extend(self.risks.iter().flat_map(|risk| {
                let mut lines = vec![format!("• {} — {}", risk.label, risk.summary)];
                if let Some(source_hint) = risk.source_hint() {
                    lines.push(format!("  {source_hint}"));
                }
                lines
            }));
        }
        if let Some(note) = self.enforcement.note() {
            body.push(note.into());
        }
        body.push(
            "Only continue if you trust the administrator or workflow that supplied these settings."
                .into(),
        );

        DialogView {
            title: self.title.clone(),
            body,
            actions: self
                .actions
                .iter()
                .map(SecurityActionView::to_dialog_action)
                .collect(),
            selected_action: 0,
        }
    }
}

/// Extracts risky managed settings without surfacing secret values.
#[must_use]
pub fn dangerous_managed_settings(
    settings: &Value,
    source: Option<&str>,
) -> Vec<ManagedSettingRiskView> {
    let Some(object) = settings.as_object() else {
        return Vec::new();
    };

    let source = source.unwrap_or_default();
    let mut risks = Vec::new();

    for key in DANGEROUS_SHELL_SETTINGS {
        if object
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
        {
            risks.push(ManagedSettingRiskView::from_key(*key, [source]));
        }
    }

    if let Some(env) = object.get("env").and_then(Value::as_object) {
        let mut keys = BTreeSet::new();
        for (key, value) in env {
            if value.as_str().is_some_and(|entry| !entry.is_empty()) && !safe_env_var(key) {
                keys.insert(key.as_str());
            }
        }
        risks.extend(
            keys.into_iter()
                .map(|key| ManagedSettingRiskView::from_key(key, [source])),
        );
    }

    if has_managed_hooks(object.get("hooks")) {
        risks.push(ManagedSettingRiskView::from_key("hooks", [source]));
    }

    risks
}

fn sanitize_sources(sources: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<String> {
    let mut rendered = BTreeSet::new();
    for source in sources {
        let source = source.as_ref().trim();
        if source.is_empty() {
            continue;
        }
        rendered.insert(sanitize_source(source));
    }
    rendered.into_iter().collect()
}

fn sanitize_source(source: &str) -> String {
    let sanitized = sanitize_inline_text(source, MAX_SOURCE_CHARS.saturating_mul(2));
    if sanitized.contains('/') || sanitized.contains('\\') {
        truncate_path(Path::new(&sanitized), MAX_SOURCE_CHARS)
    } else {
        truncate_text(&sanitized, MAX_SOURCE_CHARS)
    }
}

fn has_managed_hooks(hooks: Option<&Value>) -> bool {
    let Some(Value::Object(hooks)) = hooks else {
        return false;
    };
    !hooks.is_empty()
}

fn safe_env_var(key: &str) -> bool {
    SAFE_ENV_VARS
        .iter()
        .any(|safe| safe.eq_ignore_ascii_case(key))
}

fn format_list_with_and(items: &[String], limit: Option<usize>) -> String {
    if items.is_empty() {
        return String::new();
    }

    let effective_limit = match limit {
        Some(0) | None => None,
        Some(limit) => Some(limit),
    };

    if effective_limit.is_none_or(|limit| items.len() <= limit) {
        match items {
            [item] => item.clone(),
            [first, second] => format!("{first} and {second}"),
            _ => {
                let last = items.last().expect("items length checked");
                let all_but_last = &items[..items.len() - 1];
                format!("{}, and {}", all_but_last.join(", "), last)
            }
        }
    } else {
        let limit = effective_limit.expect("checked above");
        let shown = &items[..limit];
        let remaining = items.len() - limit;
        match shown {
            [item] => format!("{item} and {remaining} more"),
            _ => format!("{}, and {remaining} more", shown.join(", ")),
        }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn trust_labels_match_workspace_state() {
        assert_eq!(WorkspaceTrustState::Trusted.label(), "trusted");
        assert_eq!(WorkspaceTrustState::Untrusted.label(), "untrusted");
        assert_eq!(
            WorkspaceTrustState::ReviewRequired.label(),
            "review required"
        );
    }

    #[test]
    fn extracts_dangerous_managed_settings_without_values() {
        let settings = json!({
            "apiKeyHelper": "run helper",
            "statusLine": "echo status",
            "env": {
                "AWS_REGION": "ignored",
                "HTTP_PROXY": "http://example.invalid",
                "CUSTOM_TOKEN": "secret"
            },
            "hooks": {
                "PostToolUse": [{ "matcher": "*", "hooks": [] }]
            }
        });
        let risks = dangerous_managed_settings(&settings, Some(".claude/managed-settings.json"));

        let labels = risks
            .iter()
            .map(|risk| risk.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![
                "apiKeyHelper",
                "statusLine",
                "CUSTOM_TOKEN",
                "HTTP_PROXY",
                "hooks"
            ]
        );
        assert!(risks.iter().all(|risk| !risk.summary.contains("secret")));
        assert!(
            risks
                .iter()
                .all(|risk| risk.sources == vec![".claude/managed-settings.json".to_string()])
        );
    }

    #[test]
    fn long_paths_and_sources_are_truncated_safely() {
        let trust = TrustDialogView::new(
            std::path::PathBuf::from(
                "/workspace/projects/alpha/security/reviews/very/long/workspace/path",
            ),
            vec![WorkspaceRiskView::new(
                WorkspaceRiskKind::Hooks,
                ["/workspace/projects/alpha/.claude/settings.local.json"],
            )],
        );

        assert!(trust.workspace.starts_with("…/"));
        assert!(trust.workspace.chars().count() <= MAX_PATH_CHARS + 2);
        assert_eq!(trust.warnings.len(), 1);
        let source_hint = trust.warnings[0]
            .source_hint()
            .expect("source hint should be present");
        assert!(source_hint.starts_with("Sources: …/"));
        assert!(source_hint.ends_with(".claude/settings.local.json"));
    }

    #[test]
    fn action_hint_supports_allow_deny_and_defer() {
        let actions = vec![
            SecurityActionView::allow("Trust", true),
            SecurityActionView::deny("Exit", false),
            SecurityActionView::defer("Review later", false),
        ];

        assert_eq!(
            security_action_hint(&actions),
            "[Trust]  Exit  Review later"
        );
    }

    #[test]
    fn unknown_setting_keys_fall_back_to_sanitized_label() {
        let risk = ManagedSettingRiskView::from_key(
            " custom.setting\nname ",
            ["remote-managed-settings-service"],
        );

        assert_eq!(risk.label, "custom.setting name");
        assert_eq!(risk.summary, "requires review before trust is granted");
    }

    #[test]
    fn dialog_conversion_includes_informational_enforcement_note() {
        let dialog = ManagedSettingsSecurityDialogView::from_settings(
            &json!({
                "apiKeyHelper": "helper",
                "env": { "HTTP_PROXY": "http://example.invalid" }
            }),
            Some("remote/policies/security-team/default"),
            ManagedSettingsEnforcement::InformationalOnly,
        )
        .to_dialog_view();

        assert_eq!(dialog.title, "Managed settings require approval");
        assert!(
            dialog
                .body
                .iter()
                .any(|line| line.contains("informational only"))
        );
        assert!(dialog.body.iter().any(|line| line.contains("apiKeyHelper")));
        assert!(dialog.body.iter().any(|line| line.contains("HTTP_PROXY")));
        assert_eq!(
            dialog.actions,
            vec![
                DialogActionView::new("Yes, I trust these settings", true),
                DialogActionView::new("No, exit", false),
                DialogActionView::new("Review later", false),
            ]
        );
    }
}
