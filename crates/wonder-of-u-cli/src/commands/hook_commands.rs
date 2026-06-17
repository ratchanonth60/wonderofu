//! Implements the `/hooks` command together with its config-file structs,
//! rendering, and template-writing helpers.
//!
//! The public surface is intentionally narrow:
//! * [`HooksCommand`] — the command registered in the command registry.

use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::Arc,
};

use async_trait::async_trait;
use clap::{Parser, Subcommand};
use serde_json::json;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
    ToolSpec,
};

use super::hook_trust::{
    HookEntry, HookTrustStatus, HooksInventory, load_hooks_inventory, resolve_hooks_path,
    set_hook_disabled, trust_hook,
};
use super::parse_command_args;
use super::plan_command::plan_editor_command;

// ── Command struct ────────────────────────────────────────────────────────────

/// Handles `/hooks` review, trust, and config editing commands.
pub struct HooksCommand {
    storage_dir: Option<PathBuf>,
    tool_specs: Arc<[ToolSpec]>,
}

impl HooksCommand {
    /// Creates a new `HooksCommand`.
    pub fn new(storage_dir: Option<PathBuf>, tool_specs: Arc<[ToolSpec]>) -> Self {
        Self {
            storage_dir,
            tool_specs,
        }
    }

    /// Returns the [`CommandSpec`] for this command.
    pub fn command_spec() -> CommandSpec {
        // immediate=true: /hooks must not drain queued prompts after executing.
        CommandSpec::new(
            "hooks",
            "View hook configurations for tool events",
            CommandKind::Local,
        )
        .with_immediate(true)
    }
}

// ── Clap arg structs ──────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct HooksArgs {
    #[command(subcommand)]
    command: Option<HooksSubcommand>,
}

#[derive(Debug, Subcommand)]
enum HooksSubcommand {
    List,
    Review,
    Show,
    Open,
    Trust(HookIdArgs),
    Disable(HookIdArgs),
    Enable(HookIdArgs),
}

#[derive(Debug, Parser)]
struct HookIdArgs {
    id: String,
}

// ── Command impl ──────────────────────────────────────────────────────────────

#[async_trait]
impl Command for HooksCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<HooksArgs>("hooks", &invocation)?;
        match args.command.unwrap_or(HooksSubcommand::Review) {
            HooksSubcommand::List => Ok(CommandOutput::Text(render_hooks_summary(
                self.storage_dir.as_deref(),
                self.tool_specs.as_ref(),
            )?)),
            HooksSubcommand::Review | HooksSubcommand::Show => Ok(CommandOutput::Text(
                render_hooks_summary(self.storage_dir.as_deref(), self.tool_specs.as_ref())?,
            )),
            HooksSubcommand::Open => {
                hooks_open_output(&context, self.storage_dir.as_deref()).map(CommandOutput::Text)
            }
            HooksSubcommand::Trust(args) => {
                let entry = trust_hook(self.storage_dir.as_deref(), &args.id)?;
                Ok(CommandOutput::Text(render_hook_mutation("trusted", &entry)))
            }
            HooksSubcommand::Disable(args) => {
                let entry = set_hook_disabled(self.storage_dir.as_deref(), &args.id, true)?;
                Ok(CommandOutput::Text(render_hook_mutation(
                    "disabled", &entry,
                )))
            }
            HooksSubcommand::Enable(args) => {
                let entry = set_hook_disabled(self.storage_dir.as_deref(), &args.id, false)?;
                Ok(CommandOutput::Text(render_hook_mutation("enabled", &entry)))
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn render_hooks_summary(storage_dir: Option<&Path>, tool_specs: &[ToolSpec]) -> Result<String> {
    let inventory = load_hooks_inventory(storage_dir)?;
    let hook_count = inventory.entries.len();
    let mut lines = vec![
        "## Hooks".into(),
        format!("config_path={}", inventory.config_path.display()),
        format!("state_path={}", inventory.state_path.display()),
        format!("available_tools={}", tool_specs.len()),
        format!("events={}", inventory.event_count),
        format!("matchers={}", inventory.matcher_count),
        format!("hooks={hook_count}"),
        format!(
            "trusted={}",
            count_status(&inventory, HookTrustStatus::Trusted)
        ),
        format!(
            "untrusted={}",
            count_status(&inventory, HookTrustStatus::Untrusted)
        ),
        format!(
            "disabled={}",
            count_status(&inventory, HookTrustStatus::Disabled)
        ),
        format!(
            "changed={}",
            count_status(&inventory, HookTrustStatus::Changed)
        ),
        format!(
            "managed={}",
            inventory
                .entries
                .iter()
                .filter(|entry| entry.managed)
                .count()
        ),
        format!(
            "unsupported={}",
            inventory
                .entries
                .iter()
                .filter(|entry| !entry.supported)
                .count()
        ),
        format!("disable_all_hooks={}", inventory.disable_all_hooks),
        format!(
            "allow_managed_hooks_only={}",
            inventory.allow_managed_hooks_only
        ),
    ];
    if inventory.entries.is_empty() {
        lines.push("No hooks configured yet.".into());
    } else {
        let mut last_event: Option<&str> = None;
        for (index, entry) in inventory.entries.iter().enumerate() {
            if last_event != Some(entry.event.as_str()) {
                last_event = Some(entry.event.as_str());
                lines.push(format!("- {}", entry.event));
                if let Some(summary) = hook_event_summary(&entry.event) {
                    lines.push(format!("  {summary}"));
                }
            }
            lines.push(render_hook_entry(index, entry));
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "Edit {} with `/hooks open` to review or update the config template.",
        inventory.config_path.display()
    ));
    lines.push(
        "Note: Command hooks are active only after `/hooks trust <id>`. New, changed, disabled, or unmanaged hooks in managed-only mode are skipped. Prompt/Agent/Http hooks are parsed but reported as unsupported by the executor."
            .into(),
    );
    Ok(lines.join("\n"))
}

fn count_status(inventory: &HooksInventory, status: HookTrustStatus) -> usize {
    inventory
        .entries
        .iter()
        .filter(|entry| entry.status == status)
        .count()
}

fn render_hook_entry(index: usize, entry: &HookEntry) -> String {
    format!(
        "  hook[{index}] id={} status={} matcher={} type={} managed={} fingerprint={} target={}",
        entry.id,
        entry.status_tags().join(","),
        sanitize_inline(&entry.matcher),
        entry.kind,
        entry.managed,
        short_fingerprint(&entry.fingerprint),
        sanitize_inline(&entry.target),
    )
}

fn render_hook_mutation(action: &str, entry: &HookEntry) -> String {
    [
        format!("hook={}", entry.id),
        format!("action={action}"),
        format!("status={}", entry.status.label()),
        format!("fingerprint={}", entry.fingerprint),
        format!("managed={}", entry.managed),
        format!("supported={}", entry.supported),
    ]
    .join("\n")
}

fn sanitize_inline(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\n' | '\r' | '\t' => ' ',
            _ => ch,
        })
        .collect()
}

fn short_fingerprint(fingerprint: &str) -> &str {
    fingerprint.get(..12).unwrap_or(fingerprint)
}

fn hooks_open_output(context: &CommandContext, storage_dir: Option<&Path>) -> Result<String> {
    let path = resolve_hooks_path(storage_dir);
    let existed = path.exists();
    ensure_hooks_file(&path)?;
    if context.interactive {
        let mut lines = vec![
            "hooks=ready".into(),
            format!("hooks_path={}", path.display()),
            format!("created={}", !existed),
        ];
        if plan_editor_command().is_some() {
            lines.push("open_external=true".into());
            lines.push(format!("external_path={}", path.display()));
            lines.push("status=opening hooks config".into());
        } else {
            lines.push("note=set VISUAL or EDITOR to enable `/hooks open`".into());
        }
        return Ok(lines.join("\n"));
    }
    let Some((editor, args)) = plan_editor_command() else {
        return Ok(format!(
            concat!(
                "hooks=ready\n",
                "hooks_path={}\n",
                "created={}\n",
                "note=set VISUAL or EDITOR to enable `/hooks open`"
            ),
            path.display(),
            !existed,
        ));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&path)
        .current_dir(&context.cwd)
        .status()?;
    Ok(format!(
        concat!(
            "hooks=ready\n",
            "hooks_path={}\n",
            "created={}\n",
            "editor={}\n",
            "editor_success={}"
        ),
        path.display(),
        !existed,
        editor,
        status.success(),
    ))
}

fn ensure_hooks_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(path, render_hooks_template())?;
    } else {
        OpenOptions::new().create(true).append(true).open(path)?;
    }
    Ok(())
}

fn render_hooks_template() -> String {
    serde_json::to_string_pretty(&json!({
        "disable_all_hooks": false,
        "allow_managed_hooks_only": false,
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "bash",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "echo auditing bash tool invocation"
                        }
                    ]
                }
            ],
            "Notification": [
                {
                    "matcher": "auth_success",
                    "hooks": [
                        {
                            "type": "http",
                            "url": "https://example.invalid/hooks/notify"
                        }
                    ]
                }
            ]
        }
    }))
    .unwrap_or_else(|_| "{\"hooks\":{}}".into())
        + "\n"
}

fn hook_event_summary(event: &str) -> Option<&'static str> {
    match event {
        "PreToolUse" => Some("Before tool execution; match on tool_name."),
        "PostToolUse" => Some("After tool execution; match on tool_name."),
        "PostToolUseFailure" => Some("After a tool fails; match on tool_name."),
        "PermissionDenied" => Some("After auto mode denies a tool call; match on tool_name."),
        "Notification" => Some("When notifications are sent; match on notification_type."),
        "UserPromptSubmit" => Some("When the user submits a prompt."),
        "SessionStart" => Some("When a session starts; match on source."),
        "Stop" => Some("Right before the assistant concludes a response."),
        "StopFailure" => Some("When a turn ends because of an API error."),
        "SubagentStart" => Some("When a subagent starts; match on agent_type."),
        "SubagentStop" => Some("Right before a subagent concludes; match on agent_type."),
        "PreCompact" => Some("Before compaction; match on trigger."),
        "PostCompact" => Some("After compaction; match on trigger."),
        "SessionEnd" => Some("When a session ends; match on reason."),
        "PermissionRequest" => Some("When a permission dialog is displayed; match on tool_name."),
        "Setup" => Some("Repo setup hooks for init and maintenance."),
        "TeammateIdle" => Some("When a teammate is about to go idle."),
        "TaskCreated" => Some("When a task is created."),
        "TaskCompleted" => Some("When a task is completed."),
        "Elicitation" => Some("When an MCP server requests user input."),
        "ElicitationResult" => Some("After a user responds to an MCP elicitation."),
        "ConfigChange" => Some("When configuration files change during a session."),
        "InstructionsLoaded" => Some("When an instruction file is loaded."),
        "WorktreeCreate" => Some("When a worktree should be created."),
        "WorktreeRemove" => Some("When a worktree should be removed."),
        "CwdChanged" => Some("After the working directory changes."),
        "FileChanged" => Some("When a watched file changes."),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::executor::block_on;
    use wonder_of_u_core::{
        Command, CommandContext, CommandInvocation, CommandOutput, FeatureSet, PermissionMode,
        SessionId,
    };
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::{HooksCommand, ensure_hooks_file, render_hooks_summary, resolve_hooks_path};

    fn test_context(cwd: &std::path::Path) -> CommandContext {
        CommandContext {
            session_id: SessionId::new(),
            cwd: cwd.to_path_buf(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            optimize_token_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    #[test]
    fn hooks_summary_reports_template_events() {
        let dir = unique_test_dir("workflow-hooks-template");
        let path = dir.join("config/hooks.json");
        ensure_hooks_file(&path).expect("write hooks template");

        let rendered = render_hooks_summary(Some(dir.as_path()), &[]).expect("render hooks");

        assert!(rendered.contains("## Hooks"));
        assert!(rendered.contains("events=2"));
        assert!(rendered.contains("PreToolUse"));
        assert!(rendered.contains("Notification"));
        assert!(rendered.contains("untrusted=2"));
        assert!(rendered.contains("Command hooks are active"));
        assert!(rendered.contains("Prompt/Agent/Http hooks are parsed"));
    }

    #[test]
    fn hooks_command_lists_trusts_disables_and_enables_hooks() {
        let dir = unique_test_dir("workflow-hooks-command-trust");
        let path = dir.join("config/hooks.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"hooks": {"PreToolUse": [{"matcher": "bash", "hooks": [{"type": "command", "command": "true"}]}]}}"#,
        )
        .unwrap();
        let command = HooksCommand::new(Some(dir.clone()), Arc::from([]));
        let context = test_context(dir.as_path());

        let list = execute_hooks(&command, &context, "list");
        assert!(list.contains("hook[0] id=pretooluse.0.0"));
        assert!(list.contains("untrusted=1"));

        let trusted = execute_hooks(&command, &context, "trust pretooluse.0.0");
        assert!(trusted.contains("action=trusted"));
        assert!(trusted.contains("status=trusted"));

        let disabled = execute_hooks(&command, &context, "disable pretooluse.0.0");
        assert!(disabled.contains("action=disabled"));
        assert!(disabled.contains("status=disabled"));

        let enabled = execute_hooks(&command, &context, "enable pretooluse.0.0");
        assert!(enabled.contains("action=enabled"));
        assert!(enabled.contains("status=trusted"));
    }

    #[test]
    fn hooks_open_hints_external_editor_when_interactive() {
        let dir = unique_test_dir("workflow-hooks-open");
        let _editor = EnvVarGuard::set("EDITOR", "true");
        let context = CommandContext {
            session_id: SessionId::new(),
            cwd: dir.clone(),
            features: FeatureSet::default(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            optimize_token_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        };
        let output = super::hooks_open_output(&context, Some(dir.as_path())).expect("open output");

        assert!(output.contains("open_external=true"));
        assert!(output.contains(&format!(
            "external_path={}",
            resolve_hooks_path(Some(dir.as_path())).display()
        )));
    }

    // Suppress unused import warning for `test_context` helper — kept for
    // future tests that need a full CommandContext.
    #[allow(dead_code)]
    fn _uses_test_context() {
        let _ = test_context(std::path::Path::new("/workspace"));
    }

    fn execute_hooks(command: &HooksCommand, context: &CommandContext, args: &str) -> String {
        let output = block_on(command.execute(
            context.clone(),
            CommandInvocation {
                name: "hooks".into(),
                args: args.into(),
                raw: format!("/hooks {args}"),
            },
        ))
        .expect("hooks command");
        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        text
    }
}
