use crate::{
    event::{KeyCode, KeyEvent},
    input::{EditAction, Motion, TextBuffer},
    keymap::{KeyBindingContext, KeyBindingResolver, ResolvedKey, SystemAction, VimCommand},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VimMode {
    #[default]
    Insert,
    Normal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingOperator {
    Delete,
    Change,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VimHandleResult {
    pub system: Option<SystemAction>,
    pub mode_changed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VimState {
    mode: VimMode,
    prefix_count: Option<usize>,
    motion_count: Option<usize>,
    pending_operator: Option<PendingOperator>,
}

impl Default for VimState {
    fn default() -> Self {
        Self::new(VimMode::Insert)
    }
}

impl VimState {
    #[must_use]
    pub const fn new(mode: VimMode) -> Self {
        Self {
            mode,
            prefix_count: None,
            motion_count: None,
            pending_operator: None,
        }
    }

    #[must_use]
    pub const fn mode(&self) -> VimMode {
        self.mode
    }

    #[must_use]
    pub const fn has_pending_operator(&self) -> bool {
        self.pending_operator.is_some()
    }

    pub fn handle_key(
        &mut self,
        buffer: &mut TextBuffer,
        resolver: &KeyBindingResolver,
        event: KeyEvent,
    ) -> VimHandleResult {
        match self.mode {
            VimMode::Insert => self.handle_insert_mode(buffer, resolver, event),
            VimMode::Normal => self.handle_normal_mode(buffer, resolver, event),
        }
    }

    fn handle_insert_mode(
        &mut self,
        buffer: &mut TextBuffer,
        resolver: &KeyBindingResolver,
        event: KeyEvent,
    ) -> VimHandleResult {
        match resolver.resolve(KeyBindingContext::VimInsert, event) {
            Some(ResolvedKey::System(system)) => VimHandleResult {
                system: Some(system),
                mode_changed: false,
            },
            Some(ResolvedKey::Edit(action)) => {
                buffer.apply_edit_action(action);
                VimHandleResult::default()
            }
            Some(ResolvedKey::InsertChar(ch)) => {
                buffer.insert_char(ch);
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::EnterNormalMode))
            | Some(ResolvedKey::Vim(VimCommand::CancelPending)) => {
                buffer.enter_normal_mode();
                self.mode = VimMode::Normal;
                self.clear_pending();
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            Some(ResolvedKey::Vim(_)) | None => {
                if event.code == KeyCode::Esc {
                    buffer.enter_normal_mode();
                    self.mode = VimMode::Normal;
                    self.clear_pending();
                    VimHandleResult {
                        system: None,
                        mode_changed: true,
                    }
                } else {
                    VimHandleResult::default()
                }
            }
        }
    }

    fn handle_normal_mode(
        &mut self,
        buffer: &mut TextBuffer,
        resolver: &KeyBindingResolver,
        event: KeyEvent,
    ) -> VimHandleResult {
        if let Some(digit) = count_digit(
            event,
            self.pending_operator.is_some(),
            self.prefix_count,
            self.motion_count,
        ) {
            self.push_count_digit(digit);
            return VimHandleResult::default();
        }

        if self.pending_operator.is_some() {
            match self.pending_operator_motion(buffer, event) {
                Some(handled) => return handled,
                None => {
                    self.clear_pending();
                }
            }
        }

        match resolver.resolve(KeyBindingContext::VimNormal, event) {
            Some(ResolvedKey::System(system)) => VimHandleResult {
                system: Some(system),
                mode_changed: false,
            },
            Some(ResolvedKey::Edit(EditAction::Move(Motion::Left))) => {
                buffer.move_normal(Motion::Left, self.take_prefix_count());
                VimHandleResult::default()
            }
            Some(ResolvedKey::Edit(EditAction::Move(Motion::Right))) => {
                buffer.move_normal(Motion::Right, self.take_prefix_count());
                VimHandleResult::default()
            }
            Some(ResolvedKey::Edit(EditAction::Move(Motion::Up))) => {
                buffer.move_normal(Motion::Up, self.take_prefix_count());
                VimHandleResult::default()
            }
            Some(ResolvedKey::Edit(EditAction::Move(Motion::Down))) => {
                buffer.move_normal(Motion::Down, self.take_prefix_count());
                VimHandleResult::default()
            }
            Some(ResolvedKey::Edit(EditAction::Move(Motion::WordForward))) => {
                buffer.move_normal(Motion::WordForward, self.take_prefix_count());
                VimHandleResult::default()
            }
            Some(ResolvedKey::Edit(EditAction::Move(Motion::WordBackward))) => {
                buffer.move_normal(Motion::WordBackward, self.take_prefix_count());
                VimHandleResult::default()
            }
            Some(ResolvedKey::Edit(EditAction::Move(Motion::WordEnd))) => {
                buffer.move_normal(Motion::WordEnd, self.take_prefix_count());
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::EnterInsertMode)) => {
                self.mode = VimMode::Insert;
                self.clear_pending();
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            Some(ResolvedKey::Vim(VimCommand::AppendAfterCursor)) => {
                buffer.append_after_cursor();
                self.mode = VimMode::Insert;
                self.clear_pending();
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            Some(ResolvedKey::Vim(VimCommand::DeleteChar)) => {
                buffer.delete_char_under_cursor(self.take_prefix_count());
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::StartDelete)) => {
                self.pending_operator = Some(PendingOperator::Delete);
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::StartChange)) => {
                self.pending_operator = Some(PendingOperator::Change);
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::AppendLineEnd)) => {
                buffer.append_line_end();
                self.mode = VimMode::Insert;
                self.clear_pending();
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            Some(ResolvedKey::Vim(VimCommand::InsertLineStart)) => {
                buffer.insert_line_start();
                self.mode = VimMode::Insert;
                self.clear_pending();
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            Some(ResolvedKey::Vim(VimCommand::CancelPending))
            | Some(ResolvedKey::Vim(VimCommand::EnterNormalMode))
            | None => {
                if let Some(line_motion) = line_motion(event) {
                    buffer.move_normal(line_motion, self.take_prefix_count());
                } else if let Some(vim_command) = extra_normal_command(event) {
                    return self.handle_extra_command(buffer, vim_command);
                } else {
                    self.clear_pending();
                }
                VimHandleResult::default()
            }
            Some(ResolvedKey::Edit(_)) | Some(ResolvedKey::InsertChar(_)) => {
                VimHandleResult::default()
            }
        }
    }

    fn pending_operator_motion(
        &mut self,
        buffer: &mut TextBuffer,
        event: KeyEvent,
    ) -> Option<VimHandleResult> {
        let motion = operator_motion(event)?;
        let count = self.take_operator_count();
        let changed = buffer.delete_motion(motion, count);

        let operator = self.pending_operator.take();
        self.prefix_count = None;
        self.motion_count = None;

        if changed && matches!(operator, Some(PendingOperator::Change)) {
            self.mode = VimMode::Insert;
            return Some(VimHandleResult {
                system: None,
                mode_changed: true,
            });
        }

        Some(VimHandleResult::default())
    }

    fn handle_extra_command(
        &mut self,
        buffer: &mut TextBuffer,
        command: VimCommand,
    ) -> VimHandleResult {
        match command {
            VimCommand::AppendLineEnd => {
                buffer.append_line_end();
                self.mode = VimMode::Insert;
                self.clear_pending();
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            VimCommand::InsertLineStart => {
                buffer.insert_line_start();
                self.mode = VimMode::Insert;
                self.clear_pending();
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            VimCommand::StartDelete => {
                self.pending_operator = Some(PendingOperator::Delete);
                VimHandleResult::default()
            }
            VimCommand::StartChange => {
                self.pending_operator = Some(PendingOperator::Change);
                VimHandleResult::default()
            }
            VimCommand::CancelPending => {
                self.clear_pending();
                VimHandleResult::default()
            }
            _ => VimHandleResult::default(),
        }
    }

    fn push_count_digit(&mut self, digit: usize) {
        let target = if self.pending_operator.is_some() {
            &mut self.motion_count
        } else {
            &mut self.prefix_count
        };

        let next = target
            .unwrap_or_default()
            .saturating_mul(10)
            .saturating_add(digit);
        *target = Some(next);
    }

    fn take_prefix_count(&mut self) -> usize {
        let count = self.prefix_count.take().unwrap_or(1);
        self.motion_count = None;
        count.max(1)
    }

    fn take_operator_count(&mut self) -> usize {
        let prefix = self.prefix_count.take().unwrap_or(1);
        let motion = self.motion_count.take().unwrap_or(1);
        prefix.saturating_mul(motion).max(1)
    }

    fn clear_pending(&mut self) {
        self.prefix_count = None;
        self.motion_count = None;
        self.pending_operator = None;
    }
}

fn count_digit(
    event: KeyEvent,
    operator_pending: bool,
    prefix_count: Option<usize>,
    motion_count: Option<usize>,
) -> Option<usize> {
    if event.modifiers.control || event.modifiers.alt {
        return None;
    }

    match event.code {
        KeyCode::Char(ch) if ch.is_ascii_digit() => {
            if ch == '0' && !operator_pending && prefix_count.is_none() && motion_count.is_none() {
                None
            } else {
                Some(ch.to_digit(10).unwrap_or_default() as usize)
            }
        }
        _ => None,
    }
}

fn operator_motion(event: KeyEvent) -> Option<Motion> {
    match event.code {
        KeyCode::Char('h') => Some(Motion::Left),
        KeyCode::Char('l') => Some(Motion::Right),
        KeyCode::Char('w') => Some(Motion::WordForward),
        KeyCode::Char('b') => Some(Motion::WordBackward),
        KeyCode::Char('e') => Some(Motion::WordEnd),
        KeyCode::Char('0') => Some(Motion::LineStart),
        KeyCode::Char('$') => Some(Motion::LineEnd),
        _ => None,
    }
}

fn line_motion(event: KeyEvent) -> Option<Motion> {
    match event.code {
        KeyCode::Char('0') => Some(Motion::LineStart),
        KeyCode::Char('$') => Some(Motion::LineEnd),
        _ => None,
    }
}

fn extra_normal_command(event: KeyEvent) -> Option<VimCommand> {
    match event.code {
        KeyCode::Char('A') => Some(VimCommand::AppendLineEnd),
        KeyCode::Char('I') => Some(VimCommand::InsertLineStart),
        KeyCode::Char('d') => Some(VimCommand::StartDelete),
        KeyCode::Char('c') => Some(VimCommand::StartChange),
        KeyCode::Esc => Some(VimCommand::CancelPending),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        event::{KeyCode, KeyEvent, KeyModifiers},
        keymap::KeyBindingResolver,
    };

    use super::*;

    const NONE: KeyModifiers = KeyModifiers {
        shift: false,
        control: false,
        alt: false,
    };
    const CONTROL: KeyModifiers = KeyModifiers {
        shift: false,
        control: true,
        alt: false,
    };

    #[test]
    fn insert_mode_edits_text_and_switches_to_normal() {
        let resolver = KeyBindingResolver::new();
        let mut buffer = TextBuffer::new(true);
        let mut vim = VimState::default();

        vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Char('h'),
                modifiers: NONE,
            },
        );
        vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Char('i'),
                modifiers: NONE,
            },
        );
        let result = vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: NONE,
            },
        );

        assert_eq!(buffer.text(), "hi");
        assert_eq!(vim.mode(), VimMode::Normal);
        assert!(result.mode_changed);
        assert_eq!(buffer.cursor(), 1);
    }

    #[test]
    fn normal_mode_supports_counts_and_delete_motion() {
        let resolver = KeyBindingResolver::new();
        let mut buffer = TextBuffer::from_text("alpha beta gamma", true);
        let mut vim = VimState::new(VimMode::Normal);
        buffer.set_cursor(0);

        vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Char('2'),
                modifiers: NONE,
            },
        );
        vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Char('w'),
                modifiers: NONE,
            },
        );

        assert_eq!(buffer.cursor(), 11);

        buffer.set_cursor(0);
        vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: NONE,
            },
        );
        let result = vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Char('w'),
                modifiers: NONE,
            },
        );

        assert_eq!(buffer.text(), "beta gamma");
        assert_eq!(vim.mode(), VimMode::Normal);
        assert!(!result.mode_changed);
    }

    #[test]
    fn change_motion_enters_insert_mode() {
        let resolver = KeyBindingResolver::new();
        let mut buffer = TextBuffer::from_text("alpha beta", true);
        let mut vim = VimState::new(VimMode::Normal);
        buffer.set_cursor(0);

        vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: NONE,
            },
        );
        let result = vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Char('w'),
                modifiers: NONE,
            },
        );

        assert_eq!(buffer.text(), "beta");
        assert_eq!(vim.mode(), VimMode::Insert);
        assert!(result.mode_changed);
    }

    #[test]
    fn ctrl_bindings_bubble_as_system_actions() {
        let resolver = KeyBindingResolver::new();
        let mut buffer = TextBuffer::new(true);
        let mut vim = VimState::default();

        let result = vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Char('l'),
                modifiers: CONTROL,
            },
        );

        assert_eq!(result.system, Some(SystemAction::Redraw));
    }
}
