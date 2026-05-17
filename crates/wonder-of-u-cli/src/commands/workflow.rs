//! Implements the core session commands: `/permissions` and `/exit`,
//! plus the small set of helpers shared across sibling command modules.
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, PermissionDecision, PermissionMode, PermissionRequest, PermissionRuleSource,
    Result, ToolPermissionContext, ToolSpec, WonderError,
};

use super::parse_command_args;

// \u2500\u2500 PermissionsCommand \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500

/// Inspects and updates the session permission mode.
pub struct PermissionsCommand {
    tool_specs: Arc<[ToolSpec]>,
}

impl PermissionsCommand {
    /// Creates a new `PermissionsCommand` backed by the given tool registry.
    pub fn new(tool_specs: Arc<[ToolSpec]>) -> Self {
        Self { tool_specs }
    }

    /// Returns the `CommandSpec` for `/permissions`.
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "permissions",
            "Inspect permission defaults and evaluate tool requests",
            CommandKind::Local,
        );
        spec.required_features = BTreeSet::from([FeatureFlag::Permissions]);
        spec
    }
}

#[derive(Debug, Parser)]
struct PermissionsArgs {
    #[command(subcommand)]
    command: Option<PermissionsSubcommand>,
}

#[derive(Debug, Subcommand)]
enum PermissionsSubcommand {
    Show,
    Set(PermissionSetArgs),
    Check(PermissionCheckArgs),
}

#[derive(Debug, Args)]
struct PermissionSetArgs {
    #[arg()]
    mode: String,
}

#[derive(Debug, Args)]
struct PermissionCheckArgs {
    #[arg(long)]
    tool: String,
    #[arg(long = "alias")]
    aliases: Vec<String>,
    #[arg(long)]
    read_only: bool,
    #[arg(long)]
    destructive: bool,
    #[arg(long = "path")]
    paths: Vec<PathBuf>,
    #[arg(long = "shell")]
    shell_command: Option<String>,
    #[arg(long = "mode")]
    mode: Option<String>,
    #[arg(long = "add-dir")]
    add_dirs: Vec<PathBuf>,
}

#[async_trait]
impl Command for PermissionsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let normalized_invocation = normalize_permissions_invocation(&invocation);
        let args = parse_command_args::<PermissionsArgs>("permissions", &normalized_invocation)?;
        match args.command.unwrap_or(PermissionsSubcommand::Show) {
            PermissionsSubcommand::Show => {
                if context.interactive && invocation.args.trim().is_empty() {
                    Ok(CommandOutput::Text(render_permission_picker(
                        context.permission_mode,
                    )))
                } else {
                    self.show(context)
                }
            }
            PermissionsSubcommand::Set(args) => Ok(CommandOutput::Text(
                render_permission_transition(parse_permission_mode(&args.mode)?),
            )),
            PermissionsSubcommand::Check(args) => self.check(context, args),
        }
    }
}

impl PermissionsCommand {
    fn show(&self, context: CommandContext) -> Result<CommandOutput> {
        let mut lines = vec![
            format!(
                "permission_mode={}",
                permission_mode_label(context.permission_mode)
            ),
            format!("working_directory={}", context.cwd.display()),
            format!(
                "additional_working_directories={}",
                context.additional_working_directories.len()
            ),
            "rules=0".into(),
            format!("tools={}", self.tool_specs.len()),
        ];
        for directory in &context.additional_working_directories {
            lines.push(format!(
                "working_directory_addition={};source={}",
                directory.path.display(),
                directory.source.label()
            ));
        }
        for spec in self.tool_specs.iter() {
            let aliases = if spec.aliases.is_empty() {
                "-".into()
            } else {
                spec.aliases.join(",")
            };
            lines.push(format!(
                "tool[{}]=kind={:?};aliases={};read_only={};destructive={}",
                spec.name, spec.kind, aliases, spec.read_only, spec.destructive
            ));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    fn check(&self, context: CommandContext, args: PermissionCheckArgs) -> Result<CommandOutput> {
        let spec = self.resolve_tool_spec(&args.tool);
        let mut permission_context = ToolPermissionContext::new(
            &context.cwd,
            args.mode
                .as_deref()
                .map(parse_permission_mode)
                .transpose()?
                .unwrap_or(context.permission_mode),
        );
        for dir in args.add_dirs {
            permission_context =
                permission_context.with_additional_directory(dir, PermissionRuleSource::CliArg);
        }
        for directory in &context.additional_working_directories {
            permission_context = permission_context
                .with_additional_directory(directory.path.clone(), directory.source);
        }

        let mut request = PermissionRequest::new(args.tool)
            .with_aliases(args.aliases)
            .read_only(args.read_only || spec.is_some_and(|spec| spec.read_only))
            .destructive(args.destructive || spec.is_some_and(|spec| spec.destructive))
            .with_paths(args.paths);
        if let Some(spec) = spec {
            request = request.with_aliases(spec.aliases.iter().cloned());
        }
        if let Some(shell_command) = args.shell_command {
            request = request.with_shell_command(shell_command);
        }

        let decision = permission_context.evaluate(&request);
        Ok(CommandOutput::Text(render_permission_decision(
            &permission_context,
            &decision,
        )))
    }

    fn resolve_tool_spec(&self, tool_name: &str) -> Option<&ToolSpec> {
        self.tool_specs.iter().find(|spec| {
            spec.name.eq_ignore_ascii_case(tool_name)
                || spec
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(tool_name))
        })
    }
}

// \u2500\u2500 ExitCommand \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500

/// Requests orderly CLI exit.
pub struct ExitCommand;

impl ExitCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Returns the `CommandSpec` for `/exit`.
    pub fn command_spec() -> CommandSpec {
        // immediate=true: /exit must not drain queued prompts after executing.
        CommandSpec::new("exit", "Request CLI exit", CommandKind::Local).with_immediate(true)
    }
}

#[async_trait]
impl Command for ExitCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::ExitRequested)
    }
}

// \u2500\u2500 Permission render helpers \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500

fn render_permission_decision(
    context: &ToolPermissionContext,
    decision: &PermissionDecision,
) -> String {
    format!(
        concat!(
            "permission_mode={}\n",
            "working_directories={}\n",
            "decision={}\n",
            "reason={}"
        ),
        permission_mode_label(context.mode),
        context
            .working_directories()
            .into_iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(","),
        decision_label(decision),
        decision.reason(),
    )
}

fn render_permission_transition(mode: PermissionMode) -> String {
    format!(
        concat!(
            "permission_mode={}\n",
            "status=permission mode updated\n",
            "plan_mode_active={}"
        ),
        permission_mode_label(mode),
        matches!(mode, PermissionMode::Plan),
    )
}

fn render_permission_picker(current_mode: PermissionMode) -> String {
    let mut lines = vec![
        "permission_picker=true".into(),
        format!("permission_mode={}", permission_mode_label(current_mode)),
    ];
    for (mode, label, description) in [
        (
            PermissionMode::Default,
            "Default",
            "Ask before edits or destructive commands.",
        ),
        (
            PermissionMode::AcceptEdits,
            "Accept edits",
            "Allow edits automatically but still ask for risky actions.",
        ),
        (
            PermissionMode::BypassPermissions,
            "Bypass permissions",
            "Allow all tools and shell actions without prompts.",
        ),
        (
            PermissionMode::DontAsk,
            "Don\'t ask",
            "Deny actions that would normally require approval.",
        ),
        (
            PermissionMode::Plan,
            "Plan mode",
            "Switch into planning-oriented permission handling.",
        ),
    ] {
        lines.push(format!(
            "permission_option={}",
            json!({
                "mode": permission_mode_label(mode),
                "label": label,
                "description": description,
                "selected": mode == current_mode,
            })
        ));
    }
    lines.join("\n")
}

fn normalize_permissions_invocation(invocation: &CommandInvocation) -> CommandInvocation {
    let trimmed = invocation.args.trim();
    if trimmed.is_empty()
        || trimmed == "show"
        || trimmed.starts_with("show ")
        || trimmed == "check"
        || trimmed.starts_with("check ")
        || trimmed == "set"
        || trimmed.starts_with("set ")
    {
        return invocation.clone();
    }
    CommandInvocation {
        name: invocation.name.clone(),
        args: format!("set {}", invocation.args),
        raw: invocation.raw.clone(),
    }
}

// \u2500\u2500 Shared helpers (used by sibling modules) \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500

/// Collapses multi-line text into a single line by joining with `\\n`.
///
/// Used in command output fields to guarantee one key=value pair per line.
pub(crate) fn sanitize_single_line(value: &str) -> String {
    value.lines().collect::<Vec<_>>().join("\\n")
}

/// Parses a permission mode string into a [`PermissionMode`].
pub(crate) fn parse_permission_mode(value: &str) -> Result<PermissionMode> {
    match value {
        "default" => Ok(PermissionMode::Default),
        "accept-edits" | "acceptEdits" => Ok(PermissionMode::AcceptEdits),
        "bypass-permissions" | "bypassPermissions" => Ok(PermissionMode::BypassPermissions),
        "dont-ask" | "dontAsk" => Ok(PermissionMode::DontAsk),
        "plan" => Ok(PermissionMode::Plan),
        other => Err(WonderError::validation(format!(
            "unknown permission mode: {other}"
        ))),
    }
}

/// Returns the canonical string label for a [`PermissionMode`].
pub(crate) fn permission_mode_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept-edits",
        PermissionMode::BypassPermissions => "bypass-permissions",
        PermissionMode::DontAsk => "dont-ask",
        PermissionMode::Plan => "plan",
    }
}

fn decision_label(decision: &PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Allow { .. } => "allow",
        PermissionDecision::Ask { .. } => "ask",
        PermissionDecision::Deny { .. } => "deny",
    }
}

#[cfg(test)]
mod tests {
    use wonder_of_u_core::CommandInvocation;

    use super::normalize_permissions_invocation;

    #[test]
    fn permissions_shorthand_normalizes_to_set_subcommand() {
        let invocation = CommandInvocation {
            name: "permissions".into(),
            args: "accept-edits".into(),
            raw: "/permissions accept-edits".into(),
        };

        let normalized = normalize_permissions_invocation(&invocation);

        assert_eq!(normalized.args, "set accept-edits");
    }
}
