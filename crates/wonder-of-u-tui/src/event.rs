use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event as CrosstermEvent, EventStream, KeyCode as CrosstermKeyCode,
    KeyEvent as CrosstermKeyEvent, KeyEventKind, MouseButton as CrosstermMouseButton,
    MouseEvent as CrosstermMouseEvent, MouseEventKind as CrosstermMouseEventKind,
};
use futures::StreamExt;
use tokio::time::timeout;
use wonder_of_u_core::{Result, WonderError};
/// Enumerates key code
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCode {
    /// Represents backspace
    Backspace,
    /// Represents enter
    Enter,
    /// Represents left
    Left,
    /// Represents right
    Right,
    /// Represents up
    Up,
    /// Represents down
    Down,
    /// Represents home
    Home,
    /// Represents end
    End,
    /// Represents page up
    PageUp,
    /// Represents page down
    PageDown,
    /// Represents tab
    Tab,
    /// Represents back tab
    BackTab,
    /// Represents delete
    Delete,
    /// Represents insert
    Insert,
    /// Represents esc
    Esc,
    /// Represents char
    Char(char),
    /// Represents f
    F(u8),
    /// Represents null
    Null,
}
/// Represents key modifiers
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyModifiers {
    /// Stores the shift
    pub shift: bool,
    /// Stores the control
    pub control: bool,
    /// Stores the alt
    pub alt: bool,
}

impl From<event::KeyModifiers> for KeyModifiers {
    fn from(value: event::KeyModifiers) -> Self {
        Self {
            shift: value.contains(event::KeyModifiers::SHIFT),
            control: value.contains(event::KeyModifiers::CONTROL),
            alt: value.contains(event::KeyModifiers::ALT),
        }
    }
}
/// Represents key event
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    /// Stores the code
    pub code: KeyCode,
    /// Stores the modifiers
    pub modifiers: KeyModifiers,
}

impl KeyEvent {
    /// Returns whether ctrl char
    #[must_use]
    pub fn is_ctrl_char(self, expected: char) -> bool {
        matches!(self.code, KeyCode::Char(actual)
            if actual.eq_ignore_ascii_case(&expected) && self.modifiers.control)
    }
}
/// Enumerates mouse button
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
    /// Represents left
    Left,
    /// Represents middle
    Middle,
    /// Represents right
    Right,
}

impl From<CrosstermMouseButton> for MouseButton {
    fn from(value: CrosstermMouseButton) -> Self {
        match value {
            CrosstermMouseButton::Left => Self::Left,
            CrosstermMouseButton::Middle => Self::Middle,
            CrosstermMouseButton::Right => Self::Right,
        }
    }
}
/// Enumerates mouse event kind
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseEventKind {
    /// Represents down
    Down(MouseButton),
    /// Represents up
    Up(MouseButton),
    /// Represents drag
    Drag(MouseButton),
    /// Represents moved
    Moved,
    /// Represents scroll up
    ScrollUp,
    /// Represents scroll down
    ScrollDown,
    /// Represents scroll left
    ScrollLeft,
    /// Represents scroll right
    ScrollRight,
}
/// Represents mouse event
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseEvent {
    /// Stores the kind
    pub kind: MouseEventKind,
    /// Stores the column
    pub column: u16,
    /// Stores the row
    pub row: u16,
    /// Stores the modifiers
    pub modifiers: KeyModifiers,
}
/// Represents click event
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClickEvent {
    /// Stores the button
    pub button: MouseButton,
    /// Stores the column
    pub column: u16,
    /// Stores the row
    pub row: u16,
    /// Stores the modifiers
    pub modifiers: KeyModifiers,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PressState {
    button: Option<MouseButton>,
    origin_column: u16,
    origin_row: u16,
    modifiers: KeyModifiers,
    dragged: bool,
}
/// Represents click tracker
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClickTracker {
    pressed: PressState,
}

impl ClickTracker {
    /// Updates state from a UI event
    pub fn observe(&mut self, event: MouseEvent) -> Option<ClickEvent> {
        match event.kind {
            MouseEventKind::Down(button) => {
                self.pressed = PressState {
                    button: Some(button),
                    origin_column: event.column,
                    origin_row: event.row,
                    modifiers: event.modifiers,
                    dragged: false,
                };
                None
            }
            MouseEventKind::Drag(button) => {
                if self.pressed.button == Some(button) {
                    self.pressed.dragged = true;
                }
                None
            }
            MouseEventKind::Moved => {
                if self.pressed.button.is_some()
                    && (event.column != self.pressed.origin_column
                        || event.row != self.pressed.origin_row)
                {
                    self.pressed.dragged = true;
                }
                None
            }
            MouseEventKind::Up(button) => {
                let pressed = self.pressed;
                self.pressed = PressState::default();
                if pressed.button == Some(button) && !pressed.dragged {
                    Some(ClickEvent {
                        button,
                        column: event.column,
                        row: event.row,
                        modifiers: event.modifiers,
                    })
                } else {
                    None
                }
            }
            MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight => None,
        }
    }
}
/// Enumerates ui event
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiEvent {
    /// Represents key
    Key(KeyEvent),
    /// Represents mouse
    Mouse(CrosstermMouseEvent),
    /// Represents paste
    Paste(String),
    /// Represents resize
    Resize {
        /// Stores the width
        width: u16,
        /// Stores the height
        height: u16,
    },
    /// Represents focus gained
    FocusGained,
    /// Represents focus lost
    FocusLost,
    /// Represents tick
    Tick,
}

impl UiEvent {
    /// Handles from crossterm
    #[must_use]
    pub fn from_crossterm(event: CrosstermEvent) -> Option<Self> {
        match event {
            CrosstermEvent::Key(key_event) => normalize_key_event(key_event).map(Self::Key),
            CrosstermEvent::Mouse(mouse_event) => Some(Self::Mouse(mouse_event)),
            CrosstermEvent::Paste(text) => Some(Self::Paste(text)),
            CrosstermEvent::Resize(width, height) => Some(Self::Resize { width, height }),
            CrosstermEvent::FocusGained => Some(Self::FocusGained),
            CrosstermEvent::FocusLost => Some(Self::FocusLost),
        }
    }
    /// Handles normalized mouse
    #[must_use]
    pub fn normalized_mouse(&self) -> Option<MouseEvent> {
        match self {
            Self::Mouse(mouse_event) => Some(normalize_mouse_event(*mouse_event)),
            _ => None,
        }
    }
}
/// Normalizes key event
#[must_use]
pub fn normalize_key_event(event: CrosstermKeyEvent) -> Option<KeyEvent> {
    if matches!(event.kind, KeyEventKind::Release) {
        return None;
    }

    Some(KeyEvent {
        code: normalize_key_code(event.code),
        modifiers: event.modifiers.into(),
    })
}
/// Normalizes mouse event
#[must_use]
pub fn normalize_mouse_event(event: CrosstermMouseEvent) -> MouseEvent {
    MouseEvent {
        kind: normalize_mouse_kind(event.kind),
        column: event.column,
        row: event.row,
        modifiers: event.modifiers.into(),
    }
}

fn normalize_key_code(code: CrosstermKeyCode) -> KeyCode {
    match code {
        CrosstermKeyCode::Backspace => KeyCode::Backspace,
        CrosstermKeyCode::Enter => KeyCode::Enter,
        CrosstermKeyCode::Left => KeyCode::Left,
        CrosstermKeyCode::Right => KeyCode::Right,
        CrosstermKeyCode::Up => KeyCode::Up,
        CrosstermKeyCode::Down => KeyCode::Down,
        CrosstermKeyCode::Home => KeyCode::Home,
        CrosstermKeyCode::End => KeyCode::End,
        CrosstermKeyCode::PageUp => KeyCode::PageUp,
        CrosstermKeyCode::PageDown => KeyCode::PageDown,
        CrosstermKeyCode::Tab => KeyCode::Tab,
        CrosstermKeyCode::BackTab => KeyCode::BackTab,
        CrosstermKeyCode::Delete => KeyCode::Delete,
        CrosstermKeyCode::Insert => KeyCode::Insert,
        CrosstermKeyCode::Esc => KeyCode::Esc,
        CrosstermKeyCode::Char(ch) => KeyCode::Char(ch),
        CrosstermKeyCode::F(index) => KeyCode::F(index),
        _ => KeyCode::Null,
    }
}

fn normalize_mouse_kind(kind: CrosstermMouseEventKind) -> MouseEventKind {
    match kind {
        CrosstermMouseEventKind::Down(button) => MouseEventKind::Down(button.into()),
        CrosstermMouseEventKind::Up(button) => MouseEventKind::Up(button.into()),
        CrosstermMouseEventKind::Drag(button) => MouseEventKind::Drag(button.into()),
        CrosstermMouseEventKind::Moved => MouseEventKind::Moved,
        CrosstermMouseEventKind::ScrollUp => MouseEventKind::ScrollUp,
        CrosstermMouseEventKind::ScrollDown => MouseEventKind::ScrollDown,
        CrosstermMouseEventKind::ScrollLeft => MouseEventKind::ScrollLeft,
        CrosstermMouseEventKind::ScrollRight => MouseEventKind::ScrollRight,
    }
}

/// Defines event source behavior
#[async_trait::async_trait]
pub trait EventSource: Send + Sync {
    /// Returns the next event, waiting up to `timeout_duration`
    async fn next_event(&mut self, timeout_duration: Duration) -> Result<CrosstermEvent>;
}
/// Represents crossterm event source
#[derive(Clone, Copy, Debug, Default)]
pub struct CrosstermEventSource;

#[async_trait::async_trait]
impl EventSource for CrosstermEventSource {
    async fn next_event(&mut self, timeout_duration: Duration) -> Result<CrosstermEvent> {
        let result = timeout(timeout_duration, async {
            let mut stream = EventStream::new();
            stream.next().await
        })
        .await;

        match result {
            Ok(Some(Ok(event))) => Ok(event),
            Ok(Some(Err(e))) => Err(WonderError::internal(format!("crossterm error: {e}"))),
            Ok(None) => Err(WonderError::internal("crossterm stream ended")),
            Err(_) => Err(WonderError::internal("event timeout")),
        }
    }
}
/// Enumerates turn state
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TurnState {
    /// Represents idle
    #[default]
    Idle,
    /// Represents editing input
    EditingInput,
    /// Represents command queued
    CommandQueued,
    /// Represents model request active
    ModelRequestActive,
    /// Represents tool permission pending
    ToolPermissionPending,
    /// Represents tool executing
    ToolExecuting,
    /// Represents streaming response
    StreamingResponse,
    /// Represents interrupted
    Interrupted,
    /// Represents completed
    Completed,
}
/// Represents event loop state
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventLoopState {
    /// Stores the turn state
    pub turn_state: TurnState,
    /// Stores the needs render
    pub needs_render: bool,
    /// Stores the exit requested
    pub exit_requested: bool,
}

impl Default for EventLoopState {
    fn default() -> Self {
        Self {
            turn_state: TurnState::Idle,
            needs_render: true,
            exit_requested: false,
        }
    }
}

impl EventLoopState {
    /// Updates state from a UI event
    pub fn observe(&mut self, event: &UiEvent) {
        match event {
            UiEvent::Key(key) if key.is_ctrl_char('c') => {
                self.turn_state = TurnState::Interrupted;
                self.exit_requested = true;
                self.needs_render = true;
            }
            UiEvent::Key(_) | UiEvent::Paste(_) => {
                if matches!(self.turn_state, TurnState::Idle | TurnState::Completed) {
                    self.turn_state = TurnState::EditingInput;
                }
                self.needs_render = true;
            }
            UiEvent::Mouse(_)
            | UiEvent::Resize { .. }
            | UiEvent::FocusGained
            | UiEvent::FocusLost
            | UiEvent::Tick => {
                self.needs_render = true;
            }
        }
    }

    /// Handles transition
    pub fn transition(&mut self, turn_state: TurnState) {
        self.turn_state = turn_state;
        self.needs_render = true;
    }

    /// Handles rendered
    pub fn rendered(&mut self) {
        self.needs_render = false;
    }
}
/// Represents event loop
#[derive(Debug)]
pub struct EventLoop<S> {
    source: S,
    tick_rate: Duration,
    last_tick: Instant,
}

impl<S: EventSource> EventLoop<S> {
    /// Creates a new value
    #[must_use]
    pub fn new(source: S, tick_rate: Duration) -> Self {
        Self {
            source,
            tick_rate,
            last_tick: Instant::now(),
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn tick_rate(&self) -> Duration {
        self.tick_rate
    }

    /// Handles next event
    pub async fn next_event(&mut self) -> Result<UiEvent> {
        self.next_event_at(Instant::now()).await
    }

    /// Handles next event at
    pub async fn next_event_at(&mut self, now: Instant) -> Result<UiEvent> {
        let elapsed = now.saturating_duration_since(self.last_tick);
        let timeout_duration = self.tick_rate.saturating_sub(elapsed);

        match self.source.next_event(timeout_duration).await {
            Ok(event) => Ok(UiEvent::from_crossterm(event).unwrap_or(UiEvent::Tick)),
            Err(_) => {
                self.last_tick = now;
                Ok(UiEvent::Tick)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, time::Duration};

    use super::*;

    #[derive(Debug, Default)]
    struct ScriptedEventSource {
        events: VecDeque<CrosstermEvent>,
        seen_timeouts: Vec<Duration>,
        poll_index: usize,
        poll_results: Vec<bool>,
    }

    impl ScriptedEventSource {
        fn with_events(poll_results: Vec<bool>, events: Vec<CrosstermEvent>) -> Self {
            Self {
                poll_results,
                events: events.into(),
                seen_timeouts: Vec::new(),
                poll_index: 0,
            }
        }
    }

    #[async_trait::async_trait]
    impl EventSource for ScriptedEventSource {
        async fn next_event(&mut self, timeout_duration: Duration) -> Result<CrosstermEvent> {
            self.seen_timeouts.push(timeout_duration);
            let has_event = self
                .poll_results
                .get(self.poll_index)
                .copied()
                .unwrap_or(false);
            self.poll_index += 1;
            if has_event {
                self.events
                    .pop_front()
                    .ok_or_else(|| WonderError::internal("no scripted terminal event"))
            } else {
                Err(WonderError::internal("event timeout"))
            }
        }
    }

    #[tokio::test]
    async fn normalize_key_event_ignores_releases() {
        let key = CrosstermKeyEvent::new(CrosstermKeyCode::Char('a'), event::KeyModifiers::NONE);
        let release = CrosstermKeyEvent {
            kind: KeyEventKind::Release,
            ..key
        };

        assert_eq!(
            normalize_key_event(key),
            Some(KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::default(),
            })
        );
        assert_eq!(normalize_key_event(release), None);
    }

    #[tokio::test]
    async fn event_loop_emits_tick_when_poll_times_out() {
        let source = ScriptedEventSource::with_events(vec![false], vec![]);
        let mut event_loop = EventLoop::new(source, Duration::from_millis(50));

        let event = event_loop.next_event().await.expect("tick");

        assert_eq!(event, UiEvent::Tick);
    }

    #[tokio::test]
    async fn event_loop_normalizes_keyboard_input() {
        let key = CrosstermKeyEvent::new(CrosstermKeyCode::Char('c'), event::KeyModifiers::CONTROL);
        let source = ScriptedEventSource::with_events(vec![true], vec![CrosstermEvent::Key(key)]);
        let mut event_loop = EventLoop::new(source, Duration::from_millis(100));

        let event = event_loop.next_event().await.expect("key event");

        assert_eq!(
            event,
            UiEvent::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers {
                    shift: false,
                    control: true,
                    alt: false,
                },
            })
        );
    }

    #[test]
    fn normalize_mouse_event_keeps_kind_coordinates_and_modifiers() {
        let raw = CrosstermMouseEvent {
            kind: CrosstermMouseEventKind::Drag(CrosstermMouseButton::Left),
            column: 7,
            row: 3,
            modifiers: event::KeyModifiers::SHIFT | event::KeyModifiers::ALT,
        };

        assert_eq!(
            normalize_mouse_event(raw),
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: 7,
                row: 3,
                modifiers: KeyModifiers {
                    shift: true,
                    control: false,
                    alt: true,
                },
            }
        );
    }

    #[test]
    fn click_tracker_ignores_drags_but_emits_clicks_on_release() {
        let mut tracker = ClickTracker::default();
        let down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 4,
            modifiers: KeyModifiers::default(),
        };
        let up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 2,
            row: 4,
            modifiers: KeyModifiers::default(),
        };

        assert_eq!(tracker.observe(down), None);
        assert_eq!(
            tracker.observe(up),
            Some(ClickEvent {
                button: MouseButton::Left,
                column: 2,
                row: 4,
                modifiers: KeyModifiers::default(),
            })
        );

        tracker.observe(down);
        tracker.observe(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 3,
            row: 4,
            modifiers: KeyModifiers::default(),
        });
        assert_eq!(tracker.observe(up), None);
    }

    #[test]
    fn loop_state_tracks_interrupt_request() {
        let key = KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers {
                control: true,
                ..KeyModifiers::default()
            },
        };
        let mut state = EventLoopState::default();

        state.observe(&UiEvent::Key(key));

        assert_eq!(state.turn_state, TurnState::Interrupted);
        assert!(state.exit_requested);
        assert!(state.needs_render);
    }
}
