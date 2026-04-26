use std::io::{self, Write};

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
}
