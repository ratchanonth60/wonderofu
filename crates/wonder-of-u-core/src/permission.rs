use std::{
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// User-facing permission modes planned for command and tool execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    #[default]
    Default,
    AcceptEdits,
    BypassPermissions,
    DontAsk,
    Plan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleBehavior {
    Allow,
    Deny,
    Ask,
}

impl PermissionRuleBehavior {
    #[must_use]
    pub const fn precedence(self) -> u8 {
        match self {
            Self::Deny => 0,
            Self::Ask => 1,
            Self::Allow => 2,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Ask => "ask",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleSource {
    Policy,
    CliArg,
    SessionRuntime,
    Command,
    Local,
    Project,
    User,
}

impl PermissionRuleSource {
    /// Lower values are stronger. Policy is never weakened by lower-precedence
    /// sources, while session/CLI overrides remain stronger than persisted user
    /// settings.
    #[must_use]
    pub const fn precedence(self) -> u8 {
        match self {
            Self::Policy => 0,
            Self::CliArg => 1,
            Self::SessionRuntime => 2,
            Self::Command => 3,
            Self::Local => 4,
            Self::Project => 5,
            Self::User => 6,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::CliArg => "cli_arg",
            Self::SessionRuntime => "session_runtime",
            Self::Command => "command",
            Self::Local => "local",
            Self::Project => "project",
            Self::User => "user",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionRuleConstraint {
    #[default]
    Any,
    PathPrefix {
        path: PathBuf,
    },
    ShellCommandContains {
        text: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub tool: String,
    pub behavior: PermissionRuleBehavior,
    pub source: PermissionRuleSource,
    #[serde(default)]
    pub constraint: PermissionRuleConstraint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PermissionRule {
    #[must_use]
    pub fn new(
        tool: impl Into<String>,
        behavior: PermissionRuleBehavior,
        source: PermissionRuleSource,
    ) -> Self {
        Self {
            tool: tool.into(),
            behavior,
            source,
            constraint: PermissionRuleConstraint::Any,
            reason: None,
        }
    }

    #[must_use]
    pub fn for_path_prefix(mut self, path: impl Into<PathBuf>) -> Self {
        self.constraint = PermissionRuleConstraint::PathPrefix { path: path.into() };
        self
    }

    #[must_use]
    pub fn for_shell_command(mut self, text: impl Into<String>) -> Self {
        self.constraint = PermissionRuleConstraint::ShellCommandContains { text: text.into() };
        self
    }

    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    #[must_use]
    pub fn matches(&self, context: &ToolPermissionContext, request: &PermissionRequest) -> bool {
        let tool = normalize_tool_name(&self.tool);
        if tool.as_deref() != Some("*") && !request.matches_tool_name(&self.tool) {
            return false;
        }

        match &self.constraint {
            PermissionRuleConstraint::Any => true,
            PermissionRuleConstraint::PathPrefix { path } => {
                if request.paths.is_empty() {
                    return false;
                }
                let prefix = context.resolve_path(path);
                request
                    .paths
                    .iter()
                    .map(|path| context.resolve_path(path))
                    .all(|path| is_path_within(&path, &prefix))
            }
            PermissionRuleConstraint::ShellCommandContains { text } => request
                .shell_command
                .as_deref()
                .is_some_and(|command| command.contains(text)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdditionalWorkingDirectory {
    pub path: PathBuf,
    pub source: PermissionRuleSource,
}

impl AdditionalWorkingDirectory {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, source: PermissionRuleSource) -> Self {
        Self {
            path: path.into(),
            source,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolPermissionContext {
    pub cwd: PathBuf,
    pub mode: PermissionMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_working_directories: Vec<AdditionalWorkingDirectory>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<PermissionRule>,
}

impl ToolPermissionContext {
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>, mode: PermissionMode) -> Self {
        Self {
            cwd: cwd.into(),
            mode,
            additional_working_directories: Vec::new(),
            rules: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_rule(mut self, rule: PermissionRule) -> Self {
        self.rules.push(rule);
        self
    }

    #[must_use]
    pub fn with_additional_directory(
        mut self,
        path: impl Into<PathBuf>,
        source: PermissionRuleSource,
    ) -> Self {
        self.additional_working_directories
            .push(AdditionalWorkingDirectory::new(path, source));
        self
    }

    #[must_use]
    pub fn working_directories(&self) -> Vec<PathBuf> {
        std::iter::once(self.resolve_path(&self.cwd))
            .chain(
                self.additional_working_directories
                    .iter()
                    .map(|directory| self.resolve_path(&directory.path)),
            )
            .collect()
    }

    #[must_use]
    pub fn resolve_path(&self, path: impl AsRef<Path>) -> PathBuf {
        resolve_path(path.as_ref(), &self.cwd)
    }

    #[must_use]
    pub fn path_in_scope(&self, path: impl AsRef<Path>) -> bool {
        let resolved = self.resolve_path(path);
        self.working_directories()
            .into_iter()
            .any(|scope| is_path_within(&resolved, &scope))
    }

    #[must_use]
    pub fn first_path_outside_scope(&self, paths: &[PathBuf]) -> Option<PathBuf> {
        paths
            .iter()
            .map(|path| self.resolve_path(path))
            .find(|path| !self.path_in_scope(path))
    }

    #[must_use]
    fn first_path_escape_attempt(&self, paths: &[PathBuf]) -> Option<PathBuf> {
        let scopes = self.working_directories();
        paths
            .iter()
            .filter(|path| contains_parent_dir(path))
            .map(|path| self.resolve_path(path))
            .find(|path| scopes.iter().all(|scope| !is_path_within(path, scope)))
    }

    #[must_use]
    pub fn evaluate(&self, request: &PermissionRequest) -> PermissionDecision {
        evaluate_permission(self, request)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_aliases: Vec<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_command: Option<String>,
}

impl PermissionRequest {
    #[must_use]
    pub fn new(tool_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            tool_aliases: Vec::new(),
            read_only: false,
            destructive: false,
            paths: Vec::new(),
            shell_command: None,
        }
    }

    #[must_use]
    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.tool_aliases.push(alias.into());
        self
    }

    #[must_use]
    pub fn with_aliases(mut self, aliases: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tool_aliases
            .extend(aliases.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    #[must_use]
    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    #[must_use]
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.paths.push(path.into());
        self
    }

    #[must_use]
    pub fn with_paths(mut self, paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        self.paths.extend(paths.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn with_shell_command(mut self, shell_command: impl Into<String>) -> Self {
        self.shell_command = Some(shell_command.into());
        self
    }

    #[must_use]
    pub fn matches_tool_name(&self, name: &str) -> bool {
        let Some(name) = normalize_tool_name(name) else {
            return false;
        };

        normalize_tool_name(&self.tool_name).as_deref() == Some(name.as_str())
            || self
                .tool_aliases
                .iter()
                .filter_map(|alias| normalize_tool_name(alias))
                .any(|alias| alias == name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ShellSafetyVerdict {
    Review,
    Blocked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShellSafetyIssue {
    pub verdict: ShellSafetyVerdict,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionDecisionReason {
    Rule {
        rule: PermissionRule,
    },
    Mode {
        mode: PermissionMode,
        detail: String,
    },
    PathScope {
        path: PathBuf,
        allowed_roots: Vec<PathBuf>,
    },
    ShellSafety {
        issue: ShellSafetyIssue,
    },
}

impl fmt::Display for PermissionDecisionReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rule { rule } => {
                write!(
                    f,
                    "matched {} rule from {} for {}",
                    rule.behavior.label(),
                    rule.source.label(),
                    rule.tool
                )?;
                match &rule.constraint {
                    PermissionRuleConstraint::Any => {}
                    PermissionRuleConstraint::PathPrefix { path } => {
                        write!(f, " on path prefix {}", path.display())?;
                    }
                    PermissionRuleConstraint::ShellCommandContains { text } => {
                        write!(f, " on shell text {text:?}")?;
                    }
                }
                if let Some(reason) = &rule.reason {
                    write!(f, " ({reason})")?;
                }
                Ok(())
            }
            Self::Mode { mode, detail } => {
                write!(f, "{mode:?}: {detail}")
            }
            Self::PathScope {
                path,
                allowed_roots,
            } => {
                write!(
                    f,
                    "path {} is outside the working directories: {}",
                    path.display(),
                    allowed_roots
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            Self::ShellSafety { issue } => write!(f, "{}", issue.message),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow { reason: PermissionDecisionReason },
    Ask { reason: PermissionDecisionReason },
    Deny { reason: PermissionDecisionReason },
}

impl PermissionDecision {
    #[must_use]
    pub fn allow(reason: PermissionDecisionReason) -> Self {
        Self::Allow { reason }
    }

    #[must_use]
    pub fn ask(reason: PermissionDecisionReason) -> Self {
        Self::Ask { reason }
    }

    #[must_use]
    pub fn deny(reason: PermissionDecisionReason) -> Self {
        Self::Deny { reason }
    }

    #[must_use]
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }

    #[must_use]
    pub fn reason(&self) -> &PermissionDecisionReason {
        match self {
            Self::Allow { reason } | Self::Ask { reason } | Self::Deny { reason } => reason,
        }
    }
}

#[must_use]
pub fn evaluate_permission(
    context: &ToolPermissionContext,
    request: &PermissionRequest,
) -> PermissionDecision {
    let matched_rule = resolve_matching_rule(context, request);

    if let Some(rule) = matched_rule
        .as_ref()
        .filter(|rule| rule.behavior == PermissionRuleBehavior::Deny)
    {
        return PermissionDecision::deny(PermissionDecisionReason::Rule { rule: rule.clone() });
    }

    if let Some(issue) = request
        .shell_command
        .as_deref()
        .and_then(check_shell_safety)
        .filter(|issue| issue.verdict == ShellSafetyVerdict::Blocked)
    {
        return PermissionDecision::deny(PermissionDecisionReason::ShellSafety { issue });
    }

    if let Some(rule) = matched_rule
        .as_ref()
        .filter(|rule| rule.behavior == PermissionRuleBehavior::Allow)
    {
        return PermissionDecision::allow(PermissionDecisionReason::Rule { rule: rule.clone() });
    }

    if context.mode != PermissionMode::BypassPermissions {
        if let Some(path) = context.first_path_escape_attempt(&request.paths) {
            return PermissionDecision::deny(PermissionDecisionReason::PathScope {
                path,
                allowed_roots: context.working_directories(),
            });
        }
    }

    if let Some(path) = context.first_path_outside_scope(&request.paths) {
        return review_decision(
            context.mode,
            request.read_only,
            PermissionDecisionReason::PathScope {
                path,
                allowed_roots: context.working_directories(),
            },
        );
    }

    if let Some(issue) = request
        .shell_command
        .as_deref()
        .and_then(check_shell_safety)
        .filter(|issue| issue.verdict == ShellSafetyVerdict::Review)
    {
        return review_decision(
            context.mode,
            request.read_only,
            PermissionDecisionReason::ShellSafety { issue },
        );
    }

    if let Some(rule) = matched_rule
        .as_ref()
        .filter(|rule| rule.behavior == PermissionRuleBehavior::Ask)
    {
        return PermissionDecision::ask(PermissionDecisionReason::Rule { rule: rule.clone() });
    }

    context
        .mode
        .default_decision(request.read_only, request.destructive)
}

#[must_use]
pub fn check_shell_safety(command: &str) -> Option<ShellSafetyIssue> {
    let normalized = command.to_ascii_lowercase();
    const BLOCKED_PATTERNS: [(&str, &str); 4] = [
        (
            "rm -rf /",
            "shell command is denied because it attempts to remove the filesystem root",
        ),
        (
            "@p}",
            "shell command uses disallowed ${var@P}-style expansion",
        ),
        (
            "${!",
            "shell command uses disallowed indirect parameter expansion",
        ),
        ("eval ", "shell command uses eval-style dynamic execution"),
    ];
    const REVIEW_PATTERNS: [(&str, &str); 5] = [
        (
            "rm -rf",
            "shell command requires review because it removes files recursively",
        ),
        (
            "| sh",
            "shell command requires review because it pipes output into a shell",
        ),
        (
            "sudo ",
            "shell command requires review because it escalates privileges",
        ),
        (
            "mkfs",
            "shell command requires review because it can reformat a device",
        ),
        (
            "dd if=",
            "shell command requires review because it can overwrite raw devices",
        ),
    ];

    for (pattern, message) in BLOCKED_PATTERNS {
        if normalized.contains(pattern) {
            return Some(ShellSafetyIssue {
                verdict: ShellSafetyVerdict::Blocked,
                message: message.into(),
            });
        }
    }

    for (pattern, message) in REVIEW_PATTERNS {
        if normalized.contains(pattern) {
            return Some(ShellSafetyIssue {
                verdict: ShellSafetyVerdict::Review,
                message: message.into(),
            });
        }
    }

    None
}

#[must_use]
pub fn resolve_path(path: &Path, cwd: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    normalize_path(&absolute)
}

#[must_use]
pub fn is_path_within(path: &Path, root: &Path) -> bool {
    let path = normalize_path(path);
    let root = normalize_path(root);
    path == root || path.starts_with(&root)
}

fn review_decision(
    mode: PermissionMode,
    read_only: bool,
    reason: PermissionDecisionReason,
) -> PermissionDecision {
    match mode {
        PermissionMode::BypassPermissions => PermissionDecision::allow(reason),
        PermissionMode::DontAsk => PermissionDecision::deny(reason),
        PermissionMode::Plan if !read_only => PermissionDecision::deny(reason),
        _ => PermissionDecision::ask(reason),
    }
}

fn resolve_matching_rule(
    context: &ToolPermissionContext,
    request: &PermissionRequest,
) -> Option<PermissionRule> {
    context
        .rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule.matches(context, request))
        .min_by_key(|(index, rule)| (rule.source.precedence(), rule.behavior.precedence(), *index))
        .map(|(_, rule)| rule.clone())
}

fn normalize_tool_name(name: &str) -> Option<String> {
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    let mut absolute = false;

    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => {
                absolute = true;
                normalized.push(component.as_os_str());
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !normalized.pop() && !absolute {
                    normalized.push(component.as_os_str());
                }
            }
            std::path::Component::Normal(part) => normalized.push(part),
        }
    }

    if normalized.as_os_str().is_empty() {
        if absolute {
            PathBuf::from(std::path::MAIN_SEPARATOR.to_string())
        } else {
            PathBuf::from(".")
        }
    } else {
        normalized
    }
}

fn contains_parent_dir(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
}

impl PermissionMode {
    /// Conservative defaults before UI prompting, hooks, and per-tool safety
    /// handlers exist.
    #[must_use]
    pub fn default_decision(self, read_only: bool, destructive: bool) -> PermissionDecision {
        match self {
            Self::BypassPermissions => PermissionDecision::allow(PermissionDecisionReason::Mode {
                mode: self,
                detail: "bypass permission mode".into(),
            }),
            Self::DontAsk => PermissionDecision::deny(PermissionDecisionReason::Mode {
                mode: self,
                detail: "dontAsk mode denies actions requiring confirmation".into(),
            }),
            Self::Plan if !read_only => PermissionDecision::deny(PermissionDecisionReason::Mode {
                mode: self,
                detail: "plan mode allows read-only actions only".into(),
            }),
            Self::AcceptEdits if !destructive => {
                PermissionDecision::allow(PermissionDecisionReason::Mode {
                    mode: self,
                    detail: "acceptEdits mode allows non-destructive edits".into(),
                })
            }
            _ if read_only => PermissionDecision::allow(PermissionDecisionReason::Mode {
                mode: self,
                detail: "read-only action".into(),
            }),
            _ => PermissionDecision::ask(PermissionDecisionReason::Mode {
                mode: self,
                detail: "confirmation required by default permission mode".into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_mode_denies_writes() {
        let decision = PermissionMode::Plan.default_decision(false, false);
        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn bypass_mode_allows_destructive_actions() {
        let decision = PermissionMode::BypassPermissions.default_decision(false, true);
        assert!(decision.is_allowed());
    }

    #[test]
    fn policy_rule_beats_lower_precedence_allow() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::Default)
            .with_rule(
                PermissionRule::new(
                    "file_read",
                    PermissionRuleBehavior::Allow,
                    PermissionRuleSource::User,
                )
                .with_reason("user wants broad access"),
            )
            .with_rule(
                PermissionRule::new(
                    "file_read",
                    PermissionRuleBehavior::Deny,
                    PermissionRuleSource::Policy,
                )
                .with_reason("policy blocks it"),
            );
        let request = PermissionRequest::new("file_read").read_only(true);

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Deny { .. }));
        assert!(decision.reason().to_string().contains("policy"));
    }

    #[test]
    fn same_source_prefers_more_restrictive_rule() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::Default)
            .with_rule(PermissionRule::new(
                "bash",
                PermissionRuleBehavior::Allow,
                PermissionRuleSource::SessionRuntime,
            ))
            .with_rule(PermissionRule::new(
                "bash",
                PermissionRuleBehavior::Ask,
                PermissionRuleSource::SessionRuntime,
            ));
        let request = PermissionRequest::new("bash");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Ask { .. }));
    }

    #[test]
    fn path_scope_resolves_relative_paths_and_additional_directories() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::Default)
            .with_additional_directory("logs", PermissionRuleSource::SessionRuntime);

        assert!(context.path_in_scope("src/lib.rs"));
        assert!(context.path_in_scope("logs/output.txt"));
        assert!(!context.path_in_scope("../secrets.txt"));
    }

    #[test]
    fn allow_rule_can_whitelist_external_directory() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::Default).with_rule(
            PermissionRule::new(
                "file_read",
                PermissionRuleBehavior::Allow,
                PermissionRuleSource::CliArg,
            )
            .for_path_prefix("../shared"),
        );
        let request = PermissionRequest::new("file_read")
            .read_only(true)
            .with_path("../shared/notes.txt");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Allow { .. }));
    }

    #[test]
    fn path_scope_requires_review_for_external_reads_without_rule() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::Default);
        let request = PermissionRequest::new("file_read")
            .read_only(true)
            .with_path("/shared/notes.txt");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Ask { .. }));
        assert!(
            decision
                .reason()
                .to_string()
                .contains("outside the working directories")
        );
    }

    #[test]
    fn dont_ask_turns_review_into_denial() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::DontAsk);
        let request = PermissionRequest::new("bash").with_shell_command("rm -rf build");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn shell_safety_blocks_obfuscated_commands() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::BypassPermissions);
        let request = PermissionRequest::new("bash").with_shell_command("echo ${cmd@P}");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Deny { .. }));
        assert!(decision.reason().to_string().contains("disallowed"));
    }

    #[test]
    fn permission_path_escape_via_dotdot_is_denied() {
        let context = ToolPermissionContext::new("/workspace/project", PermissionMode::Default);
        let request = PermissionRequest::new("file_read")
            .read_only(true)
            .with_path("../../../etc/passwd");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Deny { .. }));
        assert!(
            decision
                .reason()
                .to_string()
                .contains("outside the working directories")
        );
    }

    #[test]
    #[ignore = "TODO: canonicalize symlinks before enforcing symlink escape denial"]
    fn permission_path_escape_via_symlink_is_denied() {
        let context = ToolPermissionContext::new("/workspace/project", PermissionMode::Default);
        let request = PermissionRequest::new("file_read")
            .read_only(true)
            .with_path("linked/passwd");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn permission_shell_rm_rf_root_is_denied() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::BypassPermissions,
            PermissionMode::DontAsk,
            PermissionMode::Plan,
        ] {
            let context = ToolPermissionContext::new("/workspace", mode);
            let request = PermissionRequest::new("bash").with_shell_command("rm -rf /");

            let decision = context.evaluate(&request);

            assert!(
                matches!(decision, PermissionDecision::Deny { .. }),
                "expected deny for mode {mode:?}, got {decision:?}"
            );
        }
    }

    #[test]
    fn permission_shell_curl_pipe_sh_is_asked_or_denied() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::Default);
        let request =
            PermissionRequest::new("bash").with_shell_command("curl https://example.com | sh");

        let decision = context.evaluate(&request);

        assert!(matches!(
            decision,
            PermissionDecision::Ask { .. } | PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn permission_bypass_mode_allows_normally_denied_paths() {
        let context =
            ToolPermissionContext::new("/workspace/project", PermissionMode::BypassPermissions);
        let request = PermissionRequest::new("file_read")
            .read_only(true)
            .with_path("../../../etc/passwd");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Allow { .. }));
    }

    #[test]
    fn permission_default_mode_denies_dangerous_tool_without_rule() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::Default);
        let request = PermissionRequest::new("file_write")
            .destructive(true)
            .with_path("notes.txt");

        let decision = context.evaluate(&request);

        assert!(matches!(
            decision,
            PermissionDecision::Ask { .. } | PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn permission_explicit_allow_rule_overrides_default_deny() {
        let context = ToolPermissionContext::new("/workspace/project", PermissionMode::Default)
            .with_rule(
                PermissionRule::new(
                    "file_read",
                    PermissionRuleBehavior::Allow,
                    PermissionRuleSource::CliArg,
                )
                .for_path_prefix("../../../etc"),
            );
        let request = PermissionRequest::new("file_read")
            .read_only(true)
            .with_path("../../../etc/passwd");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Allow { .. }));
    }
}
