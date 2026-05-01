//! Button and action-row view models.

/// Interaction flags surfaced by button components.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ButtonInteractionState {
    pub focused: bool,
    pub hovered: bool,
    pub active: bool,
}

impl ButtonInteractionState {
    #[must_use]
    pub const fn engaged(self) -> bool {
        self.focused || self.hovered || self.active
    }
}

/// Button metadata with tab-order and presentation hints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ButtonView {
    pub label: String,
    pub state: ButtonInteractionState,
    pub tab_index: i32,
    pub auto_focus: bool,
    pub primary: bool,
}

impl ButtonView {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            state: ButtonInteractionState::default(),
            tab_index: 0,
            auto_focus: false,
            primary: false,
        }
    }

    #[must_use]
    pub const fn state(mut self, state: ButtonInteractionState) -> Self {
        self.state = state;
        self
    }

    #[must_use]
    pub const fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }
}

/// One horizontal row of actions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActionRowView {
    pub buttons: Vec<ButtonView>,
    pub gap: u16,
}

impl ActionRowView {
    #[must_use]
    pub fn new(buttons: impl IntoIterator<Item = ButtonView>) -> Self {
        Self {
            buttons: buttons.into_iter().collect(),
            gap: 1,
        }
    }

    #[must_use]
    pub fn primary_index(&self) -> Option<usize> {
        self.buttons.iter().position(|button| button.primary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_row_finds_primary_button() {
        let row = ActionRowView::new([
            ButtonView::new("Cancel"),
            ButtonView::new("Confirm").primary(true),
        ]);

        assert_eq!(row.primary_index(), Some(1));
    }

    #[test]
    fn button_state_reports_engagement() {
        let state = ButtonInteractionState {
            hovered: true,
            ..ButtonInteractionState::default()
        };

        assert!(state.engaged());
    }
}
