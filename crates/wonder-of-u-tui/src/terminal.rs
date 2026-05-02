use std::{
    collections::BTreeMap,
    io::{self, IsTerminal, Write},
};

use crossterm::{
    cursor::{Hide, Show},
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalCommand {
    EnterAlternateScreen,
    LeaveAlternateScreen,
    HideCursor,
    ShowCursor,
    EnableBracketedPaste,
    DisableBracketedPaste,
    EnableMouseCapture,
    DisableMouseCapture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalConfig {
    pub raw_mode: bool,
    pub alternate_screen: bool,
    pub bracketed_paste: bool,
    pub mouse_capture: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            raw_mode: true,
            alternate_screen: true,
            bracketed_paste: true,
            mouse_capture: false,
        }
    }
}

pub trait TerminalControl {
    fn enable_raw_mode(&mut self) -> io::Result<()>;
    fn disable_raw_mode(&mut self) -> io::Result<()>;
    fn apply(&mut self, command: TerminalCommand) -> io::Result<()>;
}

#[derive(Debug)]
pub struct CrosstermControl<W> {
    writer: W,
}

impl<W> CrosstermControl<W> {
    #[must_use]
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    #[must_use]
    pub fn writer(&self) -> &W {
        &self.writer
    }

    pub fn writer_mut(&mut self) -> &mut W {
        &mut self.writer
    }
}

impl<W: Write> TerminalControl for CrosstermControl<W> {
    fn enable_raw_mode(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()
    }

    fn disable_raw_mode(&mut self) -> io::Result<()> {
        terminal::disable_raw_mode()
    }

    fn apply(&mut self, command: TerminalCommand) -> io::Result<()> {
        match command {
            TerminalCommand::EnterAlternateScreen => execute!(self.writer, EnterAlternateScreen),
            TerminalCommand::LeaveAlternateScreen => execute!(self.writer, LeaveAlternateScreen),
            TerminalCommand::HideCursor => execute!(self.writer, Hide),
            TerminalCommand::ShowCursor => execute!(self.writer, Show),
            TerminalCommand::EnableBracketedPaste => execute!(self.writer, EnableBracketedPaste),
            TerminalCommand::DisableBracketedPaste => execute!(self.writer, DisableBracketedPaste),
            TerminalCommand::EnableMouseCapture => execute!(self.writer, EnableMouseCapture),
            TerminalCommand::DisableMouseCapture => execute!(self.writer, DisableMouseCapture),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalState {
    pub raw_mode: bool,
    pub alternate_screen: bool,
    pub cursor_hidden: bool,
    pub bracketed_paste: bool,
    pub mouse_capture: bool,
}

#[derive(Debug)]
pub struct TerminalLifecycle<C: TerminalControl> {
    control: C,
    state: TerminalState,
}

impl<C: TerminalControl> TerminalLifecycle<C> {
    pub fn enter(control: C, config: TerminalConfig) -> io::Result<Self> {
        let mut lifecycle = Self {
            control,
            state: TerminalState::default(),
        };

        if let Err(error) = lifecycle.enter_inner(config) {
            let _ = lifecycle.restore();
            return Err(error);
        }

        Ok(lifecycle)
    }

    pub fn reenter(&mut self, config: TerminalConfig) -> io::Result<()> {
        self.enter_inner(config)
    }

    #[must_use]
    pub const fn state(&self) -> TerminalState {
        self.state
    }

    #[must_use]
    pub fn control(&self) -> &C {
        &self.control
    }

    pub fn control_mut(&mut self) -> &mut C {
        &mut self.control
    }

    pub fn restore(&mut self) -> io::Result<()> {
        let mut first_error = None;

        if self.state.mouse_capture {
            record_first_error(
                &mut first_error,
                self.control.apply(TerminalCommand::DisableMouseCapture),
            );
            self.state.mouse_capture = false;
        }

        if self.state.bracketed_paste {
            record_first_error(
                &mut first_error,
                self.control.apply(TerminalCommand::DisableBracketedPaste),
            );
            self.state.bracketed_paste = false;
        }

        if self.state.cursor_hidden {
            record_first_error(
                &mut first_error,
                self.control.apply(TerminalCommand::ShowCursor),
            );
            self.state.cursor_hidden = false;
        }

        if self.state.alternate_screen {
            record_first_error(
                &mut first_error,
                self.control.apply(TerminalCommand::LeaveAlternateScreen),
            );
            self.state.alternate_screen = false;
        }

        if self.state.raw_mode {
            record_first_error(&mut first_error, self.control.disable_raw_mode());
            self.state.raw_mode = false;
        }

        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn enter_inner(&mut self, config: TerminalConfig) -> io::Result<()> {
        if config.raw_mode {
            self.control.enable_raw_mode()?;
            self.state.raw_mode = true;
        }

        if config.alternate_screen {
            self.control.apply(TerminalCommand::EnterAlternateScreen)?;
            self.state.alternate_screen = true;
        }

        self.control.apply(TerminalCommand::HideCursor)?;
        self.state.cursor_hidden = true;

        if config.bracketed_paste {
            self.control.apply(TerminalCommand::EnableBracketedPaste)?;
            self.state.bracketed_paste = true;
        }

        if config.mouse_capture {
            self.control.apply(TerminalCommand::EnableMouseCapture)?;
            self.state.mouse_capture = true;
        }

        Ok(())
    }
}

impl<C: TerminalControl> Drop for TerminalLifecycle<C> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn record_first_error(slot: &mut Option<io::Error>, result: io::Result<()>) {
    if let Err(error) = result {
        let _ = slot.get_or_insert(error);
    }
}

const ADDITIONAL_HYPERLINK_TERMINALS: &[&str] = &[
    "ghostty",
    "Hyper",
    "kitty",
    "alacritty",
    "iTerm.app",
    "iTerm2",
];

/// The host platform used for terminal capability modeling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalPlatform {
    Unix,
    Windows,
}

/// Serializable terminal environment input for capability detection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalEnv {
    pub platform: TerminalPlatform,
    pub is_tty: bool,
    vars: BTreeMap<String, String>,
}

impl TerminalEnv {
    #[must_use]
    pub fn new(platform: TerminalPlatform, is_tty: bool) -> Self {
        Self {
            platform,
            is_tty,
            vars: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn capture_stdout() -> Self {
        let platform = if cfg!(windows) {
            TerminalPlatform::Windows
        } else {
            TerminalPlatform::Unix
        };

        Self {
            platform,
            is_tty: io::stdout().is_terminal(),
            vars: std::env::vars().collect(),
        }
    }

    #[must_use]
    pub fn from_iter<K, V, I>(platform: TerminalPlatform, is_tty: bool, vars: I) -> Self
    where
        K: Into<String>,
        V: Into<String>,
        I: IntoIterator<Item = (K, V)>,
    {
        let mut env = Self::new(platform, is_tty);
        for (key, value) in vars {
            env.vars.insert(key.into(), value.into());
        }
        env
    }

    #[must_use]
    pub fn with_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn var(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    #[must_use]
    pub fn has_var(&self, key: &str) -> bool {
        self.vars.contains_key(key)
    }
}

/// Home-position control used after a clear-screen request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorHomeCommand {
    /// CSI H
    CursorHome,
    /// CSI 0 f
    HorizontalVerticalPosition,
}

impl CursorHomeCommand {
    #[must_use]
    pub const fn ansi(self) -> &'static str {
        match self {
            Self::CursorHome => "\u{1b}[H",
            Self::HorizontalVerticalPosition => "\u{1b}[0f",
        }
    }
}

/// Renderer-independent clear-screen intent and emission details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClearTerminalCommand {
    pub include_scrollback: bool,
    pub cursor_home: CursorHomeCommand,
}

impl ClearTerminalCommand {
    #[must_use]
    pub fn detect(env: &TerminalEnv) -> Self {
        if env.platform != TerminalPlatform::Windows {
            return Self {
                include_scrollback: true,
                cursor_home: CursorHomeCommand::CursorHome,
            };
        }

        if is_modern_windows_terminal(env) {
            Self {
                include_scrollback: true,
                cursor_home: CursorHomeCommand::CursorHome,
            }
        } else {
            Self {
                include_scrollback: false,
                cursor_home: CursorHomeCommand::HorizontalVerticalPosition,
            }
        }
    }

    #[must_use]
    pub fn ansi_sequence(self) -> String {
        let mut sequence = String::from("\u{1b}[2J");
        if self.include_scrollback {
            sequence.push_str("\u{1b}[3J");
        }
        sequence.push_str(self.cursor_home.ansi());
        sequence
    }
}

/// Bundled terminal capability decisions for the current renderer/runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalCapabilities {
    pub hyperlinks: bool,
    pub synchronized_output: bool,
    pub clear_terminal: ClearTerminalCommand,
    pub cursor_up_viewport_yank_bug: bool,
}

impl TerminalCapabilities {
    #[must_use]
    pub fn detect(env: &TerminalEnv, hyperlink_baseline: bool) -> Self {
        Self {
            hyperlinks: supports_hyperlinks(env, hyperlink_baseline),
            synchronized_output: supports_synchronized_output(env),
            clear_terminal: ClearTerminalCommand::detect(env),
            cursor_up_viewport_yank_bug: has_cursor_up_viewport_yank_bug(env),
        }
    }
}

/// Extends a baseline hyperlink detector with additional Ink-compatible terminals.
#[must_use]
pub fn supports_hyperlinks(env: &TerminalEnv, baseline_supported: bool) -> bool {
    if baseline_supported {
        return true;
    }

    if !env.is_tty {
        return false;
    }

    if env
        .var("TERM_PROGRAM")
        .is_some_and(|value| ADDITIONAL_HYPERLINK_TERMINALS.contains(&value))
    {
        return true;
    }

    if env
        .var("LC_TERMINAL")
        .is_some_and(|value| ADDITIONAL_HYPERLINK_TERMINALS.contains(&value))
    {
        return true;
    }

    env.var("TERM").is_some_and(|value| value.contains("kitty"))
}

/// Models DEC 2026 synchronized output support from environment hints alone.
#[must_use]
pub fn supports_synchronized_output(env: &TerminalEnv) -> bool {
    if env.has_var("TMUX") {
        return false;
    }

    if env.var("TERM_PROGRAM").is_some_and(|term_program| {
        matches!(
            term_program,
            "iTerm.app"
                | "WezTerm"
                | "WarpTerminal"
                | "ghostty"
                | "contour"
                | "vscode"
                | "alacritty"
        )
    }) {
        return true;
    }

    if env
        .var("TERM")
        .is_some_and(|term| term.contains("kitty") || term.contains("alacritty"))
    {
        return true;
    }

    if env.var("TERM") == Some("xterm-ghostty") {
        return true;
    }

    if env.var("TERM").is_some_and(|term| term.starts_with("foot")) {
        return true;
    }

    if env.has_var("KITTY_WINDOW_ID") || env.has_var("ZED_TERM") || env.has_var("WT_SESSION") {
        return true;
    }

    env.var("VTE_VERSION")
        .and_then(|value| value.parse::<u16>().ok())
        .is_some_and(|version| version >= 6800)
}

/// Windows terminals and Windows Terminal-backed WSL sessions can yank scrollback on cursor-up.
#[must_use]
pub fn has_cursor_up_viewport_yank_bug(env: &TerminalEnv) -> bool {
    env.platform == TerminalPlatform::Windows || env.has_var("WT_SESSION")
}

fn is_modern_windows_terminal(env: &TerminalEnv) -> bool {
    if env.has_var("WT_SESSION") {
        return true;
    }

    if matches!(
        env.var("TERM_PROGRAM"),
        Some("vscode") if env.var("TERM_PROGRAM_VERSION").is_some()
    ) {
        return true;
    }

    if env.var("TERM_PROGRAM") == Some("mintty") {
        return true;
    }

    env.platform == TerminalPlatform::Windows && env.has_var("MSYSTEM")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct FakeTerminalControl {
        fail_at: Option<usize>,
        operations: Vec<String>,
    }

    impl FakeTerminalControl {
        fn with_failure(step: usize) -> Self {
            Self {
                fail_at: Some(step),
                operations: Vec::new(),
            }
        }

        fn record(&mut self, label: &str) -> io::Result<()> {
            self.operations.push(label.to_string());
            if self.fail_at == Some(self.operations.len()) {
                return Err(io::Error::other(format!("forced failure at {label}")));
            }
            Ok(())
        }
    }

    impl TerminalControl for FakeTerminalControl {
        fn enable_raw_mode(&mut self) -> io::Result<()> {
            self.record("enable_raw_mode")
        }

        fn disable_raw_mode(&mut self) -> io::Result<()> {
            self.record("disable_raw_mode")
        }

        fn apply(&mut self, command: TerminalCommand) -> io::Result<()> {
            self.record(match command {
                TerminalCommand::EnterAlternateScreen => "enter_alt",
                TerminalCommand::LeaveAlternateScreen => "leave_alt",
                TerminalCommand::HideCursor => "hide_cursor",
                TerminalCommand::ShowCursor => "show_cursor",
                TerminalCommand::EnableBracketedPaste => "enable_paste",
                TerminalCommand::DisableBracketedPaste => "disable_paste",
                TerminalCommand::EnableMouseCapture => "enable_mouse",
                TerminalCommand::DisableMouseCapture => "disable_mouse",
            })
        }
    }

    #[test]
    fn terminal_lifecycle_tracks_enabled_capabilities() {
        let control = FakeTerminalControl::default();
        let lifecycle =
            TerminalLifecycle::enter(control, TerminalConfig::default()).expect("enter");

        assert_eq!(
            lifecycle.state(),
            TerminalState {
                raw_mode: true,
                alternate_screen: true,
                cursor_hidden: true,
                bracketed_paste: true,
                mouse_capture: false,
            }
        );
        assert_eq!(
            lifecycle.control().operations,
            vec![
                "enable_raw_mode".to_string(),
                "enter_alt".to_string(),
                "hide_cursor".to_string(),
                "enable_paste".to_string(),
            ]
        );
    }

    #[test]
    fn failed_enter_restores_successful_steps() {
        let control = FakeTerminalControl::with_failure(4);
        let error =
            TerminalLifecycle::enter(control, TerminalConfig::default()).expect_err("failure");

        assert!(error.to_string().contains("forced failure"));
    }

    #[test]
    fn restore_runs_in_reverse_order_and_is_idempotent() {
        let control = FakeTerminalControl::default();
        let mut lifecycle = TerminalLifecycle::enter(
            control,
            TerminalConfig {
                mouse_capture: true,
                ..TerminalConfig::default()
            },
        )
        .expect("enter");

        lifecycle.restore().expect("restore");
        lifecycle.restore().expect("idempotent restore");

        assert_eq!(
            lifecycle.control().operations,
            vec![
                "enable_raw_mode".to_string(),
                "enter_alt".to_string(),
                "hide_cursor".to_string(),
                "enable_paste".to_string(),
                "enable_mouse".to_string(),
                "disable_mouse".to_string(),
                "disable_paste".to_string(),
                "show_cursor".to_string(),
                "leave_alt".to_string(),
                "disable_raw_mode".to_string(),
            ]
        );
        assert_eq!(lifecycle.state(), TerminalState::default());
    }

    #[test]
    fn hyperlink_support_extends_baseline_detector() {
        let env = TerminalEnv::from_iter(
            TerminalPlatform::Unix,
            true,
            [("TERM_PROGRAM", "ghostty"), ("TERM", "xterm-256color")],
        );

        assert!(supports_hyperlinks(&env, false));
        assert!(supports_hyperlinks(
            &TerminalEnv::new(TerminalPlatform::Unix, false),
            true
        ));
        assert!(!supports_hyperlinks(
            &TerminalEnv::from_iter(TerminalPlatform::Unix, false, [("TERM_PROGRAM", "ghostty")]),
            false
        ));
    }

    #[test]
    fn synchronized_output_support_matches_known_terminals() {
        let wezterm =
            TerminalEnv::from_iter(TerminalPlatform::Unix, true, [("TERM_PROGRAM", "WezTerm")]);
        let tmux = TerminalEnv::from_iter(
            TerminalPlatform::Unix,
            true,
            [("TERM_PROGRAM", "WezTerm"), ("TMUX", "1")],
        );
        let vte = TerminalEnv::from_iter(TerminalPlatform::Unix, true, [("VTE_VERSION", "6800")]);

        assert!(supports_synchronized_output(&wezterm));
        assert!(supports_synchronized_output(&vte));
        assert!(!supports_synchronized_output(&tmux));
    }

    #[test]
    fn clear_terminal_command_keeps_legacy_windows_compatible() {
        let legacy = TerminalEnv::new(TerminalPlatform::Windows, true);
        let modern = TerminalEnv::from_iter(
            TerminalPlatform::Windows,
            true,
            [("WT_SESSION", "1"), ("TERM_PROGRAM", "vscode")],
        );

        assert_eq!(
            ClearTerminalCommand::detect(&legacy),
            ClearTerminalCommand {
                include_scrollback: false,
                cursor_home: CursorHomeCommand::HorizontalVerticalPosition,
            }
        );
        assert_eq!(
            ClearTerminalCommand::detect(&legacy).ansi_sequence(),
            "\u{1b}[2J\u{1b}[0f"
        );
        assert_eq!(
            ClearTerminalCommand::detect(&modern).ansi_sequence(),
            "\u{1b}[2J\u{1b}[3J\u{1b}[H"
        );
    }

    #[test]
    fn cursor_up_viewport_yank_bug_tracks_windows_hosts_and_wt_sessions() {
        let linux = TerminalEnv::new(TerminalPlatform::Unix, true);
        let windows = TerminalEnv::new(TerminalPlatform::Windows, true);
        let wsl_in_windows_terminal =
            TerminalEnv::from_iter(TerminalPlatform::Unix, true, [("WT_SESSION", "1")]);

        assert!(!has_cursor_up_viewport_yank_bug(&linux));
        assert!(has_cursor_up_viewport_yank_bug(&windows));
        assert!(has_cursor_up_viewport_yank_bug(&wsl_in_windows_terminal));
    }

    #[test]
    fn terminal_capabilities_bundle_consistent_detection() {
        let env = TerminalEnv::from_iter(
            TerminalPlatform::Unix,
            true,
            [("TERM", "xterm-kitty"), ("KITTY_WINDOW_ID", "99")],
        );

        let capabilities = TerminalCapabilities::detect(&env, false);

        assert!(capabilities.hyperlinks);
        assert!(capabilities.synchronized_output);
        assert_eq!(
            capabilities.clear_terminal,
            ClearTerminalCommand {
                include_scrollback: true,
                cursor_home: CursorHomeCommand::CursorHome,
            }
        );
        assert!(!capabilities.cursor_up_viewport_yank_bug);
    }
}
