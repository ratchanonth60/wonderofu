//! Foundation primitives for the future wonder-of-u terminal UI.
//!
//! This crate intentionally stays small in the first pass: it owns terminal
//! lifecycle guards, normalized input events, frame/style/layout primitives,
//! and a snapshot-friendly shell renderer.

pub mod dialog;
pub mod event;
pub mod frame;
pub mod input;
pub mod keymap;
pub mod layout;
pub mod message;
pub mod render;
pub mod style;
pub mod terminal;
pub mod vim;

pub use dialog::{DialogActionView, DialogKind, DialogView};
pub use event::{
    CrosstermEventSource, EventLoop, EventLoopState, EventSource, KeyCode, KeyEvent, KeyModifiers,
    TurnState, UiEvent, normalize_key_event,
};
pub use frame::{Cell, FrameBuffer, Rect};
pub use input::{EditAction, Motion, TextBuffer};
pub use keymap::{
    KeyBinding, KeyBindingContext, KeyBindingError, KeyBindingResolver, ResolvedKey, SystemAction,
    VimCommand,
};
pub use layout::ShellLayout;
pub use message::{
    MessageLineView, MessageRole, TaskPanelView, footer_text, message_lines, queued_panel_view,
    status_text, task_panel_view,
};
pub use render::{ShellView, render_shell, render_snapshot};
pub use style::{Color, TextStyle, Theme};
pub use terminal::{
    CrosstermControl, TerminalCommand, TerminalConfig, TerminalControl, TerminalLifecycle,
    TerminalState,
};
pub use vim::{VimMode, VimState};
