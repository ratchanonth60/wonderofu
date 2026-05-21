//! Implements the `/privacy-settings` command.
//!
//! The public surface is intentionally narrow:
//! * [`PrivacySettingsCommand`] — the command registered in the command registry.

use async_trait::async_trait;
use clap::{Parser, Subcommand};
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
};

use super::{parse_command_args, try_open_browser};

// ── URL constant ──────────────────────────────────────────────────────────────

const PRIVACY_SETTINGS_URL: &str = "https://claude.ai/settings/data-privacy-controls";

// ── Command struct ────────────────────────────────────────────────────────────

/// Handles `/privacy-settings` — opens the browser to the privacy controls
/// page and summarises the action taken.
pub struct PrivacySettingsCommand;

impl PrivacySettingsCommand {
    /// Creates a new `PrivacySettingsCommand`.
    pub const fn new() -> Self {
        Self
    }

    /// Returns the [`CommandSpec`] for this command.
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "privacy-settings",
            "View and update your privacy settings",
            CommandKind::Local,
        )
    }
}

// ── Clap arg structs ──────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct PrivacySettingsArgs {
    #[command(subcommand)]
    command: Option<PrivacySettingsSubcommand>,
}

#[derive(Debug, Subcommand)]
enum PrivacySettingsSubcommand {
    Show,
}

// ── Command impl ──────────────────────────────────────────────────────────────

#[async_trait]
impl Command for PrivacySettingsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<PrivacySettingsArgs>("privacy-settings", &invocation)?;
        match args.command.unwrap_or(PrivacySettingsSubcommand::Show) {
            PrivacySettingsSubcommand::Show => Ok(CommandOutput::Text(
                render_privacy_settings_summary(try_open_browser(PRIVACY_SETTINGS_URL)),
            )),
        }
    }
}

// ── Render helper ─────────────────────────────────────────────────────────────

fn render_privacy_settings_summary(browser_launch_attempted: bool) -> String {
    [
        "## Privacy Settings".into(),
        format!("privacy_settings_url={PRIVACY_SETTINGS_URL}"),
        format!("browser_launch_attempted={browser_launch_attempted}"),
        "status=privacy settings opened".into(),
        format!("Review and manage your privacy settings at {PRIVACY_SETTINGS_URL}"),
    ]
    .join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use wonder_of_u_core::{
        Command, CommandContext, CommandInvocation, CommandOutput, FeatureSet, PermissionMode,
        SessionId,
    };

    use super::{PrivacySettingsCommand, render_privacy_settings_summary};

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
    fn privacy_settings_summary_points_to_web_controls() {
        let rendered = render_privacy_settings_summary(false);

        assert!(rendered.contains("## Privacy Settings"));
        assert!(rendered.contains("status=privacy settings opened"));
        assert!(rendered.contains("browser_launch_attempted=false"));
        assert!(rendered.contains("https://claude.ai/settings/data-privacy-controls"));
    }

    #[test]
    fn privacy_settings_command_does_not_launch_browser_under_tests() {
        let output = block_on(PrivacySettingsCommand::new().execute(
            test_context(std::path::Path::new("/workspace")),
            CommandInvocation {
                name: "privacy-settings".into(),
                args: String::new(),
                raw: "/privacy-settings".into(),
            },
        ))
        .expect("run privacy settings command");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };

        assert!(text.contains("browser_launch_attempted=false"));
    }
}
