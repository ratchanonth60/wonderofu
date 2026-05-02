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
                let prefix = canonicalize_best_effort(&context.resolve_path(path));
                request
                    .paths
                    .iter()
                    .map(|path| canonicalize_best_effort(&context.resolve_path(path)))
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
        std::iter::once(canonicalize_best_effort(&self.resolve_path(&self.cwd)))
            .chain(
                self.additional_working_directories
                    .iter()
                    .map(|directory| canonicalize_best_effort(&self.resolve_path(&directory.path))),
            )
            .collect()
    }

    #[must_use]
    pub fn resolve_path(&self, path: impl AsRef<Path>) -> PathBuf {
        resolve_path(path.as_ref(), &self.cwd)
    }

    #[must_use]
    pub fn path_in_scope(&self, path: impl AsRef<Path>) -> bool {
        let resolved = canonicalize_best_effort(&self.resolve_path(path));
        self.working_directories()
            .into_iter()
            .any(|scope| is_path_within(&resolved, &scope))
    }

    #[must_use]
    pub fn first_path_outside_scope(&self, paths: &[PathBuf]) -> Option<PathBuf> {
        paths
            .iter()
            .map(|path| canonicalize_best_effort(&self.resolve_path(path)))
            .find(|path| !self.path_in_scope(path))
    }

    #[must_use]
    fn first_path_escape_attempt(&self, paths: &[PathBuf]) -> Option<PathBuf> {
        let scopes = self.working_directories();
        paths.iter().find_map(|path| {
            let resolved = self.resolve_path(path);
            let canonical = canonicalize_best_effort(&resolved);
            let canonical_outside = scopes
                .iter()
                .all(|scope| !is_path_within(&canonical, scope));
            if contains_parent_dir(path) && canonical_outside {
                return Some(canonical);
            }

            let raw_inside = scopes.iter().any(|scope| is_path_within(&resolved, scope));
            (raw_inside && canonical_outside).then_some(canonical)
        })
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
        .and_then(|command| check_shell_safety_for_request(request, command))
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
        .and_then(|command| check_shell_safety_for_request(request, command))
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
    let normalized = normalize_shell_command(command);

    if normalized.contains("rm -rf /") || normalized.contains("rm -fr /") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Blocked,
            message: "shell command is denied because it attempts to remove the filesystem root"
                .into(),
        });
    }
    if normalized.contains("@p}") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Blocked,
            message: "shell command uses disallowed ${var@P}-style expansion".into(),
        });
    }
    if normalized.contains("${!") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Blocked,
            message: "shell command uses disallowed indirect parameter expansion".into(),
        });
    }
    if contains_shell_token(&normalized, "eval") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Blocked,
            message: "shell command uses eval-style dynamic execution".into(),
        });
    }

    if normalized.contains("rm -rf") || normalized.contains("rm -fr") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "shell command requires review because it removes files recursively".into(),
        });
    }
    if pipes_into_shell(&normalized) {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "shell command requires review because it pipes output into a shell".into(),
        });
    }
    if contains_shell_token(&normalized, "sudo") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "shell command requires review because it escalates privileges".into(),
        });
    }
    if shell_tokens(&normalized)
        .into_iter()
        .any(|token| token == "mkfs" || token.starts_with("mkfs."))
    {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "shell command requires review because it can reformat a device".into(),
        });
    }
    if normalized.contains("dd if=") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "shell command requires review because it can overwrite raw devices".into(),
        });
    }

    None
}

fn check_shell_safety_for_request(
    request: &PermissionRequest,
    command: &str,
) -> Option<ShellSafetyIssue> {
    if request.matches_tool_name("powershell")
        && let Some(issue) = check_powershell_safety(command)
    {
        return Some(issue);
    }

    check_shell_safety(command)
}

fn check_powershell_safety(command: &str) -> Option<ShellSafetyIssue> {
    let normalized = normalize_shell_command(command);
    let tokens = shell_tokens(&normalized);

    if tokens.iter().any(|token| {
        matches!(*token, "-enc" | "-encodedcommand")
            || token.starts_with("-enc:")
            || token.starts_with("-encodedcommand:")
    }) {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "PowerShell command requires review because it uses encoded parameters".into(),
        });
    }

    let start_process = tokens.contains(&"start-process");
    let run_as = tokens
        .windows(2)
        .any(|pair| pair[0] == "-verb" && pair[1] == "runas")
        || tokens.iter().any(|token| token.starts_with("-verb:runas"));
    if start_process && run_as {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "PowerShell command requires review because it requests elevated execution"
                .into(),
        });
    }

    if tokens
        .iter()
        .any(|token| matches!(*token, "invoke-expression" | "iex"))
    {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message:
                "PowerShell command requires review because it executes dynamic PowerShell code"
                    .into(),
        });
    }

    if tokens.first().is_some_and(|token| {
        matches!(
            *token,
            "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
        )
    }) {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message:
                "PowerShell command requires review because it spawns a nested PowerShell process"
                    .into(),
        });
    }

    None
}

fn normalize_shell_command(command: &str) -> String {
    let mut normalized = String::with_capacity(command.len());
    let mut saw_whitespace = false;

    for ch in command.chars() {
        let ch = match ch {
            '\u{2013}' | '\u{2014}' | '\u{2015}' => '-',
            ';' | '&' | '(' | ')' => ' ',
            _ => ch.to_ascii_lowercase(),
        };

        if ch.is_whitespace() {
            if !saw_whitespace {
                normalized.push(' ');
                saw_whitespace = true;
            }
            continue;
        }

        normalized.push(ch);
        saw_whitespace = false;
    }

    normalized.trim().to_string()
}

fn shell_tokens(command: &str) -> Vec<&str> {
    command
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '|' | '<' | '>'))
        .filter(|token| !token.is_empty())
        .collect()
}

fn contains_shell_token(command: &str, token: &str) -> bool {
    shell_tokens(command).into_iter().any(|part| part == token)
}

fn pipes_into_shell(command: &str) -> bool {
    let mut segments = command.split('|').map(str::trim);
    let Some(_) = segments.next() else {
        return false;
    };

    segments.any(|segment| {
        shell_tokens(segment)
            .first()
            .is_some_and(|token| matches!(*token, "sh" | "bash" | "dash" | "ksh" | "zsh"))
    })
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

fn canonicalize_best_effort(path: &Path) -> PathBuf {
    let normalized = normalize_path(path);
    if let Ok(canonical) = std::fs::canonicalize(&normalized) {
        return normalize_path(&canonical);
    }

    let mut missing = Vec::new();
    let mut existing = normalized.as_path();
    while !existing.exists() {
        let Some(parent) = existing.parent() else {
            return normalized;
        };
        if let Some(name) = existing.file_name() {
            missing.push(name.to_os_string());
        }
        existing = parent;
    }

    let Ok(mut canonical) = std::fs::canonicalize(existing) else {
        return normalized;
    };
    for part in missing.iter().rev() {
        canonical.push(part);
    }
    normalize_path(&canonical)
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

    #[cfg(unix)]
    #[test]
    fn permission_path_escape_via_symlink_is_denied() {
        let root = std::env::temp_dir().join(format!(
            "wonder-permission-symlink-{}",
            uuid::Uuid::new_v4()
        ));
        let project = root.join("project");
        let outside = root.join("outside");
        std::fs::create_dir_all(&project).expect("create project");
        std::fs::create_dir_all(&outside).expect("create outside");
        std::fs::write(outside.join("passwd"), "root:x").expect("write outside file");
        std::os::unix::fs::symlink(&outside, project.join("linked")).expect("create symlink");

        let context = ToolPermissionContext::new(&project, PermissionMode::Default);
        let request = PermissionRequest::new("file_read")
            .read_only(true)
            .with_path("linked/passwd");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Deny { .. }));
        let _ = std::fs::remove_dir_all(root);
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
    fn permission_shell_eval_is_denied() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::Default);
        let request = PermissionRequest::new("bash").with_shell_command("echo ok; eval \"$cmd\"");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn permission_powershell_encoded_command_requires_review() {
        let context = ToolPermissionContext::new("/workspace", PermissionMode::Default);
        let request = PermissionRequest::new("powershell")
            .with_shell_command("pwsh –EncodedCommand ZQBjAGgAbwA=");

        let decision = context.evaluate(&request);

        assert!(matches!(decision, PermissionDecision::Ask { .. }));
        assert!(decision.reason().to_string().contains("encoded"));
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
