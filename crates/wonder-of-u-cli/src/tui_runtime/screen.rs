use super::*;
use crossterm::event::{
    DisableBracketedPaste, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};

fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
}

/// Enters raw mode and alternate screen via ratatui's `CrosstermBackend`, returning
/// a fully initialised `Terminal` ready for `draw()` calls.
pub(super) fn setup_ratatui_terminal<W: Write>(writer: W) -> Result<Terminal<CrosstermBackend<W>>> {
    terminal::enable_raw_mode()?;
    let mut backend = CrosstermBackend::new(writer);
    execute!(
        backend,
        EnterAlternateScreen,
        Hide,
        EnableBracketedPaste,
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
    )?;
    Ok(Terminal::new(backend)?)
}

/// Leaves alternate screen and restores the terminal to a usable state.
pub(super) fn restore_ratatui_terminal<W: Write>(term: &mut Terminal<CrosstermBackend<W>>) {
    let _ = execute!(
        term.backend_mut(),
        LeaveAlternateScreen,
        Show,
        DisableBracketedPaste,
        PopKeyboardEnhancementFlags
    );
    let _ = terminal::disable_raw_mode();
}
