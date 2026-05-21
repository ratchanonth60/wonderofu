use crate::{
    event::{KeyCode, KeyEvent},
    input::{EditAction, Motion, TextBuffer},
    keymap::{KeyBindingContext, KeyBindingResolver, ResolvedKey, SystemAction, VimCommand},
};
/// Enumerates vim mode
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VimMode {
    /// Represents insert
    #[default]
    Insert,
    /// Represents normal
    Normal,
    /// Represents visual
    Visual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingOperator {
    Delete,
    Change,
    Yank,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FindDirection {
    Forward,
    Backward,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct VimRegister {
    text: String,
    linewise: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BufferSnapshot {
    text: String,
    cursor: usize,
}
/// Represents vim handle result
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VimHandleResult {
    /// Stores the system
    pub system: Option<SystemAction>,
    /// Stores the mode changed
    pub mode_changed: bool,
}
/// Represents vim state
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VimState {
    mode: VimMode,
    prefix_count: Option<usize>,
    motion_count: Option<usize>,
    pending_operator: Option<PendingOperator>,
    pending_find: Option<FindDirection>,
    register: VimRegister,
    undo_stack: Vec<BufferSnapshot>,
    redo_stack: Vec<BufferSnapshot>,
    visual_anchor: Option<usize>,
    last_find: Option<(char, FindDirection)>,
}

impl Default for VimState {
    fn default() -> Self {
        Self::new(VimMode::Insert)
    }
}

impl VimState {
    /// Creates a new vim state.
    #[must_use]
    pub fn new(mode: VimMode) -> Self {
        Self {
            mode,
            prefix_count: None,
            motion_count: None,
            pending_operator: None,
            pending_find: None,
            register: VimRegister {
                text: String::new(),
                linewise: false,
            },
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            visual_anchor: None,
            last_find: None,
        }
    }
    /// Constant fn
    #[must_use]
    pub const fn mode(&self) -> VimMode {
        self.mode
    }
    /// Constant fn
    #[must_use]
    pub const fn has_pending_operator(&self) -> bool {
        self.pending_operator.is_some()
    }

    /// Handles handle key
    pub fn handle_key(
        &mut self,
        buffer: &mut TextBuffer,
        resolver: &KeyBindingResolver,
        event: KeyEvent,
    ) -> VimHandleResult {
        if let Some(direction) = self.pending_find.take() {
            return self.finish_find(buffer, event, direction);
        }

        match self.mode {
            VimMode::Insert => self.handle_insert_mode(buffer, resolver, event),
            VimMode::Normal => self.handle_normal_mode(buffer, resolver, event),
            VimMode::Visual => self.handle_visual_mode(buffer, resolver, event),
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
                self.push_undo(buffer);
                buffer.apply_edit_action(action);
                self.redo_stack.clear();
                VimHandleResult::default()
            }
            Some(ResolvedKey::InsertChar(ch)) => {
                self.push_undo(buffer);
                buffer.insert_char(ch);
                self.redo_stack.clear();
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::EnterNormalMode))
            | Some(ResolvedKey::Vim(VimCommand::CancelPending)) => {
                buffer.enter_normal_mode();
                self.mode = VimMode::Normal;
                self.clear_pending();
                self.visual_anchor = None;
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
                    self.visual_anchor = None;
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
            Some(ResolvedKey::Vim(VimCommand::StartYank)) => {
                self.pending_operator = Some(PendingOperator::Yank);
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::StartChange)) => {
                self.pending_operator = Some(PendingOperator::Change);
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::PasteAfter)) => self.paste(buffer, true),
            Some(ResolvedKey::Vim(VimCommand::PasteBefore)) => self.paste(buffer, false),
            Some(ResolvedKey::Vim(VimCommand::Undo)) => self.undo(buffer),
            Some(ResolvedKey::Vim(VimCommand::Redo)) => self.redo(buffer),
            Some(ResolvedKey::Vim(VimCommand::EnterVisualMode)) => {
                self.mode = VimMode::Visual;
                self.visual_anchor = Some(buffer.cursor());
                self.clear_pending();
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            Some(ResolvedKey::Vim(VimCommand::FindForward)) => {
                self.pending_find = Some(FindDirection::Forward);
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::FindBackward)) => {
                self.pending_find = Some(FindDirection::Backward);
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::RepeatFind)) => self.repeat_find(buffer, false),
            Some(ResolvedKey::Vim(VimCommand::RepeatFindReverse)) => self.repeat_find(buffer, true),
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
        if let Some(operator) = self.pending_operator {
            if matches!(
                (operator, event.code),
                (PendingOperator::Delete, KeyCode::Char('d'))
                    | (PendingOperator::Change, KeyCode::Char('c'))
                    | (PendingOperator::Yank, KeyCode::Char('y'))
            ) {
                return Some(self.apply_line_operator(buffer, operator));
            }
        }

        let motion = operator_motion(event)?;
        let count = self.take_operator_count();
        let operator = self.pending_operator.take()?;
        let changed = match operator {
            PendingOperator::Delete | PendingOperator::Change => {
                let Some((start, end)) = buffer.normal_motion_range(motion, count) else {
                    return Some(VimHandleResult::default());
                };
                self.register.text = buffer.range_text(start, end);
                self.register.linewise = false;
                self.push_undo(buffer);
                let changed = buffer.delete_span(start, end);
                if changed {
                    self.redo_stack.clear();
                }
                changed
            }
            PendingOperator::Yank => {
                if let Some((start, end)) = buffer.normal_motion_range(motion, count) {
                    self.register.text = buffer.range_text(start, end);
                    self.register.linewise = false;
                }
                false
            }
        };

        self.prefix_count = None;
        self.motion_count = None;

        if changed && matches!(operator, PendingOperator::Change) {
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
            VimCommand::StartYank => {
                self.pending_operator = Some(PendingOperator::Yank);
                VimHandleResult::default()
            }
            VimCommand::StartChange => {
                self.pending_operator = Some(PendingOperator::Change);
                VimHandleResult::default()
            }
            VimCommand::PasteAfter => self.paste(buffer, true),
            VimCommand::PasteBefore => self.paste(buffer, false),
            VimCommand::Undo => self.undo(buffer),
            VimCommand::Redo => self.redo(buffer),
            VimCommand::EnterVisualMode => {
                self.mode = VimMode::Visual;
                self.visual_anchor = Some(buffer.cursor());
                self.clear_pending();
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            VimCommand::FindForward => {
                self.pending_find = Some(FindDirection::Forward);
                VimHandleResult::default()
            }
            VimCommand::FindBackward => {
                self.pending_find = Some(FindDirection::Backward);
                VimHandleResult::default()
            }
            VimCommand::RepeatFind => self.repeat_find(buffer, false),
            VimCommand::RepeatFindReverse => self.repeat_find(buffer, true),
            VimCommand::CancelPending => {
                self.clear_pending();
                VimHandleResult::default()
            }
            _ => VimHandleResult::default(),
        }
    }

    fn handle_visual_mode(
        &mut self,
        buffer: &mut TextBuffer,
        resolver: &KeyBindingResolver,
        event: KeyEvent,
    ) -> VimHandleResult {
        match resolver.resolve(KeyBindingContext::VimNormal, event) {
            Some(ResolvedKey::Edit(EditAction::Move(motion))) => {
                buffer.move_normal(motion, self.take_prefix_count());
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::CancelPending))
            | Some(ResolvedKey::Vim(VimCommand::EnterNormalMode)) => {
                self.mode = VimMode::Normal;
                self.visual_anchor = None;
                self.clear_pending();
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            Some(ResolvedKey::Vim(VimCommand::StartYank)) => {
                self.yank_visual(buffer);
                self.mode = VimMode::Normal;
                self.visual_anchor = None;
                VimHandleResult {
                    system: None,
                    mode_changed: true,
                }
            }
            Some(ResolvedKey::Vim(VimCommand::StartDelete)) => self.delete_visual(buffer, false),
            Some(ResolvedKey::Vim(VimCommand::StartChange)) => self.delete_visual(buffer, true),
            Some(ResolvedKey::Vim(VimCommand::FindForward)) => {
                self.pending_find = Some(FindDirection::Forward);
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::FindBackward)) => {
                self.pending_find = Some(FindDirection::Backward);
                VimHandleResult::default()
            }
            Some(ResolvedKey::Vim(VimCommand::RepeatFind)) => self.repeat_find(buffer, false),
            Some(ResolvedKey::Vim(VimCommand::RepeatFindReverse)) => self.repeat_find(buffer, true),
            _ => {
                if let Some(line_motion) = line_motion(event) {
                    buffer.move_normal(line_motion, self.take_prefix_count());
                } else if let Some(VimCommand::StartYank) = extra_normal_command(event) {
                    self.yank_visual(buffer);
                    self.mode = VimMode::Normal;
                    self.visual_anchor = None;
                    return VimHandleResult {
                        system: None,
                        mode_changed: true,
                    };
                } else if let Some(VimCommand::StartDelete) = extra_normal_command(event) {
                    return self.delete_visual(buffer, false);
                } else if let Some(VimCommand::StartChange) = extra_normal_command(event) {
                    return self.delete_visual(buffer, true);
                }
                VimHandleResult::default()
            }
        }
    }

    fn apply_line_operator(
        &mut self,
        buffer: &mut TextBuffer,
        operator: PendingOperator,
    ) -> VimHandleResult {
        let count = self.take_operator_count();
        self.pending_operator = None;
        let Some((start, end)) = buffer.current_line_range(count) else {
            return VimHandleResult::default();
        };
        self.register.text = buffer.range_text(start, end);
        self.register.linewise = true;

        match operator {
            PendingOperator::Yank => VimHandleResult::default(),
            PendingOperator::Delete | PendingOperator::Change => {
                self.push_undo(buffer);
                if buffer.delete_span(start, end) {
                    self.redo_stack.clear();
                }
                if matches!(operator, PendingOperator::Change) {
                    self.mode = VimMode::Insert;
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

    fn paste(&mut self, buffer: &mut TextBuffer, after: bool) -> VimHandleResult {
        if self.register.text.is_empty() {
            return VimHandleResult::default();
        }

        self.push_undo(buffer);
        let insert_at = if self.register.linewise {
            if after {
                buffer.next_line_insertion_index()
            } else {
                buffer.current_line_start()
            }
        } else if after {
            (buffer.cursor() + 1).min(buffer.char_len())
        } else {
            buffer.cursor().min(buffer.char_len())
        };
        buffer.insert_text_at(insert_at, &self.register.text);
        if !buffer.is_empty() {
            buffer.set_cursor(insert_at.min(buffer.char_len().saturating_sub(1)));
        }
        self.redo_stack.clear();
        VimHandleResult::default()
    }

    fn undo(&mut self, buffer: &mut TextBuffer) -> VimHandleResult {
        let Some(snapshot) = self.undo_stack.pop() else {
            return VimHandleResult::default();
        };
        self.redo_stack.push(BufferSnapshot {
            text: buffer.text(),
            cursor: buffer.cursor(),
        });
        buffer.replace_text(snapshot.text, snapshot.cursor);
        self.clear_pending();
        VimHandleResult::default()
    }

    fn redo(&mut self, buffer: &mut TextBuffer) -> VimHandleResult {
        let Some(snapshot) = self.redo_stack.pop() else {
            return VimHandleResult::default();
        };
        self.undo_stack.push(BufferSnapshot {
            text: buffer.text(),
            cursor: buffer.cursor(),
        });
        buffer.replace_text(snapshot.text, snapshot.cursor);
        self.clear_pending();
        VimHandleResult::default()
    }

    fn finish_find(
        &mut self,
        buffer: &mut TextBuffer,
        event: KeyEvent,
        direction: FindDirection,
    ) -> VimHandleResult {
        if let KeyCode::Char(ch) = event.code {
            let count = self.take_prefix_count();
            if let Some(index) =
                buffer.find_char(ch, matches!(direction, FindDirection::Forward), count)
            {
                buffer.set_cursor(index);
                self.last_find = Some((ch, direction));
            }
        }
        VimHandleResult::default()
    }

    fn repeat_find(&mut self, buffer: &mut TextBuffer, reverse: bool) -> VimHandleResult {
        let Some((ch, direction)) = self.last_find else {
            return VimHandleResult::default();
        };
        let direction = if reverse {
            match direction {
                FindDirection::Forward => FindDirection::Backward,
                FindDirection::Backward => FindDirection::Forward,
            }
        } else {
            direction
        };
        let count = self.take_prefix_count();
        if let Some(index) =
            buffer.find_char(ch, matches!(direction, FindDirection::Forward), count)
        {
            buffer.set_cursor(index);
            self.last_find = Some((ch, direction));
        }
        VimHandleResult::default()
    }

    fn yank_visual(&mut self, buffer: &TextBuffer) {
        if let Some((start, end)) = self.visual_range(buffer) {
            self.register.text = buffer.range_text(start, end);
            self.register.linewise = false;
        }
    }

    fn delete_visual(&mut self, buffer: &mut TextBuffer, change: bool) -> VimHandleResult {
        let Some((start, end)) = self.visual_range(buffer) else {
            return VimHandleResult::default();
        };
        self.register.text = buffer.range_text(start, end);
        self.register.linewise = false;
        self.push_undo(buffer);
        if buffer.delete_span(start, end) {
            self.redo_stack.clear();
        }
        self.visual_anchor = None;
        if change {
            self.mode = VimMode::Insert;
        } else {
            self.mode = VimMode::Normal;
        }
        VimHandleResult {
            system: None,
            mode_changed: true,
        }
    }

    fn visual_range(&self, buffer: &TextBuffer) -> Option<(usize, usize)> {
        let anchor = self.visual_anchor?;
        if buffer.is_empty() {
            return None;
        }
        let cursor = buffer.cursor().min(buffer.char_len().saturating_sub(1));
        let start = anchor.min(cursor);
        let end = anchor.max(cursor).saturating_add(1).min(buffer.char_len());
        (start < end).then_some((start, end))
    }

    fn push_undo(&mut self, buffer: &TextBuffer) {
        let snapshot = BufferSnapshot {
            text: buffer.text(),
            cursor: buffer.cursor(),
        };
        if self.undo_stack.last() != Some(&snapshot) {
            self.undo_stack.push(snapshot);
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
        self.pending_find = None;
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
        KeyCode::Char('y') => Some(VimCommand::StartYank),
        KeyCode::Char('p') => Some(VimCommand::PasteAfter),
        KeyCode::Char('P') => Some(VimCommand::PasteBefore),
        KeyCode::Char('u') => Some(VimCommand::Undo),
        KeyCode::Char('v') => Some(VimCommand::EnterVisualMode),
        KeyCode::Char('f') => Some(VimCommand::FindForward),
        KeyCode::Char('F') => Some(VimCommand::FindBackward),
        KeyCode::Char(';') => Some(VimCommand::RepeatFind),
        KeyCode::Char(',') => Some(VimCommand::RepeatFindReverse),
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

    #[test]
    fn normal_mode_yanks_and_pastes_lines() {
        let resolver = KeyBindingResolver::new();
        let mut buffer = TextBuffer::from_text("alpha\nbeta\n", true);
        let mut vim = VimState::new(VimMode::Normal);
        buffer.set_cursor(0);

        vim.handle_key(&mut buffer, &resolver, key('y'));
        vim.handle_key(&mut buffer, &resolver, key('y'));
        vim.handle_key(&mut buffer, &resolver, key('p'));

        assert_eq!(buffer.text(), "alpha\nalpha\nbeta\n");
        assert_eq!(vim.mode(), VimMode::Normal);
    }

    #[test]
    fn normal_mode_undo_and_redo_restore_mutations() {
        let resolver = KeyBindingResolver::new();
        let mut buffer = TextBuffer::from_text("alpha beta", true);
        let mut vim = VimState::new(VimMode::Normal);
        buffer.set_cursor(0);

        vim.handle_key(&mut buffer, &resolver, key('d'));
        vim.handle_key(&mut buffer, &resolver, key('w'));
        assert_eq!(buffer.text(), "beta");

        vim.handle_key(&mut buffer, &resolver, key('u'));
        assert_eq!(buffer.text(), "alpha beta");

        vim.handle_key(
            &mut buffer,
            &resolver,
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers: CONTROL,
            },
        );
        assert_eq!(buffer.text(), "beta");
    }

    #[test]
    fn visual_mode_yanks_and_deletes_selection() {
        let resolver = KeyBindingResolver::new();
        let mut buffer = TextBuffer::from_text("alpha beta", true);
        let mut vim = VimState::new(VimMode::Normal);
        buffer.set_cursor(0);

        vim.handle_key(&mut buffer, &resolver, key('v'));
        vim.handle_key(&mut buffer, &resolver, key('e'));
        vim.handle_key(&mut buffer, &resolver, key('y'));
        assert_eq!(vim.mode(), VimMode::Normal);

        buffer.set_cursor(6);
        vim.handle_key(&mut buffer, &resolver, key('P'));
        assert_eq!(buffer.text(), "alpha alphabeta");

        buffer.set_cursor(0);
        vim.handle_key(&mut buffer, &resolver, key('v'));
        vim.handle_key(&mut buffer, &resolver, key('e'));
        vim.handle_key(&mut buffer, &resolver, key('d'));
        assert_eq!(buffer.text(), " alphabeta");
    }

    #[test]
    fn normal_mode_find_and_repeat_find() {
        let resolver = KeyBindingResolver::new();
        let mut buffer = TextBuffer::from_text("abc abc abc", true);
        let mut vim = VimState::new(VimMode::Normal);
        buffer.set_cursor(0);

        vim.handle_key(&mut buffer, &resolver, key('f'));
        vim.handle_key(&mut buffer, &resolver, key('c'));
        assert_eq!(buffer.cursor(), 2);

        vim.handle_key(&mut buffer, &resolver, key(';'));
        assert_eq!(buffer.cursor(), 6);

        vim.handle_key(&mut buffer, &resolver, key(','));
        assert_eq!(buffer.cursor(), 2);
    }

    fn key(ch: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(ch),
            modifiers: NONE,
        }
    }
}
