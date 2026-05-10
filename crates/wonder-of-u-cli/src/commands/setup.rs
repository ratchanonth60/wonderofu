//! Provides the `/setup` command surface.
//!
//! In interactive (TUI) mode the command emits a machine-readable
//! `setup_menu` payload that the TUI controller can parse to open the
//! setup-hub overlay.  In non-interactive mode it prints a readable
//! provider/readiness summary alongside usage hints.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;
use wonder_of_u_agent::ProviderResolver;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
};

/// Handles the `/setup` slash command.
///
/// Executing this command without arguments in the TUI opens a setup
/// hub overlay.  Running it outside the TUI prints a provider/readiness
/// summary and guided next steps.
pub struct SetupCommand {
    storage_dir: Option<PathBuf>,
}

impl SetupCommand {
    /// Creates a new `SetupCommand`.
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Returns the [`CommandSpec`] used to register and display this command.
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "setup",
            "Open the setup hub to configure providers, models, theme, and more",
            CommandKind::Local,
        );
        // "setup" is the canonical entry point; accept a common alias.
        spec.aliases.push("settings".into());
        spec
    }
}

#[async_trait]
impl Command for SetupCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;

        if context.interactive {
            Ok(CommandOutput::Text(render_setup_menu(&report)))
        } else {
            Ok(CommandOutput::Text(render_setup_guidance(&report)))
        }
    }
}

// ---------------------------------------------------------------------------
// Interactive payload
// ---------------------------------------------------------------------------

/// Renders the machine-readable setup menu payload consumed by the TUI
/// controller to open the setup overlay.
///
/// The payload begins with a `setup_menu=true` sentinel followed by provider
/// context fields, then one `setup_item=<json>` line per first-slice menu
/// entry.  The controller should treat these lines as an ordered list of
/// actions available in the setup hub.
fn render_setup_menu(report: &wonder_of_u_agent::ProviderStatusReport) -> String {
    let mut lines = vec![
        "setup_menu=true".to_owned(),
        format!(
            "provider_selection={}",
            report
                .selection_label()
                .unwrap_or_else(|| "unconfigured".into())
        ),
        format!("provider_readiness={}", report.readiness.label()),
    ];

    for item in setup_menu_items() {
        lines.push(format!(
            "setup_item={}",
            json!({
                "id":          item.id,
                "label":       item.label,
                "description": item.description,
                "command":     item.command,
                "icon":        item.icon,
            })
        ));
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Non-interactive guidance
// ---------------------------------------------------------------------------

/// Renders a human-readable provider/readiness summary and setup hints
/// for non-interactive (pipe / CI) invocations.
fn render_setup_guidance(report: &wonder_of_u_agent::ProviderStatusReport) -> String {
    let selection = report
        .selection_label()
        .unwrap_or_else(|| "unconfigured".into());
    let readiness = report.readiness.label();

    let mut lines = vec![
        "=== Wonder of U — Setup Guide ===".to_owned(),
        String::new(),
        format!("provider_selection : {selection}"),
        format!("provider_readiness : {readiness}"),
        String::new(),
        "Available setup steps (run each slash command inside the TUI):".to_owned(),
    ];

    for item in setup_menu_items() {
        lines.push(format!("  /{:<16}  {}", item.command, item.description));
    }

    lines.push(String::new());
    lines.push(
        "Tip: launch the TUI with `wonder-of-u` and type /setup to open the interactive hub."
            .to_owned(),
    );

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Menu item catalogue
// ---------------------------------------------------------------------------

struct SetupMenuItem {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    /// The slash command that this item dispatches to.
    command: &'static str,
    icon: &'static str,
}

/// Returns the ordered list of first-slice setup menu items.
///
/// The order matches the plan: provider auth first, then model/API, then UX.
fn setup_menu_items() -> [SetupMenuItem; 9] {
    [
        SetupMenuItem {
            id: "login",
            label: "Provider login",
            description: "Store an API key for Anthropic, OpenAI, or another provider.",
            command: "login",
            icon: "key",
        },
        SetupMenuItem {
            id: "copilot-oauth",
            label: "Copilot OAuth",
            description: "Authenticate via GitHub Copilot device-code OAuth flow.",
            command: "login",
            icon: "oauth",
        },
        SetupMenuItem {
            id: "model",
            label: "Model selection",
            description: "Choose the active provider and model.",
            command: "model",
            icon: "model",
        },
        SetupMenuItem {
            id: "api-base",
            label: "API base override",
            description: "Override the API endpoint URL for a provider.",
            command: "config",
            icon: "api",
        },
        SetupMenuItem {
            id: "theme",
            label: "Theme",
            description: "Choose a TUI colour theme (default, midnight, light).",
            command: "theme",
            icon: "theme",
        },
        SetupMenuItem {
            id: "permissions",
            label: "Permission mode",
            description: "Set tool permission policy (default, accept-edits, bypass, plan).",
            command: "permissions",
            icon: "lock",
        },
        SetupMenuItem {
            id: "terminal-setup",
            label: "Terminal setup",
            description: "Configure multiline input and terminal keybinding options.",
            command: "terminal-setup",
            icon: "terminal",
        },
        SetupMenuItem {
            id: "memory",
            label: "Memory target",
            description: "Select project or user memory file for context injection.",
            command: "memory",
            icon: "memory",
        },
        SetupMenuItem {
            id: "keybindings",
            label: "Keybindings",
            description: "View or customise keyboard shortcuts for the TUI.",
            command: "keybindings",
            icon: "keyboard",
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use wonder_of_u_core::{CommandInvocation, FeatureSet, PermissionMode, SessionId};

    // Build a minimal CommandContext for tests.
    fn ctx(interactive: bool) -> CommandContext {
        CommandContext {
            session_id: SessionId::new(),
            cwd: std::env::temp_dir(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive,
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

    fn invoke() -> CommandInvocation {
        CommandInvocation {
            name: "setup".into(),
            args: String::new(),
            raw: "/setup".into(),
        }
    }

    // -----------------------------------------------------------------------
    // Spec / registration tests
    // -----------------------------------------------------------------------

    #[test]
    fn command_spec_has_correct_name_and_alias() {
        let spec = SetupCommand::command_spec();
        assert_eq!(spec.name, "setup");
        assert!(
            spec.aliases.contains(&"settings".to_owned()),
            "expected 'settings' alias"
        );
    }

    #[test]
    fn command_spec_is_local_kind() {
        let spec = SetupCommand::command_spec();
        assert_eq!(spec.kind, CommandKind::Local);
    }

    // -----------------------------------------------------------------------
    // Interactive payload shape tests
    // -----------------------------------------------------------------------

    #[test]
    fn interactive_output_starts_with_setup_menu_sentinel() {
        let cmd = SetupCommand::new(None);
        let CommandOutput::Text(text) =
            block_on(cmd.execute(ctx(true), invoke())).expect("execute setup")
        else {
            panic!("expected Text output");
        };
        assert!(
            text.starts_with("setup_menu=true"),
            "payload must start with setup_menu=true, got: {text}"
        );
    }

    #[test]
    fn interactive_output_contains_provider_context_fields() {
        let cmd = SetupCommand::new(None);
        let CommandOutput::Text(text) =
            block_on(cmd.execute(ctx(true), invoke())).expect("execute setup")
        else {
            panic!("expected Text output");
        };
        assert!(
            text.contains("provider_selection="),
            "missing provider_selection"
        );
        assert!(
            text.contains("provider_readiness="),
            "missing provider_readiness"
        );
    }

    #[test]
    fn interactive_output_contains_all_nine_setup_items() {
        let cmd = SetupCommand::new(None);
        let CommandOutput::Text(text) =
            block_on(cmd.execute(ctx(true), invoke())).expect("execute setup")
        else {
            panic!("expected Text output");
        };
        let item_count = text
            .lines()
            .filter(|l| l.starts_with("setup_item="))
            .count();
        assert_eq!(
            item_count, 9,
            "expected 9 setup_item lines, got {item_count}"
        );
    }

    #[test]
    fn interactive_output_contains_all_first_slice_item_ids() {
        let cmd = SetupCommand::new(None);
        let CommandOutput::Text(text) =
            block_on(cmd.execute(ctx(true), invoke())).expect("execute setup")
        else {
            panic!("expected Text output");
        };

        for expected_id in [
            "login",
            "copilot-oauth",
            "model",
            "api-base",
            "theme",
            "permissions",
            "terminal-setup",
            "memory",
            "keybindings",
        ] {
            assert!(
                text.contains(&format!("\"id\":\"{expected_id}\"")),
                "missing setup item id={expected_id}"
            );
        }
    }

    #[test]
    fn interactive_output_items_are_valid_json() {
        let cmd = SetupCommand::new(None);
        let CommandOutput::Text(text) =
            block_on(cmd.execute(ctx(true), invoke())).expect("execute setup")
        else {
            panic!("expected Text output");
        };

        for line in text.lines().filter(|l| l.starts_with("setup_item=")) {
            let json_part = line.trim_start_matches("setup_item=");
            serde_json::from_str::<serde_json::Value>(json_part)
                .unwrap_or_else(|e| panic!("invalid JSON in setup_item line: {e}\nline: {line}"));
        }
    }

    #[test]
    fn interactive_item_objects_have_required_fields() {
        let cmd = SetupCommand::new(None);
        let CommandOutput::Text(text) =
            block_on(cmd.execute(ctx(true), invoke())).expect("execute setup")
        else {
            panic!("expected Text output");
        };

        for line in text.lines().filter(|l| l.starts_with("setup_item=")) {
            let json_part = line.trim_start_matches("setup_item=");
            let val: serde_json::Value =
                serde_json::from_str(json_part).expect("valid JSON in setup_item");
            for field in ["id", "label", "description", "command", "icon"] {
                assert!(
                    val.get(field).and_then(|v| v.as_str()).is_some(),
                    "setup_item missing string field '{field}'"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Non-interactive output tests
    // -----------------------------------------------------------------------

    #[test]
    fn non_interactive_output_contains_provider_summary() {
        let cmd = SetupCommand::new(None);
        let CommandOutput::Text(text) =
            block_on(cmd.execute(ctx(false), invoke())).expect("execute setup non-interactive")
        else {
            panic!("expected Text output");
        };
        assert!(
            text.contains("provider_selection"),
            "non-interactive output should contain provider_selection"
        );
        assert!(
            text.contains("provider_readiness"),
            "non-interactive output should contain provider_readiness"
        );
    }

    #[test]
    fn non_interactive_output_lists_slash_commands() {
        let cmd = SetupCommand::new(None);
        let CommandOutput::Text(text) =
            block_on(cmd.execute(ctx(false), invoke())).expect("execute setup non-interactive")
        else {
            panic!("expected Text output");
        };

        for slash_cmd in [
            "login",
            "model",
            "config",
            "theme",
            "permissions",
            "memory",
            "keybindings",
        ] {
            assert!(
                text.contains(&format!("/{slash_cmd}")),
                "non-interactive output should mention /{slash_cmd}"
            );
        }
    }

    #[test]
    fn non_interactive_output_does_not_contain_setup_menu_sentinel() {
        let cmd = SetupCommand::new(None);
        let CommandOutput::Text(text) =
            block_on(cmd.execute(ctx(false), invoke())).expect("execute setup non-interactive")
        else {
            panic!("expected Text output");
        };
        assert!(
            !text.contains("setup_menu=true"),
            "non-interactive output must not contain the TUI sentinel"
        );
    }
}
