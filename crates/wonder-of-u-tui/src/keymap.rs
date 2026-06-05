use crate::{
    event::{KeyCode, KeyEvent, KeyModifiers},
    input::{EditAction, Motion},
};
/// Enumerates key binding context
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyBindingContext {
    /// Represents any
    Any,
    /// Represents prompt
    Prompt,
    /// Represents vim insert
    VimInsert,
    /// Represents vim normal
    VimNormal,
    /// When navigating messages in the transcript with a cursor.
    MessageActions,
}
/// Enumerates system action
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemAction {
    /// Represents interrupt
    Interrupt,
    /// Represents redraw
    Redraw,
    /// Represents history search
    HistorySearch,
    /// Represents opening the workspace search overlay.
    OpenGlobalSearch,
    /// Represents expanding or collapsing grouped tool output
    ExpandToolOutput,
    /// Open the model picker from the prompt.
    OpenModelPicker,
    /// Toggle extended-thinking mode from the prompt.
    ToggleThinking,
    /// Toggle fast model mode from the prompt.
    ToggleFastMode,
}
/// Enumerates vim command
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimCommand {
    /// Represents enter insert mode
    EnterInsertMode,
    /// Represents enter normal mode
    EnterNormalMode,
    /// Represents append after cursor
    AppendAfterCursor,
    /// Represents append line end
    AppendLineEnd,
    /// Represents insert line start
    InsertLineStart,
    /// Represents delete char
    DeleteChar,
    /// Represents start delete
    StartDelete,
    /// Represents start change
    StartChange,
    /// Represents start yank
    StartYank,
    /// Represents paste after cursor
    PasteAfter,
    /// Represents paste before cursor
    PasteBefore,
    /// Represents undo
    Undo,
    /// Represents redo
    Redo,
    /// Represents enter visual mode
    EnterVisualMode,
    /// Represents find forward
    FindForward,
    /// Represents find backward
    FindBackward,
    /// Represents repeat find
    RepeatFind,
    /// Represents repeat find reverse
    RepeatFindReverse,
    /// Represents cancel pending
    CancelPending,
    /// Delete from cursor to end of line (equivalent to d$)
    DeleteToLineEnd,
    /// Delete from cursor to end of line then enter Insert mode (equivalent to c$)
    ChangeToLineEnd,
    /// Delete char before cursor (equivalent to dh/backspace in normal mode)
    DeleteCharBefore,
    /// Toggle case of char at cursor and advance one position
    ToggleCase,
    /// Open new line after current line and enter Insert mode
    OpenLineBelow,
    /// Open new line before current line and enter Insert mode
    OpenLineAbove,
    /// Replace single char at cursor with next keypress
    ReplaceChar,
}
/// Enumerates resolved key
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolvedKey {
    /// Represents edit
    Edit(EditAction),
    /// Represents insert char
    InsertChar(char),
    /// Represents system
    System(SystemAction),
    /// Represents vim
    Vim(VimCommand),
}
/// Represents key binding
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyBinding {
    /// Stores the context
    pub context: KeyBindingContext,
    /// Stores the event
    pub event: KeyEvent,
    /// Stores the result
    pub result: ResolvedKey,
}
/// Describes key binding error
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyBindingError {
    message: String,
}

impl std::fmt::Display for KeyBindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for KeyBindingError {}
/// Represents key binding resolver
#[derive(Clone, Debug)]
pub struct KeyBindingResolver {
    bindings: Vec<KeyBinding>,
}

impl Default for KeyBindingResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyBindingResolver {
    /// Creates a new value
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: default_bindings(),
        }
    }

    /// Handles with overrides
    pub fn with_overrides(
        overrides: impl IntoIterator<Item = KeyBinding>,
    ) -> Result<Self, KeyBindingError> {
        let overrides: Vec<KeyBinding> = overrides.into_iter().collect();
        validate_overrides(&overrides)?;

        let mut bindings = default_bindings();
        bindings.extend(overrides);
        Ok(Self { bindings })
    }
    /// Handles resolve
    #[must_use]
    pub fn resolve(&self, context: KeyBindingContext, event: KeyEvent) -> Option<ResolvedKey> {
        for binding in self.bindings.iter().rev() {
            if matches_context(binding.context, context) && binding.event == event {
                return Some(binding.result);
            }
        }

        if accepts_text_input(context) {
            return plain_text_key(event).map(ResolvedKey::InsertChar);
        }

        None
    }
    /// Handles bindings
    #[must_use]
    pub fn bindings(&self) -> &[KeyBinding] {
        &self.bindings
    }
}

fn validate_overrides(overrides: &[KeyBinding]) -> Result<(), KeyBindingError> {
    for binding in overrides {
        if is_reserved(binding.context, binding.event) {
            return Err(KeyBindingError {
                message: format!(
                    "reserved binding cannot be overridden for {:?}: {:?}",
                    binding.context, binding.event
                ),
            });
        }
    }

    Ok(())
}

fn is_reserved(context: KeyBindingContext, event: KeyEvent) -> bool {
    RESERVED_BINDINGS
        .iter()
        .any(|binding| matches_context(binding.context, context) && binding.event == event)
}

fn matches_context(expected: KeyBindingContext, actual: KeyBindingContext) -> bool {
    matches!(expected, KeyBindingContext::Any) || expected == actual
}

fn accepts_text_input(context: KeyBindingContext) -> bool {
    matches!(
        context,
        KeyBindingContext::Prompt | KeyBindingContext::VimInsert
    )
}

fn plain_text_key(event: KeyEvent) -> Option<char> {
    if event.modifiers.control || event.modifiers.alt {
        return None;
    }

    match event.code {
        KeyCode::Char(ch) => Some(ch),
        _ => None,
    }
}

fn default_bindings() -> Vec<KeyBinding> {
    let mut bindings = Vec::new();

    bindings.extend(RESERVED_BINDINGS);
    bindings.extend(prompt_bindings(KeyBindingContext::Prompt));
    bindings.extend(prompt_bindings(KeyBindingContext::VimInsert));
    bindings.extend(vim_normal_bindings());

    bindings
}

const NONE: KeyModifiers = KeyModifiers {
    shift: false,
    control: false,
    alt: false,
};

const SHIFT: KeyModifiers = KeyModifiers {
    shift: true,
    control: false,
    alt: false,
};

const CONTROL: KeyModifiers = KeyModifiers {
    shift: false,
    control: true,
    alt: false,
};

const ALT: KeyModifiers = KeyModifiers {
    shift: false,
    control: false,
    alt: true,
};

const RESERVED_BINDINGS: [KeyBinding; 5] = [
    KeyBinding {
        context: KeyBindingContext::Any,
        event: KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: CONTROL,
        },
        result: ResolvedKey::System(SystemAction::Interrupt),
    },
    KeyBinding {
        context: KeyBindingContext::Any,
        event: KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: CONTROL,
        },
        result: ResolvedKey::System(SystemAction::Interrupt),
    },
    KeyBinding {
        context: KeyBindingContext::Any,
        event: KeyEvent {
            code: KeyCode::Char('l'),
            modifiers: CONTROL,
        },
        result: ResolvedKey::System(SystemAction::Redraw),
    },
    KeyBinding {
        context: KeyBindingContext::Any,
        event: KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: CONTROL,
        },
        result: ResolvedKey::System(SystemAction::HistorySearch),
    },
    KeyBinding {
        context: KeyBindingContext::Any,
        event: KeyEvent {
            code: KeyCode::Char('f'),
            modifiers: CONTROL,
        },
        result: ResolvedKey::System(SystemAction::OpenGlobalSearch),
    },
];

fn prompt_bindings(context: KeyBindingContext) -> [KeyBinding; 17] {
    [
        bind(
            context,
            KeyCode::Left,
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Left)),
        ),
        bind(
            context,
            KeyCode::Right,
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Right)),
        ),
        bind(
            context,
            KeyCode::Up,
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Up)),
        ),
        bind(
            context,
            KeyCode::Down,
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Down)),
        ),
        bind(
            context,
            KeyCode::Home,
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::LineStart)),
        ),
        bind(
            context,
            KeyCode::End,
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::LineEnd)),
        ),
        bind(
            context,
            KeyCode::Char('a'),
            CONTROL,
            ResolvedKey::Edit(EditAction::Move(Motion::LineStart)),
        ),
        bind(
            context,
            KeyCode::Char('e'),
            CONTROL,
            ResolvedKey::Edit(EditAction::Move(Motion::LineEnd)),
        ),
        bind(
            context,
            KeyCode::Backspace,
            NONE,
            ResolvedKey::Edit(EditAction::Backspace),
        ),
        bind(
            context,
            KeyCode::Char('h'),
            CONTROL,
            ResolvedKey::Edit(EditAction::Backspace),
        ),
        bind(
            context,
            KeyCode::Delete,
            NONE,
            ResolvedKey::Edit(EditAction::Delete),
        ),
        bind(
            context,
            KeyCode::Enter,
            NONE,
            ResolvedKey::Edit(EditAction::InsertNewline),
        ),
        // Shift+Enter inserts a literal newline for multiline composition.
        bind(
            context,
            KeyCode::Enter,
            SHIFT,
            ResolvedKey::Edit(EditAction::InsertLiteralNewline),
        ),
        bind(
            context,
            KeyCode::Char('p'),
            ALT,
            ResolvedKey::System(SystemAction::OpenModelPicker),
        ),
        bind(
            context,
            KeyCode::Char('e'),
            ALT,
            ResolvedKey::System(SystemAction::ExpandToolOutput),
        ),
        bind(
            context,
            KeyCode::Char('t'),
            ALT,
            ResolvedKey::System(SystemAction::ToggleThinking),
        ),
        bind(
            context,
            KeyCode::Char('o'),
            ALT,
            ResolvedKey::System(SystemAction::ToggleFastMode),
        ),
    ]
}

fn vim_normal_bindings() -> [KeyBinding; 25] {
    [
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Left,
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Left)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Right,
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Right)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Up,
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Up)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Down,
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Down)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Esc,
            NONE,
            ResolvedKey::Vim(VimCommand::CancelPending),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('h'),
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Left)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('j'),
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Down)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('k'),
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Up)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('l'),
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::Right)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('w'),
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::WordForward)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('b'),
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::WordBackward)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('e'),
            NONE,
            ResolvedKey::Edit(EditAction::Move(Motion::WordEnd)),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('i'),
            NONE,
            ResolvedKey::Vim(VimCommand::EnterInsertMode),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('a'),
            NONE,
            ResolvedKey::Vim(VimCommand::AppendAfterCursor),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('x'),
            NONE,
            ResolvedKey::Vim(VimCommand::DeleteChar),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('y'),
            NONE,
            ResolvedKey::Vim(VimCommand::StartYank),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('p'),
            NONE,
            ResolvedKey::Vim(VimCommand::PasteAfter),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('P'),
            NONE,
            ResolvedKey::Vim(VimCommand::PasteBefore),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('u'),
            NONE,
            ResolvedKey::Vim(VimCommand::Undo),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('r'),
            CONTROL,
            ResolvedKey::Vim(VimCommand::Redo),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('v'),
            NONE,
            ResolvedKey::Vim(VimCommand::EnterVisualMode),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('f'),
            NONE,
            ResolvedKey::Vim(VimCommand::FindForward),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char('F'),
            NONE,
            ResolvedKey::Vim(VimCommand::FindBackward),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char(';'),
            NONE,
            ResolvedKey::Vim(VimCommand::RepeatFind),
        ),
        bind(
            KeyBindingContext::VimNormal,
            KeyCode::Char(','),
            NONE,
            ResolvedKey::Vim(VimCommand::RepeatFindReverse),
        ),
    ]
}

fn bind(
    context: KeyBindingContext,
    code: KeyCode,
    modifiers: KeyModifiers,
    result: ResolvedKey,
) -> KeyBinding {
    KeyBinding {
        context,
        event: KeyEvent { code, modifiers },
        result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_enter_resolves_to_insert_literal_newline_in_prompt_and_vim_insert() {
        let resolver = KeyBindingResolver::new();
        let shift_enter = KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers {
                shift: true,
                control: false,
                alt: false,
            },
        };

        assert_eq!(
            resolver.resolve(KeyBindingContext::Prompt, shift_enter),
            Some(ResolvedKey::Edit(EditAction::InsertLiteralNewline))
        );
        assert_eq!(
            resolver.resolve(KeyBindingContext::VimInsert, shift_enter),
            Some(ResolvedKey::Edit(EditAction::InsertLiteralNewline))
        );
        // Plain Enter still submits.
        let plain_enter = KeyEvent {
            code: KeyCode::Enter,
            modifiers: NONE,
        };
        assert_eq!(
            resolver.resolve(KeyBindingContext::Prompt, plain_enter),
            Some(ResolvedKey::Edit(EditAction::InsertNewline))
        );
    }

    #[test]
    fn resolver_maps_ctrl_bindings_and_printable_input() {
        let resolver = KeyBindingResolver::new();

        assert_eq!(
            resolver.resolve(
                KeyBindingContext::Prompt,
                KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: CONTROL,
                }
            ),
            Some(ResolvedKey::System(SystemAction::Interrupt))
        );
        assert_eq!(
            resolver.resolve(
                KeyBindingContext::Prompt,
                KeyEvent {
                    code: KeyCode::Char('f'),
                    modifiers: CONTROL,
                }
            ),
            Some(ResolvedKey::System(SystemAction::OpenGlobalSearch))
        );
        assert_eq!(
            resolver.resolve(
                KeyBindingContext::Prompt,
                KeyEvent {
                    code: KeyCode::Char('x'),
                    modifiers: NONE,
                }
            ),
            Some(ResolvedKey::InsertChar('x'))
        );
    }

    #[test]
    fn resolver_maps_meta_prompt_hotkeys() {
        let resolver = KeyBindingResolver::new();

        assert_eq!(
            resolver.resolve(
                KeyBindingContext::Prompt,
                KeyEvent {
                    code: KeyCode::Char('p'),
                    modifiers: ALT,
                }
            ),
            Some(ResolvedKey::System(SystemAction::OpenModelPicker))
        );
        assert_eq!(
            resolver.resolve(
                KeyBindingContext::Prompt,
                KeyEvent {
                    code: KeyCode::Char('e'),
                    modifiers: ALT,
                }
            ),
            Some(ResolvedKey::System(SystemAction::ExpandToolOutput))
        );
        assert_eq!(
            resolver.resolve(
                KeyBindingContext::Prompt,
                KeyEvent {
                    code: KeyCode::Char('t'),
                    modifiers: ALT,
                }
            ),
            Some(ResolvedKey::System(SystemAction::ToggleThinking))
        );
        assert_eq!(
            resolver.resolve(
                KeyBindingContext::Prompt,
                KeyEvent {
                    code: KeyCode::Char('o'),
                    modifiers: ALT,
                }
            ),
            Some(ResolvedKey::System(SystemAction::ToggleFastMode))
        );
    }

    #[test]
    fn resolver_exposes_vim_normal_commands() {
        let resolver = KeyBindingResolver::new();

        assert_eq!(
            resolver.resolve(
                KeyBindingContext::VimNormal,
                KeyEvent {
                    code: KeyCode::Char('w'),
                    modifiers: NONE,
                }
            ),
            Some(ResolvedKey::Edit(EditAction::Move(Motion::WordForward)))
        );
        assert_eq!(
            resolver.resolve(
                KeyBindingContext::VimNormal,
                KeyEvent {
                    code: KeyCode::Char('a'),
                    modifiers: NONE,
                }
            ),
            Some(ResolvedKey::Vim(VimCommand::AppendAfterCursor))
        );
    }

    #[test]
    fn overrides_cannot_replace_reserved_keys() {
        let error = KeyBindingResolver::with_overrides([KeyBinding {
            context: KeyBindingContext::Prompt,
            event: KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: CONTROL,
            },
            result: ResolvedKey::InsertChar('c'),
        }])
        .expect_err("reserved binding");

        assert!(error.to_string().contains("reserved binding"));
    }
}
