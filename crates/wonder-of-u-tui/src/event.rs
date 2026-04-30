use std::{
    io,
    time::{Duration, Instant},
};

use crossterm::event::{
    self, Event as CrosstermEvent, KeyCode as CrosstermKeyCode, KeyEvent as CrosstermKeyEvent,
    KeyEventKind, MouseEvent,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCode {
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Delete,
    Insert,
    Esc,
    Char(char),
    F(u8),
    Null,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyModifiers {
    pub shift: bool,
    pub control: bool,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyEvent {
    #[must_use]
    pub fn is_ctrl_char(self, expected: char) -> bool {
        matches!(self.code, KeyCode::Char(actual)
            if actual.eq_ignore_ascii_case(&expected) && self.modifiers.control)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize { width: u16, height: u16 },
    FocusGained,
    FocusLost,
    Tick,
}

impl UiEvent {
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
}

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

pub trait EventSource {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<CrosstermEvent>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CrosstermEventSource;

impl EventSource for CrosstermEventSource {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        event::poll(timeout)
    }

    fn read(&mut self) -> io::Result<CrosstermEvent> {
        event::read()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TurnState {
    #[default]
    Idle,
    EditingInput,
    CommandQueued,
    ModelRequestActive,
    ToolPermissionPending,
    ToolExecuting,
    StreamingResponse,
    Interrupted,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventLoopState {
    pub turn_state: TurnState,
    pub needs_render: bool,
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

    pub fn transition(&mut self, turn_state: TurnState) {
        self.turn_state = turn_state;
        self.needs_render = true;
    }

    pub fn rendered(&mut self) {
        self.needs_render = false;
    }
}

#[derive(Debug)]
pub struct EventLoop<S> {
    source: S,
    tick_rate: Duration,
    last_tick: Instant,
}

impl<S: EventSource> EventLoop<S> {
    #[must_use]
    pub fn new(source: S, tick_rate: Duration) -> Self {
        Self {
            source,
            tick_rate,
            last_tick: Instant::now(),
        }
    }

    #[must_use]
    pub const fn tick_rate(&self) -> Duration {
        self.tick_rate
    }

    pub fn next_event(&mut self) -> io::Result<UiEvent> {
        self.next_event_at(Instant::now())
    }

    pub fn next_event_at(&mut self, now: Instant) -> io::Result<UiEvent> {
        let elapsed = now.saturating_duration_since(self.last_tick);
        let timeout = self.tick_rate.saturating_sub(elapsed);

        if self.source.poll(timeout)? {
            return self
                .source
                .read()
                .map(|event| UiEvent::from_crossterm(event).unwrap_or(UiEvent::Tick));
        }

        self.last_tick = now;
        Ok(UiEvent::Tick)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, time::Duration};

    use super::*;

    #[derive(Debug, Default)]
    struct ScriptedEventSource {
        polls: VecDeque<bool>,
        events: VecDeque<CrosstermEvent>,
        seen_timeouts: Vec<Duration>,
    }

    impl ScriptedEventSource {
        fn with_events(polls: Vec<bool>, events: Vec<CrosstermEvent>) -> Self {
            Self {
                polls: polls.into(),
                events: events.into(),
                seen_timeouts: Vec::new(),
            }
        }
    }

    impl EventSource for ScriptedEventSource {
        fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            self.seen_timeouts.push(timeout);
            Ok(self.polls.pop_front().unwrap_or(false))
        }

        fn read(&mut self) -> io::Result<CrosstermEvent> {
            self.events.pop_front().ok_or_else(|| {
                io::Error::new(io::ErrorKind::UnexpectedEof, "no scripted terminal event")
            })
        }
    }

    #[test]
    fn normalize_key_event_ignores_releases() {
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

    #[test]
    fn event_loop_emits_tick_when_poll_times_out() {
        let source = ScriptedEventSource::with_events(vec![false], vec![]);
        let mut event_loop = EventLoop::new(source, Duration::from_millis(50));
        let now = Instant::now();

        let event = event_loop
            .next_event_at(now + Duration::from_millis(50))
            .expect("tick");

        assert_eq!(event, UiEvent::Tick);
    }

    #[test]
    fn event_loop_normalizes_keyboard_input() {
        let key = CrosstermKeyEvent::new(CrosstermKeyCode::Char('c'), event::KeyModifiers::CONTROL);
        let source = ScriptedEventSource::with_events(vec![true], vec![CrosstermEvent::Key(key)]);
        let mut event_loop = EventLoop::new(source, Duration::from_millis(100));

        let event = event_loop.next_event().expect("key event");

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
