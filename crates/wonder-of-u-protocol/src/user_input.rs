//! User-input items: the payload of `Op::UserInput`.
//!
//! `UserInput` mirrors codex's `user_input::UserInput` — a sum type over the
//! shapes a user can send in a single turn. Phase 0 keeps just the essentials:
//! text, image (referenced by path), and the local-image placeholder used by
//! the `view_image` tool flow.

use serde::{Deserialize, Serialize};

/// A single user-input item inside an `Op::UserInput`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserInput {
    /// Free-form text from the user (typed or pasted).
    Text {
        /// The text content.
        text: String,
    },
    /// Local image referenced by filesystem path; the agent decides how to
    /// inline it for the model.
    LocalImage {
        /// Absolute path to the image on disk.
        path: String,
    },
    /// Image referenced by URL.
    RemoteImage {
        /// The image URL.
        url: String,
    },
    /// Mentions: zero or more `@path` tokens resolved by the TUI's
    /// file-mention popup. Each entry expands to a tool-call side-channel
    /// before the message reaches the model.
    Mention {
        /// `@path` token (the leading `@` is stripped before reaching here).
        path: String,
    },
    /// Slash command invocation. The TUI routes the named command through
    /// `wonder-of-u-cli` (Phase 4) before sending anything to the model.
    SlashCommand {
        /// Slash command name (without the leading `/`).
        name: String,
        /// Argument string after the slash command name (may be empty).
        args: String,
    },
}

/// Convenience alias used by `Op::UserInput::items`.
///
/// Phase 1 may split `UserInput` and `UserInputItem` (the former is what the
/// TUI emits, the latter is what the model sees post-resolution). For now
/// they are the same type.
pub type UserInputItem = UserInput;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip() {
        let input = UserInput::Text {
            text: "hello".to_string(),
        };
        let json = serde_json::to_string(&input).unwrap();
        let parsed: UserInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }

    #[test]
    fn slash_command_roundtrip() {
        let input = UserInput::SlashCommand {
            name: "compact".to_string(),
            args: String::new(),
        };
        let json = serde_json::to_string(&input).unwrap();
        let parsed: UserInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }

    #[test]
    fn local_image_roundtrip() {
        let input = UserInput::LocalImage {
            path: "/tmp/x.png".to_string(),
        };
        let json = serde_json::to_string(&input).unwrap();
        let parsed: UserInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }
}
