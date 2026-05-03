//! Ephemeral TUI state for the `/setup` hub overlay and provider login forms.
//!
//! [`SetupOverlayState`] models the setup menu opened by `/setup`.
//! [`ProviderFormState`] models the two-stage API-key / API-base form opened
//! from within that menu.  [`CopilotOAuthFlowState`] drives the Copilot
//! device-code OAuth dialog.  All are purely ephemeral and are never persisted.

use std::sync::mpsc;

use wonder_of_u_agent::{CopilotDeviceCode, CopilotOAuthToken};
use wonder_of_u_core::Result;
use wonder_of_u_tui::TextBuffer;

// ── Copilot OAuth flow ────────────────────────────────────────────────────────

/// Ephemeral state driving the Copilot device-code OAuth flow.
///
/// - `AwaitingConfirmation`: device code fetched; user must press Enter before
///   the browser is opened and polling begins.
/// - `Polling`: browser opened; a background thread is calling the GitHub token
///   endpoint.  The main thread checks the channel on each [`UiEvent::Tick`].
pub(super) enum CopilotOAuthFlowState {
    /// Device code fetched; dialog shown; waiting for user confirmation.
    AwaitingConfirmation {
        /// Full device-code response.  `device_code` is the ephemeral secret
        /// used for polling; `user_code` and `verification_uri` are safe to display.
        device_code: CopilotDeviceCode,
    },
    /// Browser opened; background thread is polling for the access token.
    Polling {
        /// User-visible code, safe to display.
        user_code: String,
        /// Receives the single polling result from the background thread.
        result_rx: mpsc::Receiver<Result<CopilotOAuthToken>>,
    },
}

impl std::fmt::Debug for CopilotOAuthFlowState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AwaitingConfirmation { device_code } => f
                .debug_struct("AwaitingConfirmation")
                .field("user_code", &device_code.user_code)
                .field("verification_uri", &device_code.verification_uri)
                // Never print the raw device_code; it is the ephemeral polling secret.
                .field("device_code", &"<redacted>")
                .finish(),
            Self::Polling { user_code, .. } => f
                .debug_struct("Polling")
                .field("user_code", user_code)
                .field("result_rx", &"<channel>")
                .finish(),
        }
    }
}

/// The kind of value the provider form collects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ProviderFormKind {
    /// Enter an API key for a provider.
    ApiKey,
    /// Override the base URL for a provider's API.
    ApiBase,
}

/// Which stage the two-step provider form is currently in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ProviderFormStage {
    /// Stage 1: the user is choosing which provider to configure.
    PickProvider,
    /// Stage 2: the user is typing the value (key or URL).
    EnterValue,
}

/// One row in the provider selection list.
#[derive(Clone, Debug)]
pub(super) struct ProviderFormOption {
    /// Matches `ProviderDescriptor::id`.
    pub(super) provider_id: String,
    /// Human-readable name shown in the picker.
    pub(super) display_name: String,
}

/// Ephemeral state for the provider API-key / API-base-URL overlay.
#[derive(Debug)]
pub(super) struct ProviderFormState {
    /// Which value type this form collects.
    pub(super) kind: ProviderFormKind,
    /// Current UI stage.
    pub(super) stage: ProviderFormStage,
    /// Selectable providers (filtered by kind at form open time).
    pub(super) options: Vec<ProviderFormOption>,
    /// Index of the currently highlighted row.
    pub(super) selected_index: usize,
    /// Text input for stage 2 (API key or URL).
    pub(super) input: TextBuffer,
}

impl ProviderFormState {
    /// Creates a new form in stage-1 (provider selection).
    pub(super) fn new(kind: ProviderFormKind, options: Vec<ProviderFormOption>) -> Self {
        Self {
            kind,
            stage: ProviderFormStage::PickProvider,
            options,
            selected_index: 0,
            input: TextBuffer::new(false),
        }
    }

    /// Returns the display name of the currently selected provider.
    pub(super) fn selected_display_name(&self) -> &str {
        self.options
            .get(self.selected_index)
            .map(|o| o.display_name.as_str())
            .unwrap_or("(none)")
    }

    /// Returns a display-safe representation of the staged input value.
    ///
    /// API keys are fully masked as bullet characters so they never appear in
    /// rendered output or test transcripts.
    pub(super) fn display_value(&self) -> String {
        match self.kind {
            // Replace every character with a bullet — length reveals nothing sensitive.
            ProviderFormKind::ApiKey => "\u{2022}".repeat(self.input.text().chars().count()),
            ProviderFormKind::ApiBase => self.input.text().to_string(),
        }
    }
}

// ── Setup hub ────────────────────────────────────────────────────────────────

/// The action the controller should take when a setup menu item is confirmed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SetupItemAction {
    /// Execute a slash-command to open an existing flow (e.g. `/model`).
    Dispatch(String),
    /// Show a placeholder notice while the full form is deferred.
    Placeholder(String),
    /// Open the two-stage provider form to collect an API key or base URL.
    ProviderForm(ProviderFormKind),
    /// Open the Copilot device-code OAuth dialog.
    CopilotOAuth,
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
    /// Builds a `SetupOverlayState` from the parsed fields.
    pub(super) fn new(
        items: Vec<SetupItem>,
        provider_label: String,
        readiness_label: String,
    ) -> Self {
        Self {
            original_input: "/setup".into(),
            items,
            selected_index: 0,
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
/// Items with existing TUI flows resolve to `Dispatch("/<command>")`.
/// Items with native provider forms resolve to `ProviderForm(kind)`.
/// Deferred items carry a descriptive `Placeholder` message.
pub(super) fn action_for_item_id(id: &str, _command: &str) -> SetupItemAction {
    match id {
        "model" => SetupItemAction::Dispatch("/model".into()),
        "theme" => SetupItemAction::Dispatch("/theme".into()),
        "permissions" => SetupItemAction::Dispatch("/permissions".into()),
        "memory" => SetupItemAction::Dispatch("/memory".into()),
        "terminal-setup" => SetupItemAction::Dispatch("/terminal-setup".into()),
        "keybindings" => SetupItemAction::Dispatch("/keybindings".into()),
        // Opens the two-stage API-key entry form.
        "login" => SetupItemAction::ProviderForm(ProviderFormKind::ApiKey),
        // Opens the two-stage API-base URL override form.
        "api-base" => SetupItemAction::ProviderForm(ProviderFormKind::ApiBase),
        // Copilot OAuth uses the built-in device-code flow.
        "copilot-oauth" => SetupItemAction::CopilotOAuth,
        _ => SetupItemAction::Placeholder(format!("'{id}' setup is not yet available in the TUI.")),
    }
}
