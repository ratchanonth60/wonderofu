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

#[cfg(test)]
mod tests {
    use super::*;

    // ── action_for_item_id ────────────────────────────────────────────────────

    /// Each known dispatch item must resolve to the expected slash-command.
    #[test]
    fn action_for_item_id_dispatch_items() {
        let cases = [
            ("model", "/model"),
            ("theme", "/theme"),
            ("permissions", "/permissions"),
            ("memory", "/memory"),
            ("terminal-setup", "/terminal-setup"),
            ("keybindings", "/keybindings"),
        ];
        for (id, cmd) in cases {
            let action = action_for_item_id(id, "/setup");
            assert_eq!(
                action,
                SetupItemAction::Dispatch(cmd.into()),
                "id={id:?} should dispatch {cmd:?}"
            );
        }
    }

    #[test]
    fn action_for_item_id_login_opens_api_key_form() {
        let action = action_for_item_id("login", "/setup");
        assert_eq!(
            action,
            SetupItemAction::ProviderForm(ProviderFormKind::ApiKey)
        );
    }

    #[test]
    fn action_for_item_id_api_base_opens_api_base_form() {
        let action = action_for_item_id("api-base", "/setup");
        assert_eq!(
            action,
            SetupItemAction::ProviderForm(ProviderFormKind::ApiBase)
        );
    }

    #[test]
    fn action_for_item_id_copilot_oauth() {
        let action = action_for_item_id("copilot-oauth", "/setup");
        assert_eq!(action, SetupItemAction::CopilotOAuth);
    }

    #[test]
    fn action_for_item_id_unknown_id_returns_placeholder() {
        let action = action_for_item_id("totally-unknown-xyz", "/setup");
        assert!(
            matches!(action, SetupItemAction::Placeholder(_)),
            "unknown id should fall back to Placeholder, got {action:?}"
        );
        if let SetupItemAction::Placeholder(msg) = action {
            assert!(
                msg.contains("totally-unknown-xyz"),
                "placeholder message should name the unknown id; got: {msg:?}"
            );
        }
    }

    // ── SetupOverlayState::clamp_selection ────────────────────────────────────

    fn make_overlay(count: usize) -> SetupOverlayState {
        let items = (0..count)
            .map(|i| SetupItem {
                id: format!("item-{i}"),
                label: format!("Item {i}"),
                description: String::new(),
                action: SetupItemAction::Placeholder(String::new()),
            })
            .collect();
        SetupOverlayState::new(items, "provider".into(), "ready".into())
    }

    #[test]
    fn clamp_selection_keeps_valid_index_unchanged() {
        let mut overlay = make_overlay(5);
        overlay.selected_index = 3;
        overlay.clamp_selection();
        assert_eq!(overlay.selected_index, 3);
    }

    #[test]
    fn clamp_selection_caps_index_at_last_item() {
        let mut overlay = make_overlay(3);
        overlay.selected_index = 10; // out of range
        overlay.clamp_selection();
        assert_eq!(
            overlay.selected_index, 2,
            "index must be clamped to items.len() - 1"
        );
    }

    #[test]
    fn clamp_selection_resets_to_zero_when_items_empty() {
        let mut overlay = make_overlay(0);
        overlay.selected_index = 5;
        overlay.clamp_selection();
        assert_eq!(overlay.selected_index, 0, "empty items → index must be 0");
    }

    // ── ProviderFormState::selected_display_name ──────────────────────────────

    #[test]
    fn selected_display_name_returns_first_option_by_default() {
        let form = ProviderFormState::new(
            ProviderFormKind::ApiKey,
            vec![
                ProviderFormOption {
                    provider_id: "openai".into(),
                    display_name: "OpenAI".into(),
                },
                ProviderFormOption {
                    provider_id: "anthropic".into(),
                    display_name: "Anthropic".into(),
                },
            ],
        );
        assert_eq!(form.selected_display_name(), "OpenAI");
    }

    #[test]
    fn selected_display_name_falls_back_when_options_empty() {
        // No options → must return the sentinel "(none)" rather than panicking.
        let form = ProviderFormState::new(ProviderFormKind::ApiKey, vec![]);
        assert_eq!(form.selected_display_name(), "(none)");
    }

    // ── ProviderFormState::display_value ─────────────────────────────────────

    #[test]
    fn display_value_masks_api_key_as_bullets() {
        // The raw key must never appear; each character is replaced with '•'.
        let mut form = ProviderFormState::new(ProviderFormKind::ApiKey, vec![]);
        form.input = TextBuffer::from_text("sk-abc123", false);

        let displayed = form.display_value();
        assert!(
            !displayed.contains("sk-abc123"),
            "raw API key must not appear in display_value; got: {displayed:?}"
        );
        // Every character → one bullet; length leaks no more than character count.
        assert_eq!(
            displayed.chars().count(),
            "sk-abc123".chars().count(),
            "bullet count must equal key character count"
        );
        assert!(
            displayed.chars().all(|c| c == '\u{2022}'),
            "every character must be a bullet; got: {displayed:?}"
        );
    }

    #[test]
    fn display_value_shows_api_base_url_in_plain_text() {
        let mut form = ProviderFormState::new(ProviderFormKind::ApiBase, vec![]);
        let url = "https://my-proxy.example.com/v1";
        form.input = TextBuffer::from_text(url, false);

        let displayed = form.display_value();
        assert_eq!(
            displayed, url,
            "API-base URL must appear verbatim in display_value"
        );
    }

    #[test]
    fn display_value_empty_input_returns_empty_string() {
        let form_key = ProviderFormState::new(ProviderFormKind::ApiKey, vec![]);
        assert_eq!(form_key.display_value(), "");

        let form_base = ProviderFormState::new(ProviderFormKind::ApiBase, vec![]);
        assert_eq!(form_base.display_value(), "");
    }

    // ── CopilotOAuthFlowState Debug redaction ─────────────────────────────────

    #[test]
    fn copilot_oauth_debug_redacts_device_code_secret() {
        use wonder_of_u_agent::CopilotDeviceCode;

        let state = CopilotOAuthFlowState::AwaitingConfirmation {
            device_code: CopilotDeviceCode {
                device_code: "do-not-log-this-secret".into(),
                user_code: "SAFE-CODE".into(),
                verification_uri: "https://github.com/login/device".into(),
                expires_in: 900,
                interval: 5,
            },
        };
        let debug_output = format!("{state:?}");

        // The ephemeral device_code secret must never appear in debug output.
        assert!(
            !debug_output.contains("do-not-log-this-secret"),
            "device_code secret must be redacted in Debug output; got: {debug_output:?}"
        );
        // The user-visible code and URI are safe to show.
        assert!(
            debug_output.contains("SAFE-CODE"),
            "user_code should appear in debug output; got: {debug_output:?}"
        );
    }
}
