//! Implements the `/hooks` command together with its config-file structs,
//! rendering, and template-writing helpers.
//!
//! The public surface is intentionally narrow:
//! * [`HooksCommand`] — the command registered in the command registry.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::Arc,
};

use async_trait::async_trait;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::json;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
    ToolSpec, WonderError,
};
use wonder_of_u_storage::StoragePaths;

use super::parse_command_args;
use super::plan_command::plan_editor_command;

// ── Command struct ────────────────────────────────────────────────────────────

/// Handles `/hooks [show|open]` — views or edits the hook configuration.
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
    Show,
    Open,
}

// ── Hooks config structs ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct HooksConfig {
    #[serde(default)]
    disable_all_hooks: bool,
    #[serde(default)]
    allow_managed_hooks_only: bool,
    #[serde(default)]
    hooks: BTreeMap<String, Vec<HookMatcherConfig>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct HookMatcherConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    matcher: Option<String>,
    #[serde(default)]
    hooks: Vec<HookActionConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HookActionConfig {
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    Prompt {
        prompt: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    Agent {
        prompt: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    Http {
        url: String,
        #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
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
        match args.command.unwrap_or(HooksSubcommand::Show) {
            HooksSubcommand::Show => Ok(CommandOutput::Text(render_hooks_summary(
                self.storage_dir.as_deref(),
                self.tool_specs.as_ref(),
            )?)),
            HooksSubcommand::Open => {
                hooks_open_output(&context, self.storage_dir.as_deref()).map(CommandOutput::Text)
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn render_hooks_summary(storage_dir: Option<&Path>, tool_specs: &[ToolSpec]) -> Result<String> {
    let path = resolve_hooks_path(storage_dir);
    let config = read_hooks_config(&path)?;
    let matcher_count = config.hooks.values().map(Vec::len).sum::<usize>();
    let hook_count = config
        .hooks
        .values()
        .flat_map(|matchers| matchers.iter())
        .map(|matcher| matcher.hooks.len())
        .sum::<usize>();
    let mut lines = vec![
        "## Hooks".into(),
        format!("config_path={}", path.display()),
        format!("available_tools={}", tool_specs.len()),
        format!("events={}", config.hooks.len()),
        format!("matchers={matcher_count}"),
        format!("hooks={hook_count}"),
        format!("disable_all_hooks={}", config.disable_all_hooks),
        format!(
            "allow_managed_hooks_only={}",
            config.allow_managed_hooks_only
        ),
    ];
    if config.hooks.is_empty() {
        lines.push("No hooks configured yet.".into());
    } else {
        for (event, matchers) in &config.hooks {
            let configured_hooks = matchers
                .iter()
                .map(|matcher| matcher.hooks.len())
                .sum::<usize>();
            lines.push(format!(
                "- {event}: {} matcher(s), {configured_hooks} hook(s)",
                matchers.len()
            ));
            if let Some(summary) = hook_event_summary(event) {
                lines.push(format!("  {summary}"));
            }
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "Edit {} with `/hooks open` to review or update the config template.",
        path.display()
    ));
    lines.push(
        "Note: hook execution is not yet wired into the Rust runtime; this command currently provides config parity only."
            .into(),
    );
    Ok(lines.join("\n"))
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

fn resolve_hooks_path(storage_dir: Option<&Path>) -> PathBuf {
    storage_dir
        .map(StoragePaths::new)
        .map(|paths| paths.config_dir())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".wonder-of-u")
                .join("config")
        })
        .join("hooks.json")
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

fn read_hooks_config(path: &Path) -> Result<HooksConfig> {
    if !path.exists() {
        return Ok(HooksConfig::default());
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|error| {
        WonderError::validation(format!(
            "invalid hooks config `{}`: {error}",
            path.display()
        ))
    })
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
    use wonder_of_u_core::{CommandContext, FeatureSet, PermissionMode, SessionId};
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::{ensure_hooks_file, render_hooks_summary, resolve_hooks_path};

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
        assert!(rendered.contains("config parity only"));
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
}
