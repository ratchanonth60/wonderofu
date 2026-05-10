//! Wire-protocol variants that identify the HTTP API shape used when talking
//! to a provider endpoint.
//!
//! Each variant maps to a distinct request/response envelope format.  The
//! runtime dispatch layer reads this tag from [`crate::ProviderDescriptor`] so
//! that new providers can be wired up without touching the core runtime.

use serde::{Deserialize, Serialize};

/// The HTTP wire-protocol shape a provider endpoint expects.
///
/// Variants map 1-to-1 to the families of API specs in the wild.  The
/// runtime uses this to choose which serialiser/deserialiser path to follow
/// when sending requests and parsing responses.
///
/// # Examples
///
/// ```
/// use wonder_of_u_agent::WireProtocol;
///
/// let proto = WireProtocol::OpenAiCompat;
/// assert_eq!(proto, WireProtocol::OpenAiCompat);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WireProtocol {
    /// OpenAI-compatible chat-completions endpoint shape.
    ///
    /// This is the most widely adopted third-party API format.  It is the
    /// fallback default when no explicit protocol is declared.
    #[default]
    OpenAiCompat,

    /// Native Anthropic Messages API (`/v1/messages`).
    AnthropicCompat,

    /// GitHub Copilot token-authenticated variant of the OpenAI-compat API.
    ///
    /// Differs from plain `OpenAiCompat` only in the authentication header
    /// shape (`Authorization: Bearer <github-oauth-token>`).
    Copilot,

    /// Anthropic models served through Amazon Bedrock.
    ///
    /// Uses SigV4 request signing and wraps the Anthropic Messages payload
    /// inside the Bedrock `InvokeModel` envelope.
    BedrockAnthropic,

    /// Google Gemini native REST API (`generativelanguage.googleapis.com`).
    GeminiNative,

    /// Gemini models served through Google Vertex AI
    /// (`aiplatform.googleapis.com`).
    VertexGemini,

    /// Azure OpenAI Service.
    ///
    /// OpenAI-compat payload, but the endpoint URL includes the Azure
    /// deployment name and the auth header is `api-key` rather than
    /// `Authorization: Bearer`.
    AzureOpenAi,

    /// Synthetic / stub protocol used in tests and offline scenarios.
    ///
    /// The runtime short-circuits immediately for this variant and never
    /// makes a real HTTP call.
    Synthetic,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_openai_compat() {
        assert_eq!(WireProtocol::default(), WireProtocol::OpenAiCompat);
    }

    #[test]
    fn clone_copy_and_eq_work() {
        let a = WireProtocol::AnthropicCompat;
        let b = a; // Copy
        assert_eq!(a, b);
        assert_eq!(a.clone(), b);
    }

    #[test]
    fn serde_round_trip_snake_case() {
        // Verify that each variant serialises to the expected snake_case key
        // and round-trips through JSON without loss.
        let cases: &[(&str, WireProtocol)] = &[
            ("\"open_ai_compat\"", WireProtocol::OpenAiCompat),
            ("\"anthropic_compat\"", WireProtocol::AnthropicCompat),
            ("\"copilot\"", WireProtocol::Copilot),
            ("\"bedrock_anthropic\"", WireProtocol::BedrockAnthropic),
            ("\"gemini_native\"", WireProtocol::GeminiNative),
            ("\"vertex_gemini\"", WireProtocol::VertexGemini),
            ("\"azure_open_ai\"", WireProtocol::AzureOpenAi),
            ("\"synthetic\"", WireProtocol::Synthetic),
        ];

        for (expected_json, variant) in cases {
            let serialised = serde_json::to_string(variant)
                .unwrap_or_else(|err| panic!("serialise {variant:?}: {err}"));
            assert_eq!(&serialised, expected_json, "serialised form of {variant:?}");

            let deserialised: WireProtocol = serde_json::from_str(&serialised)
                .unwrap_or_else(|err| panic!("deserialise {variant:?}: {err}"));
            assert_eq!(&deserialised, variant, "round-trip of {variant:?}");
        }
    }
}
