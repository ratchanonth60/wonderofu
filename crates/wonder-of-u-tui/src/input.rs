/// Enumerates motion
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Motion {
    /// Represents left
    Left,
    /// Represents right
    Right,
    /// Represents up
    Up,
    /// Represents down
    Down,
    /// Represents line start
    LineStart,
    /// Represents line end
    LineEnd,
    /// Represents first non blank
    FirstNonBlank,
    /// Represents word forward
    WordForward,
    /// Represents word backward
    WordBackward,
    /// Represents word end
    WordEnd,
}
/// Enumerates edit action
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditAction {
    /// Represents move
    Move(Motion),
    /// Inserts a newline — the controller interprets this as **submit**.
    ///
    /// Plain `Enter` maps to this action; use [`InsertLiteralNewline`] for
    /// `Shift+Enter` multiline composition.
    ///
    /// [`InsertLiteralNewline`]: EditAction::InsertLiteralNewline
    InsertNewline,
    /// Inserts a literal `\n` into the buffer without triggering submit.
    ///
    /// Only meaningful for multiline prompt buffers; single-line buffers
    /// silently discard the character (see [`TextBuffer::insert_char`]).
    InsertLiteralNewline,
    /// Represents backspace
    Backspace,
    /// Represents delete
    Delete,
}
/// Represents text buffer
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextBuffer {
    chars: Vec<char>,
    cursor: usize,
    multiline: bool,
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new(false)
    }
}

impl TextBuffer {
    /// Creates a new value
    #[must_use]
    pub fn new(multiline: bool) -> Self {
        Self {
            chars: Vec::new(),
            cursor: 0,
            multiline,
        }
    }
    /// Handles from text
    #[must_use]
    pub fn from_text(text: impl AsRef<str>, multiline: bool) -> Self {
        let chars: Vec<char> = text.as_ref().chars().collect();
        let cursor = chars.len();
        Self {
            chars,
            cursor,
            multiline,
        }
    }
    /// Handles text
    #[must_use]
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }
    /// Constant fn
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }
    /// Handles char len
    #[must_use]
    pub fn char_len(&self) -> usize {
        self.chars.len()
    }
    /// Constant fn
    #[must_use]
    pub const fn is_multiline(&self) -> bool {
        self.multiline
    }
    /// Returns whether empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Handles set cursor
    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.chars.len());
    }

    /// Replaces the full buffer content and restores the cursor.
    pub fn replace_text(&mut self, text: impl AsRef<str>, cursor: usize) {
        self.chars = text.as_ref().chars().collect();
        self.cursor = cursor.min(self.chars.len());
    }

    /// Handles insert char
    pub fn insert_char(&mut self, ch: char) {
        if ch == '\n' && !self.multiline {
            return;
        }

        self.chars.insert(self.cursor, ch);
        self.cursor += 1;
    }

    /// Handles insert text
    pub fn insert_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.insert_char(ch);
        }
    }

    /// Inserts text at an absolute character offset.
    pub fn insert_text_at(&mut self, index: usize, text: &str) {
        self.cursor = index.min(self.chars.len());
        self.insert_text(text);
    }

    /// Returns the text in an absolute character range.
    #[must_use]
    pub fn range_text(&self, start: usize, end: usize) -> String {
        let start = start.min(self.chars.len());
        let end = end.min(self.chars.len());
        if start >= end {
            return String::new();
        }

        self.chars[start..end].iter().collect()
    }

    /// Handles apply edit action
    pub fn apply_edit_action(&mut self, action: EditAction) {
        match action {
            EditAction::Move(motion) => self.move_caret(motion, 1),
            // InsertNewline is intercepted by the controller as "submit";
            // hitting it through the buffer directly still inserts '\n'.
            EditAction::InsertNewline | EditAction::InsertLiteralNewline => {
                self.insert_char('\n');
            }
            EditAction::Backspace => {
                self.backspace();
            }
            EditAction::Delete => {
                self.delete();
            }
        }
    }

    /// Handles move caret
    pub fn move_caret(&mut self, motion: Motion, count: usize) {
        let count = count.max(1);
        match motion {
            Motion::Left => self.cursor = self.cursor.saturating_sub(count),
            Motion::Right => self.cursor = (self.cursor + count).min(self.chars.len()),
            Motion::Up => self.cursor = self.vertical_move_caret(false, count),
            Motion::Down => self.cursor = self.vertical_move_caret(true, count),
            Motion::LineStart => self.cursor = self.line_start(self.cursor),
            Motion::LineEnd => self.cursor = self.line_end(self.cursor),
            Motion::FirstNonBlank => self.cursor = self.first_non_blank_in_line(self.cursor),
            Motion::WordForward => self.cursor = self.next_word_start(self.cursor, count),
            Motion::WordBackward => self.cursor = self.prev_word_start(self.cursor, count),
            Motion::WordEnd => self.cursor = self.word_end(self.cursor, count).saturating_add(1),
        }
    }

    /// Handles move normal
    pub fn move_normal(&mut self, motion: Motion, count: usize) {
        if self.chars.is_empty() {
            self.cursor = 0;
            return;
        }

        let count = count.max(1);
        let current = self.normal_cursor();
        self.cursor = match motion {
            Motion::Left => current.saturating_sub(count),
            Motion::Right => (current + count).min(self.chars.len().saturating_sub(1)),
            Motion::Up => self.vertical_move_normal(current, false, count),
            Motion::Down => self.vertical_move_normal(current, true, count),
            Motion::LineStart => self.line_start(current),
            Motion::LineEnd => self.line_last_char(current),
            Motion::FirstNonBlank => self.first_non_blank_normal(current),
            Motion::WordForward => self.next_word_start(current, count).min(self.last_cursor()),
            Motion::WordBackward => self.prev_word_start(current, count),
            Motion::WordEnd => self.word_end(current, count),
        };
    }

    /// Handles enter normal mode
    pub fn enter_normal_mode(&mut self) {
        if self.chars.is_empty() {
            self.cursor = 0;
            return;
        }

        if self.cursor > 0 {
            let previous = self.chars[self.cursor - 1];
            if previous != '\n' {
                self.cursor -= 1;
            }
        }

        self.cursor = self.cursor.min(self.last_cursor());
    }

    /// Handles append after cursor
    pub fn append_after_cursor(&mut self) {
        if self.chars.is_empty() {
            self.cursor = 0;
            return;
        }

        self.cursor = (self.normal_cursor() + 1).min(self.chars.len());
    }

    /// Handles append line end
    pub fn append_line_end(&mut self) {
        self.cursor = self.line_end(self.normal_cursor_for_insert());
    }

    /// Handles insert line start
    pub fn insert_line_start(&mut self) {
        self.cursor = self.first_non_blank_in_line(self.normal_cursor_for_insert());
    }

    /// Handles backspace
    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }

        self.cursor -= 1;
        self.chars.remove(self.cursor);
        true
    }

    /// Handles delete
    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.chars.len() {
            return false;
        }

        self.chars.remove(self.cursor);
        true
    }

    /// Handles delete char under cursor
    pub fn delete_char_under_cursor(&mut self, count: usize) -> bool {
        if self.chars.is_empty() {
            return false;
        }

        let cursor = self.normal_cursor();
        let end = (cursor + count.max(1)).min(self.chars.len());
        self.delete_range(cursor, end);
        if !self.chars.is_empty() {
            self.cursor = cursor.min(self.last_cursor());
        } else {
            self.cursor = 0;
        }
        true
    }

    /// Handles delete motion
    pub fn delete_motion(&mut self, motion: Motion, count: usize) -> bool {
        let Some((start, end)) = self.motion_delete_range(motion, count) else {
            return false;
        };

        self.delete_range(start, end);
        if self.chars.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor = start.min(self.last_cursor());
        }
        true
    }

    /// Deletes an absolute character range and normalizes the cursor.
    pub fn delete_span(&mut self, start: usize, end: usize) -> bool {
        let start = start.min(self.chars.len());
        let end = end.min(self.chars.len());
        if start >= end {
            return false;
        }

        self.delete_range(start, end);
        if self.chars.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor = start.min(self.last_cursor());
        }
        true
    }

    /// Returns the deletion/yank range that would be affected by a normal-mode motion.
    #[must_use]
    pub fn normal_motion_range(&self, motion: Motion, count: usize) -> Option<(usize, usize)> {
        self.motion_delete_range(motion, count)
    }

    /// Returns a range covering `count` complete lines from the current line.
    #[must_use]
    pub fn current_line_range(&self, count: usize) -> Option<(usize, usize)> {
        if self.chars.is_empty() {
            return None;
        }

        let start = self.line_start(self.normal_cursor());
        let mut end = start;
        for _ in 0..count.max(1) {
            end = self.line_end(end);
            if end < self.chars.len() {
                end += 1;
            } else {
                break;
            }
        }
        (start < end).then_some((start, end))
    }

    /// Returns the current line start.
    #[must_use]
    pub fn current_line_start(&self) -> usize {
        self.line_start(self.normal_cursor_for_insert())
    }

    /// Returns the insertion point immediately after the current line.
    #[must_use]
    pub fn next_line_insertion_index(&self) -> usize {
        let end = self.line_end(self.normal_cursor_for_insert());
        if end < self.chars.len() {
            end + 1
        } else {
            self.chars.len()
        }
    }

    /// Finds a character from the normal cursor, excluding the current cell.
    #[must_use]
    pub fn find_char(&self, ch: char, forward: bool, count: usize) -> Option<usize> {
        if self.chars.is_empty() {
            return None;
        }

        let mut remaining = count.max(1);
        if forward {
            let mut index = self.normal_cursor().saturating_add(1);
            while index < self.chars.len() {
                if self.chars[index] == ch {
                    remaining -= 1;
                    if remaining == 0 {
                        return Some(index);
                    }
                }
                index += 1;
            }
        } else {
            let mut index = self.normal_cursor();
            while index > 0 {
                index -= 1;
                if self.chars[index] == ch {
                    remaining -= 1;
                    if remaining == 0 {
                        return Some(index);
                    }
                }
            }
        }

        None
    }

    fn motion_delete_range(&self, motion: Motion, count: usize) -> Option<(usize, usize)> {
        if self.chars.is_empty() {
            return None;
        }

        let current = self.normal_cursor();
        let count = count.max(1);
        let range = match motion {
            Motion::Left | Motion::WordBackward | Motion::LineStart | Motion::FirstNonBlank => {
                let target = match motion {
                    Motion::Left => current.saturating_sub(count),
                    Motion::WordBackward => self.prev_word_start(current, count),
                    Motion::LineStart => self.line_start(current),
                    Motion::FirstNonBlank => self.first_non_blank_normal(current),
                    _ => unreachable!(),
                };
                (target.min(current), current)
            }
            Motion::Right => {
                let target = (current + count).min(self.chars.len());
                (current, target)
            }
            Motion::WordForward => {
                let target = self.next_word_start(current, count);
                (current, target.max(current + 1).min(self.chars.len()))
            }
            Motion::WordEnd => {
                let target = self.word_end(current, count).saturating_add(1);
                (current, target.min(self.chars.len()))
            }
            Motion::LineEnd => {
                let end = self.line_end(current);
                (current, end)
            }
            Motion::Up | Motion::Down => return None,
        };

        (range.0 < range.1).then_some(range)
    }

    /// Delete a range of characters between `start` (inclusive) and `end` (exclusive).
    pub fn delete_range(&mut self, start: usize, end: usize) {
        self.chars.drain(start..end);
    }

    fn normal_cursor(&self) -> usize {
        if self.chars.is_empty() {
            0
        } else {
            self.cursor.min(self.last_cursor())
        }
    }

    fn normal_cursor_for_insert(&self) -> usize {
        self.normal_cursor().min(self.chars.len())
    }

    fn last_cursor(&self) -> usize {
        self.chars.len().saturating_sub(1)
    }

    fn vertical_move_caret(&self, down: bool, count: usize) -> usize {
        let mut index = self.cursor.min(self.chars.len());
        let column = index.saturating_sub(self.line_start(index));
        for _ in 0..count {
            index = if down {
                self.next_line_start(index)
            } else {
                self.prev_line_start(index)
            };
        }

        let line_end = self.line_end(index);
        (index + column).min(line_end)
    }

    fn vertical_move_normal(&self, current: usize, down: bool, count: usize) -> usize {
        let mut index = current;
        let column = current.saturating_sub(self.line_start(current));
        for _ in 0..count {
            let next = if down {
                self.next_line_start(index)
            } else {
                self.prev_line_start(index)
            };
            if next == index {
                break;
            }
            index = next;
        }

        let line_start = self.line_start(index);
        let line_end = self.line_end(index);
        if line_start == line_end {
            line_start
        } else {
            (line_start + column).min(line_end.saturating_sub(1))
        }
    }

    fn line_start(&self, cursor: usize) -> usize {
        let index = cursor.min(self.chars.len());
        let mut start = index;
        while start > 0 && self.chars[start - 1] != '\n' {
            start -= 1;
        }
        start
    }

    fn line_end(&self, cursor: usize) -> usize {
        let mut end = cursor.min(self.chars.len());
        while end < self.chars.len() && self.chars[end] != '\n' {
            end += 1;
        }
        end
    }

    fn line_last_char(&self, cursor: usize) -> usize {
        let line_start = self.line_start(cursor);
        let line_end = self.line_end(cursor);
        if line_start == line_end {
            line_start
        } else {
            line_end - 1
        }
    }

    fn first_non_blank_in_line(&self, cursor: usize) -> usize {
        let line_start = self.line_start(cursor);
        let line_end = self.line_end(cursor);
        let mut index = line_start;
        while index < line_end && self.chars[index].is_whitespace() && self.chars[index] != '\n' {
            index += 1;
        }
        index
    }

    fn first_non_blank_normal(&self, cursor: usize) -> usize {
        let position = self.first_non_blank_in_line(cursor);
        position.min(self.line_last_char(cursor))
    }

    fn next_line_start(&self, cursor: usize) -> usize {
        let line_end = self.line_end(cursor);
        if line_end < self.chars.len() {
            line_end + 1
        } else {
            self.line_start(cursor.min(self.chars.len()))
        }
    }

    fn prev_line_start(&self, cursor: usize) -> usize {
        let current_start = self.line_start(cursor);
        if current_start == 0 {
            0
        } else {
            self.line_start(current_start - 1)
        }
    }

    fn next_word_start(&self, start: usize, count: usize) -> usize {
        let mut index = start.min(self.chars.len());
        for _ in 0..count {
            if index >= self.chars.len() {
                return self.chars.len();
            }

            let current_is_word = self.chars.get(index).is_some_and(|ch| is_word_char(*ch));
            if current_is_word {
                while index < self.chars.len() && is_word_char(self.chars[index]) {
                    index += 1;
                }
            }

            while index < self.chars.len() && !is_word_char(self.chars[index]) {
                index += 1;
            }
        }
        index
    }

    fn prev_word_start(&self, start: usize, count: usize) -> usize {
        let mut index = start.min(self.chars.len());
        for _ in 0..count {
            if index == 0 {
                return 0;
            }

            index -= 1;
            while index > 0 && !is_word_char(self.chars[index]) {
                index -= 1;
            }
            while index > 0 && is_word_char(self.chars[index - 1]) {
                index -= 1;
            }
        }
        index
    }

    fn word_end(&self, start: usize, count: usize) -> usize {
        if self.chars.is_empty() {
            return 0;
        }

        let mut index = start.min(self.last_cursor());
        for step in 0..count {
            if !is_word_char(self.chars[index]) {
                while index < self.chars.len() && !is_word_char(self.chars[index]) {
                    if index == self.last_cursor() {
                        return index;
                    }
                    index += 1;
                }
            }

            while index + 1 < self.chars.len() && is_word_char(self.chars[index + 1]) {
                index += 1;
            }

            if step + 1 < count {
                index = (index + 1).min(self.last_cursor());
                while index < self.chars.len() && !is_word_char(self.chars[index]) {
                    if index == self.last_cursor() {
                        return index;
                    }
                    index += 1;
                }
            }
        }
        index
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_buffer_supports_insertion_and_vertical_navigation() {
        let mut buffer = TextBuffer::new(true);
        buffer.insert_text("alpha\nbeta");
        buffer.set_cursor(2);

        buffer.move_caret(Motion::Down, 1);
        assert_eq!(buffer.cursor(), 8);

        buffer.apply_edit_action(EditAction::InsertNewline);
        buffer.insert_text("x");

        assert_eq!(buffer.text(), "alpha\nbe\nxta");
    }

    #[test]
    fn single_line_buffer_ignores_newlines() {
        let mut buffer = TextBuffer::new(false);
        buffer.insert_text("abc");
        buffer.insert_char('\n');

        assert_eq!(buffer.text(), "abc");
    }

    #[test]
    fn normal_mode_delete_motion_and_append_behave_safely() {
        let mut buffer = TextBuffer::from_text("alpha beta", true);
        buffer.enter_normal_mode();
        buffer.move_normal(Motion::LineStart, 1);

        assert!(buffer.delete_motion(Motion::WordForward, 1));
        assert_eq!(buffer.text(), "beta");
        assert_eq!(buffer.cursor(), 0);

        buffer.append_after_cursor();
        buffer.insert_text("A");
        assert_eq!(buffer.text(), "bAeta");
    }

    #[test]
    fn insert_literal_newline_inserts_into_multiline_buffer() {
        let mut buf = TextBuffer::new(true);
        buf.insert_text("hello");
        buf.apply_edit_action(EditAction::InsertLiteralNewline);
        buf.insert_text("world");
        assert_eq!(buf.text(), "hello\nworld");
        assert_eq!(buf.cursor(), 11);
    }

    #[test]
    fn insert_literal_newline_is_blocked_in_single_line_buffer() {
        let mut buf = TextBuffer::new(false);
        buf.insert_text("hello");
        buf.apply_edit_action(EditAction::InsertLiteralNewline);
        assert_eq!(buf.text(), "hello");
    }

    #[test]
    fn backspace_and_delete_are_boundary_safe() {
        let mut buffer = TextBuffer::from_text("abc", false);
        buffer.set_cursor(1);
        assert!(buffer.backspace());
        assert_eq!(buffer.text(), "bc");
        assert_eq!(buffer.cursor(), 0);

        assert!(buffer.delete());
        assert_eq!(buffer.text(), "c");
        assert!(!buffer.backspace());
    }
}
