//! Implements the `/plan` command — enter, exit, show, and open plan mode.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use async_trait::async_trait;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, PermissionMode, Result, WonderError,
};

use super::workflow::{permission_mode_label, sanitize_single_line};

// ── Public command struct ────────────────────────────────────────────────────

/// Represents plan command
pub struct PlanCommand;

impl PlanCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "plan",
            "Show plan-mode readiness and current permission mode",
            CommandKind::Local,
        )
        .with_argument_hint("[open|<description>]");
        spec.required_features = BTreeSet::from([FeatureFlag::Permissions]);
        spec
    }
}

// ── Internal action enum ─────────────────────────────────────────────────────

#[derive(Debug, Eq, PartialEq)]
pub(super) enum PlanAction {
    Show,
    Enter { queued_prompt: Option<String> },
    Exit,
    Open,
}

// ── Command impl ─────────────────────────────────────────────────────────────

#[async_trait]
impl Command for PlanCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        match parse_plan_action(&invocation.args, context.permission_mode)? {
            PlanAction::Show => Ok(CommandOutput::Text(render_plan_display(
                context.permission_mode,
                &resolve_plan_path(&context.cwd),
            ))),
            PlanAction::Enter { queued_prompt } => Ok(CommandOutput::Text(render_plan_enter(
                context.permission_mode,
                queued_prompt,
            ))),
            // Emit `permission_mode=default` as a conservative fallback.
            // The TUI controller tracks the pre-plan mode independently and
            // restores the correct origin (AcceptEdits, BypassPermissions, …)
            // when it processes this output; the hardcoded `Default` here is
            // only ever used by non-TUI consumers that lack that saved state.
            PlanAction::Exit => Ok(CommandOutput::Text(render_plan_transition(
                PermissionMode::Default,
                "plan mode disabled",
                None,
            ))),
            PlanAction::Open => open_plan_in_editor(&context),
        }
    }
}

// ── Plan action parsing ───────────────────────────────────────────────────────

pub(super) fn parse_plan_action(args: &str, current_mode: PermissionMode) -> Result<PlanAction> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(if matches!(current_mode, PermissionMode::Plan) {
            PlanAction::Show
        } else {
            PlanAction::Enter {
                queued_prompt: None,
            }
        });
    }
    let tokens = shell_words::split(trimmed)
        .map_err(|error| WonderError::validation(format!("invalid /plan arguments: {error}")))?;
    match tokens.first().map(String::as_str) {
        Some("show") if tokens.len() == 1 => Ok(PlanAction::Show),
        Some("exit") if tokens.len() == 1 => Ok(PlanAction::Exit),
        Some("open") if tokens.len() == 1 => Ok(PlanAction::Open),
        Some("enter") => {
            let remainder = trimmed.strip_prefix("enter").unwrap_or_default().trim();
            Ok(PlanAction::Enter {
                queued_prompt: (!remainder.is_empty()).then(|| remainder.to_string()),
            })
        }
        Some("show" | "exit" | "open") => Err(WonderError::validation(
            "`/plan show`, `/plan exit`, and `/plan open` do not take additional arguments",
        )),
        _ => Ok(if matches!(current_mode, PermissionMode::Plan) {
            PlanAction::Show
        } else {
            PlanAction::Enter {
                queued_prompt: Some(trimmed.to_string()),
            }
        }),
    }
}

// ── Path resolution ───────────────────────────────────────────────────────────

pub(super) fn resolve_plan_path(cwd: &Path) -> PathBuf {
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join("plan.md");
        if candidate.is_file() {
            return candidate;
        }
    }
    cwd.join("plan.md")
}

// ── Rendering ─────────────────────────────────────────────────────────────────

/// Renders the output for entering plan mode.
///
/// Includes a `pre_plan_mode=` key that records the permission mode that was
/// active *before* plan mode, giving non-TUI consumers a way to know which
/// mode to restore on exit.  The TUI controller tracks this independently via
/// [`TuiController::pre_plan_permission_mode`].
pub(super) fn render_plan_enter(origin: PermissionMode, queued_prompt: Option<String>) -> String {
    let mut lines = vec![
        format!(
            "permission_mode={}",
            permission_mode_label(PermissionMode::Plan)
        ),
        "plan_mode_ready=true".into(),
        "plan_mode_active=true".into(),
        format!("pre_plan_mode={}", permission_mode_label(origin)),
        "status=plan mode enabled".into(),
    ];
    if let Some(queued_prompt) = queued_prompt.filter(|p| !p.trim().is_empty()) {
        lines.push(format!(
            "enqueue_prompt={}",
            sanitize_single_line(&queued_prompt)
        ));
    }
    lines.join("\n")
}

pub(super) fn render_plan_transition(
    mode: PermissionMode,
    status: &str,
    queued_prompt: Option<String>,
) -> String {
    let mut lines = vec![
        format!("permission_mode={}", permission_mode_label(mode)),
        "plan_mode_ready=true".into(),
        format!("plan_mode_active={}", matches!(mode, PermissionMode::Plan)),
        format!("status={status}"),
    ];
    if let Some(queued_prompt) = queued_prompt.filter(|prompt| !prompt.trim().is_empty()) {
        lines.push(format!(
            "enqueue_prompt={}",
            sanitize_single_line(&queued_prompt)
        ));
    }
    lines.join("\n")
}

pub(super) fn render_plan_display(mode: PermissionMode, plan_path: &Path) -> String {
    let active = matches!(mode, PermissionMode::Plan);
    let mut lines = vec![
        format!("permission_mode={}", permission_mode_label(mode)),
        "plan_mode_ready=true".into(),
        format!("plan_mode_active={active}"),
        format!("plan_path={}", plan_path.display()),
    ];
    match fs::read_to_string(plan_path) {
        Ok(content) => {
            lines.push("plan_exists=true".into());
            lines.push(String::new());
            lines.push("Current Plan".into());
            lines.push(plan_path.display().to_string());
            lines.push(String::new());
            lines.push(content.trim_end().to_string());
            if let Some(editor) = plan_editor_command() {
                lines.push(String::new());
                lines.push(format!(
                    "hint=`/plan open` will launch {} when available outside the live TUI",
                    editor.0
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            lines.push("plan_exists=false".into());
            lines.push(if active {
                "note=already in plan mode. no plan written yet".into()
            } else {
                "note=no plan written yet".into()
            });
        }
        Err(error) => {
            lines.push("plan_exists=false".into());
            lines.push(format!("note=failed to read plan: {error}"));
        }
    }
    lines.join("\n")
}

fn open_plan_in_editor(context: &CommandContext) -> Result<CommandOutput> {
    let plan_path = resolve_plan_path(&context.cwd);
    if !plan_path.is_file() {
        return Ok(CommandOutput::Text(render_plan_display(
            context.permission_mode,
            &plan_path,
        )));
    }
    if context.interactive {
        let mut lines = vec![
            format!(
                "permission_mode={}",
                permission_mode_label(context.permission_mode)
            ),
            "plan_mode_ready=true".into(),
            format!(
                "plan_mode_active={}",
                matches!(context.permission_mode, PermissionMode::Plan)
            ),
            format!("plan_path={}", plan_path.display()),
        ];
        if plan_editor_command().is_some() {
            lines.push("plan_open_external=true".into());
            lines.push("note=launching plan in external editor".into());
        } else {
            lines.push("note=set VISUAL or EDITOR to enable `/plan open`".into());
        }
        return Ok(CommandOutput::Text(lines.join("\n")));
    }
    let Some((editor, args)) = plan_editor_command() else {
        return Ok(CommandOutput::Text(format!(
            concat!(
                "permission_mode={}\n",
                "plan_mode_ready=true\n",
                "plan_mode_active={}\n",
                "plan_path={}\n",
                "note=set VISUAL or EDITOR to enable `/plan open`"
            ),
            permission_mode_label(context.permission_mode),
            matches!(context.permission_mode, PermissionMode::Plan),
            plan_path.display(),
        )));
    };
    let status = ProcessCommand::new(&editor)
        .args(args)
        .arg(&plan_path)
        .current_dir(&context.cwd)
        .status()?;
    Ok(CommandOutput::Text(format!(
        concat!(
            "permission_mode={}\n",
            "plan_mode_ready=true\n",
            "plan_mode_active={}\n",
            "plan_path={}\n",
            "editor={}\n",
            "editor_success={}"
        ),
        permission_mode_label(context.permission_mode),
        matches!(context.permission_mode, PermissionMode::Plan),
        plan_path.display(),
        editor,
        status.success(),
    )))
}

/// Resolves the user's preferred editor from `VISUAL` or `EDITOR`.
///
/// Returns `(program, extra_args)` when one of those variables is set to a
/// non-empty value, or `None` when neither is configured.
pub(crate) fn plan_editor_command() -> Option<(String, Vec<String>)> {
    let raw = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })?;
    let mut tokens = shell_words::split(&raw).ok()?;
    let command = tokens.first()?.clone();
    Some((command, tokens.drain(1..).collect()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wonder_of_u_core::PermissionMode;
    use wonder_of_u_test_support::unique_test_dir;

    use super::{
        PlanAction, PlanCommand, parse_plan_action, render_plan_display, resolve_plan_path,
    };

    #[test]
    fn plan_defaults_to_enter_outside_plan_mode_and_show_inside() {
        assert_eq!(
            parse_plan_action("", PermissionMode::Default).expect("default plan action"),
            PlanAction::Enter {
                queued_prompt: None
            }
        );
        assert_eq!(
            parse_plan_action("", PermissionMode::Plan).expect("plan-mode plan action"),
            PlanAction::Show
        );
        assert_eq!(
            parse_plan_action(
                "draft a detailed migration checklist",
                PermissionMode::Default
            )
            .expect("plan prompt action"),
            PlanAction::Enter {
                queued_prompt: Some("draft a detailed migration checklist".into())
            }
        );
    }

    #[test]
    fn plan_display_reads_nearest_plan_file() {
        let dir = unique_test_dir("workflow-plan-display");
        let nested = dir.join("nested/project");
        std::fs::create_dir_all(&nested).expect("create nested");
        let plan_path = dir.join("plan.md");
        std::fs::write(&plan_path, "# test plan\n- keep `/plan` parity\n").expect("write plan");

        let resolved = resolve_plan_path(&nested);
        assert_eq!(resolved, plan_path);

        let rendered = render_plan_display(PermissionMode::Plan, &resolved);
        assert!(rendered.contains("plan_exists=true"));
        assert!(rendered.contains("Current Plan"));
        assert!(rendered.contains("- keep `/plan` parity"));
    }

    #[test]
    fn render_plan_enter_includes_pre_plan_mode() {
        let output = super::render_plan_enter(PermissionMode::AcceptEdits, None);
        assert!(
            output.contains("permission_mode=plan"),
            "should set plan mode"
        );
        assert!(output.contains("plan_mode_active=true"));
        assert!(
            output.contains("pre_plan_mode=accept-edits"),
            "should record origin mode"
        );
        assert!(output.contains("status=plan mode enabled"));
    }

    #[test]
    fn render_plan_enter_records_bypass_origin() {
        let output = super::render_plan_enter(PermissionMode::BypassPermissions, None);
        assert!(output.contains("pre_plan_mode=bypass-permissions"));
    }

    #[test]
    fn render_plan_enter_records_default_origin() {
        let output = super::render_plan_enter(PermissionMode::Default, None);
        assert!(output.contains("pre_plan_mode=default"));
        assert!(output.contains("permission_mode=plan"));
    }

    #[test]
    fn render_plan_enter_includes_queued_prompt() {
        let output =
            super::render_plan_enter(PermissionMode::Default, Some("outline the steps".into()));
        assert!(output.contains("enqueue_prompt=outline the steps"));
    }

    #[test]
    fn render_plan_transition_exit_emits_default_fallback() {
        // The exit arm emits `permission_mode=default` as a conservative fallback.
        // TUI controller overrides this with the saved pre-plan mode; non-TUI
        // consumers see `default` and can treat it as a safe starting point.
        let output =
            super::render_plan_transition(PermissionMode::Default, "plan mode disabled", None);
        assert!(output.contains("permission_mode=default"));
        assert!(output.contains("plan_mode_active=false"));
        assert!(output.contains("status=plan mode disabled"));
    }

    #[test]
    fn plan_command_spec_carries_argument_hint() {
        let spec = PlanCommand::command_spec();
        assert_eq!(
            spec.argument_hint.as_deref(),
            Some("[open|<description>]"),
            "/plan spec should carry the argument hint"
        );
    }
}
