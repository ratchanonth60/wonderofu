//! Implements the `/terminal-setup` command together with terminal detection
//! and recommendation helpers.
//!
//! The public surface is intentionally narrow:
//! * [`TerminalSetupCommand`] — the command registered in the command registry.
//! * [`detect_terminal_type`] — exported for use by the TUI setup flow.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
};

use super::keybinding_commands::resolve_keybindings_path;

// ── Command struct ────────────────────────────────────────────────────────────

/// Handles `/terminal-setup` — explains multiline-prompt setup and keybinding
/// options for the local TUI.
pub struct TerminalSetupCommand {
    storage_dir: Option<PathBuf>,
}

impl TerminalSetupCommand {
    /// Creates a new `TerminalSetupCommand`.
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Returns the [`CommandSpec`] for this command.
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "terminal-setup",
            "Explain multiline prompt setup and keybinding options for the local TUI",
            CommandKind::Local,
        );
        spec.aliases.push("terminalSetup".into());
        spec
    }
}

// ── Command impl ──────────────────────────────────────────────────────────────

#[async_trait]
impl Command for TerminalSetupCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::Text(render_terminal_setup_notice(
            self.storage_dir.as_deref(),
        )))
    }
}

// ── Terminal detection ────────────────────────────────────────────────────────

/// Terminals that natively decode Shift+Enter via the Kitty keyboard protocol,
/// so no extra keybinding setup is needed for multi-line input.
const NATIVE_CSIU_TERMINALS: &[&str] =
    &["ghostty", "kitty", "iTerm.app", "WezTerm", "WarpTerminal"];

/// Returns the value of `TERM_PROGRAM`, or `"unknown"` when the variable is
/// not set or is empty.  Used by `render_terminal_setup_notice` to tailor its
/// advice.
pub(crate) fn detect_terminal_type() -> String {
    std::env::var("TERM_PROGRAM")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Returns `true` when `terminal` natively handles the Kitty keyboard protocol
/// (so Shift+Enter already works without any extra setup).
fn is_native_csiu_terminal(terminal: &str) -> bool {
    NATIVE_CSIU_TERMINALS
        .iter()
        .any(|&t| t.eq_ignore_ascii_case(terminal))
}

/// Returns a short human-readable recommendation for the given terminal.
fn terminal_setup_recommendation(terminal: &str) -> &'static str {
    match terminal {
        "Apple_Terminal" => {
            "Use Option+Enter (⌥↵) for newlines. Shift+Enter is not supported in Apple Terminal."
        }
        "vscode" | "cursor" | "windsurf" => {
            "Use Shift+Enter for newlines. Your editor's built-in terminal handles the keybinding."
        }
        t if is_native_csiu_terminal(t) => {
            "Your terminal natively supports Shift+Enter via the Kitty keyboard protocol — no extra setup needed."
        }
        _ => {
            "Use `/keybindings open` to add a Shift+Enter → insert_newline keybinding to the config file."
        }
    }
}

fn render_terminal_setup_notice(storage_dir: Option<&Path>) -> String {
    let path = resolve_keybindings_path(storage_dir);
    let terminal = detect_terminal_type();
    let setup_needed = !is_native_csiu_terminal(&terminal)
        && terminal != "vscode"
        && terminal != "cursor"
        && terminal != "windsurf";
    let recommendation = terminal_setup_recommendation(&terminal);

    [
        "## Terminal Setup".into(),
        format!("terminal_type={terminal}"),
        format!("setup_needed={setup_needed}"),
        recommendation.into(),
        String::new(),
        "Use `/keybindings` to inspect the active shortcuts.".into(),
        format!(
            "Use `/keybindings open` to create or edit {} with the starter template, which includes a Shift+Enter -> insert_newline example.",
            path.display()
        ),
        "status=terminal keybinding template ready".into(),
    ]
    .join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wonder_of_u_core::{
        Command, CommandContext, CommandInvocation, CommandOutput, FeatureSet, PermissionMode,
        SessionId,
    };
    use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};

    use super::{
        detect_terminal_type, is_native_csiu_terminal, render_terminal_setup_notice,
        terminal_setup_recommendation,
    };
    use crate::commands::keybinding_commands::resolve_keybindings_path;

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

    /// With TERM_PROGRAM unset, detect_terminal_type returns "unknown".
    #[test]
    fn detect_terminal_type_returns_unknown_when_unset() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "");
        assert_eq!(detect_terminal_type(), "unknown");
    }

    /// With TERM_PROGRAM set, detect_terminal_type echoes its value.
    #[test]
    fn detect_terminal_type_echoes_term_program() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "ghostty");
        assert_eq!(detect_terminal_type(), "ghostty");
    }

    /// Kitty-protocol-native terminals must report setup_needed=false.
    #[test]
    fn native_csiu_terminals_report_setup_not_needed() {
        for terminal in &["ghostty", "kitty", "WezTerm", "WarpTerminal"] {
            assert!(
                is_native_csiu_terminal(terminal),
                "{terminal} should be recognised as a native Kitty-protocol terminal"
            );
        }
    }

    /// iTerm.app (mixed case) must also be recognised.
    #[test]
    fn iterm_app_is_native_csiu_terminal() {
        assert!(is_native_csiu_terminal("iTerm.app"));
    }

    /// Apple Terminal gets the Option+Enter advice.
    #[test]
    fn terminal_setup_apple_terminal_shows_option_enter_advice() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "Apple_Terminal");
        let dir = unique_test_dir("workflow-terminal-setup-apple");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));
        assert!(
            rendered.contains("Option+Enter"),
            "Apple Terminal advice must mention Option+Enter; got:\n{rendered}"
        );
        assert!(
            rendered.contains("setup_needed=true"),
            "Apple Terminal must require setup; got:\n{rendered}"
        );
    }

    /// Ghostty (a native Kitty-protocol terminal) must report setup_needed=false.
    #[test]
    fn terminal_setup_ghostty_reports_no_setup_needed() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "ghostty");
        let dir = unique_test_dir("workflow-terminal-setup-ghostty");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));
        assert!(
            rendered.contains("setup_needed=false"),
            "ghostty must not need setup; got:\n{rendered}"
        );
    }

    /// VSCode reports setup_needed=false and Shift+Enter advice.
    #[test]
    fn terminal_setup_vscode_shows_shift_enter_advice() {
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "vscode");
        let dir = unique_test_dir("workflow-terminal-setup-vscode");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));
        assert!(
            terminal_setup_recommendation("vscode").contains("Shift+Enter"),
            "VSCode advice must mention Shift+Enter"
        );
        assert!(
            rendered.contains("terminal_type=vscode"),
            "must include terminal_type=vscode; got:\n{rendered}"
        );
    }

    #[test]
    fn terminal_setup_notice_points_to_keybindings_flow() {
        let dir = unique_test_dir("workflow-terminal-setup-notice");
        // Run with TERM_PROGRAM unset so the output is deterministic regardless
        // of the CI environment.
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "");
        let rendered = render_terminal_setup_notice(Some(dir.as_path()));

        assert!(rendered.starts_with("## Terminal Setup"));
        assert!(
            rendered.contains("terminal_type="),
            "must include terminal_type field; got:\n{rendered}"
        );
        assert!(
            rendered.contains("setup_needed="),
            "must include setup_needed field; got:\n{rendered}"
        );
        assert!(rendered.contains("Use `/keybindings` to inspect the active shortcuts."));
        assert!(rendered.contains(&format!(
            "Use `/keybindings open` to create or edit {}",
            resolve_keybindings_path(Some(dir.as_path())).display()
        )));
    }

    #[test]
    fn terminal_setup_command_renders_notice() {
        let dir = unique_test_dir("workflow-terminal-setup-command");
        let _guard = EnvVarGuard::set("TERM_PROGRAM", "");
        let output = futures::executor::block_on(
            super::TerminalSetupCommand::new(Some(dir.clone())).execute(
                test_context(&dir),
                CommandInvocation {
                    name: "terminal-setup".into(),
                    args: String::new(),
                    raw: "/terminal-setup".into(),
                },
            ),
        )
        .expect("terminal setup output");

        let CommandOutput::Text(text) = output else {
            panic!("expected text output");
        };
        assert!(text.contains("## Terminal Setup"));
        assert!(
            text.contains("terminal_type="),
            "must include terminal_type field; got:\n{text}"
        );
        assert!(
            text.contains("setup_needed="),
            "must include setup_needed field; got:\n{text}"
        );
        assert!(text.contains("Shift+Enter -> insert_newline"));
    }
}
