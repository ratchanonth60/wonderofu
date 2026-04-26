use crate::{
    event::{KeyCode, KeyEvent, KeyModifiers},
    input::{EditAction, Motion},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyBindingContext {
    Any,
    Prompt,
    VimInsert,
    VimNormal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemAction {
    Interrupt,
    Redraw,
    HistorySearch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimCommand {
    EnterInsertMode,
    EnterNormalMode,
    AppendAfterCursor,
    AppendLineEnd,
    InsertLineStart,
    DeleteChar,
    StartDelete,
    StartChange,
    CancelPending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolvedKey {
    Edit(EditAction),
    InsertChar(char),
    System(SystemAction),
    Vim(VimCommand),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyBinding {
    pub context: KeyBindingContext,
    pub event: KeyEvent,
    pub result: ResolvedKey,
}

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
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: default_bindings(),
        }
    }

    pub fn with_overrides(
        overrides: impl IntoIterator<Item = KeyBinding>,
    ) -> Result<Self, KeyBindingError> {
        let overrides: Vec<KeyBinding> = overrides.into_iter().collect();
        validate_overrides(&overrides)?;

        let mut bindings = default_bindings();
        bindings.extend(overrides);
        Ok(Self { bindings })
    }

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

const CONTROL: KeyModifiers = KeyModifiers {
    shift: false,
    control: true,
    alt: false,
};

const RESERVED_BINDINGS: [KeyBinding; 4] = [
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
];

fn prompt_bindings(context: KeyBindingContext) -> [KeyBinding; 12] {
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
    ]
}

fn vim_normal_bindings() -> [KeyBinding; 15] {
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
                    code: KeyCode::Char('x'),
                    modifiers: NONE,
                }
            ),
            Some(ResolvedKey::InsertChar('x'))
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
