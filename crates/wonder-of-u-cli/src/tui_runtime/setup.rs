//! Ephemeral TUI state for the `/setup` hub overlay.
//!
//! [`SetupOverlayState`] models the setup menu that opens when the user
//! runs `/setup` in the TUI.  It is purely ephemeral: it is never persisted
//! to [`AppState`] and is cleared whenever the controller navigates away.
//!
//! Each [`SetupItem`] maps one-to-one to an entry emitted by the `/setup`
//! command as a `setup_item=<json>` line.  Items that already have TUI flows
//! (model, theme, permissions, memory) carry a [`SetupItemAction::Dispatch`]
//! pointing at the slash-command string to execute.  Items whose full forms
//! are deferred to future todos carry [`SetupItemAction::Placeholder`].

/// The action the controller should take when a setup menu item is confirmed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SetupItemAction {
    /// Execute a slash-command to open an existing flow (e.g. `/model`).
    Dispatch(String),
    /// Show a placeholder notice while the full form is deferred.
    Placeholder(String),
}

/// A single entry in the setup hub menu.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SetupItem {
    /// Unique machine-readable identifier matching the `id` field in the JSON payload.
    pub(super) id: String,
    /// Human-readable label shown in the picker.
    pub(super) label: String,
    /// Short description shown below the label.
    pub(super) description: String,
    /// What the controller does when this item is confirmed.
    pub(super) action: SetupItemAction,
}

/// Ephemeral state for the setup hub overlay.
///
/// This is the single picker state that backs the `/setup` TUI menu.
/// It is created from the `setup_menu=true` + `setup_item=…` lines emitted
/// by the `SetupCommand` and cleared when the overlay is dismissed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SetupOverlayState {
    /// The raw command that opened this overlay (always `"/setup"`).
    pub(super) original_input: String,
    /// Ordered list of menu items, parsed from `setup_item=<json>` lines.
    pub(super) items: Vec<SetupItem>,
    /// Index into `items` of the currently highlighted row.
    pub(super) selected_index: usize,
    /// Provider selection label shown in the overlay title area.
    pub(super) provider_label: String,
    /// Provider readiness label shown in the overlay hint.
    pub(super) readiness_label: String,
}

impl SetupOverlayState {
    /// Builds a `SetupOverlayState` from the fields extracted by the parser.
    pub(super) fn new(
        items: Vec<SetupItem>,
        provider_label: String,
        readiness_label: String,
    ) -> Self {
        let selected_index = 0;
        Self {
            original_input: "/setup".into(),
            items,
            selected_index,
            provider_label,
            readiness_label,
        }
    }

    /// Clamps `selected_index` to the valid range after items change.
    pub(super) fn clamp_selection(&mut self) {
        if self.items.is_empty() {
            self.selected_index = 0;
        } else {
            self.selected_index = self.selected_index.min(self.items.len().saturating_sub(1));
        }
    }
}

/// Maps a setup menu item `id` to its concrete [`SetupItemAction`].
///
/// Items whose flows exist today resolve to `Dispatch("/<command>")`.
/// Items whose full forms are deferred carry a descriptive placeholder message.
pub(super) fn action_for_item_id(id: &str, _command: &str) -> SetupItemAction {
    match id {
        "model" => SetupItemAction::Dispatch("/model".into()),
        "theme" => SetupItemAction::Dispatch("/theme".into()),
        "permissions" => SetupItemAction::Dispatch("/permissions".into()),
        "memory" => SetupItemAction::Dispatch("/memory".into()),
        "terminal-setup" => SetupItemAction::Dispatch("/terminal-setup".into()),
        "keybindings" => SetupItemAction::Dispatch("/keybindings".into()),
        // Provider auth forms are deferred to `tui-provider-forms`.
        "login" => SetupItemAction::Placeholder(
            "Provider API-key login form is coming soon.\n\
             For now, run: wonder-of-u login --provider <name>"
                .into(),
        ),
        // Copilot OAuth is deferred to `tui-copilot-oauth`.
        "copilot-oauth" => SetupItemAction::Placeholder(
            "Copilot OAuth flow is coming soon.\n\
             For now, run: wonder-of-u login --provider copilot"
                .into(),
        ),
        // API base override form is deferred to `tui-provider-forms`.
        "api-base" => SetupItemAction::Placeholder(
            "API base override form is coming soon.\n\
             For now, edit the settings file or use: wonder-of-u config set api_base <url>"
                .into(),
        ),
        // Any unknown future items default to a generic placeholder.
        _ => SetupItemAction::Placeholder(format!(
            "'{id}' setup is not yet available in the TUI."
        )),
    }
}
