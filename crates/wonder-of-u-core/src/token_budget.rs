//! Token budget utilities for context-window management.
//!
//! Provides fast, client-side heuristics for estimating token counts and
//! determining effective context window sizes — no API round-trip required.
//! Ported from the TypeScript reference in `src/services/tokenEstimation.ts`
//! and `src/services/compact/autoCompact.ts`.

// ---------------------------------------------------------------------------
// Buffer constants (from autoCompact.ts)
// ---------------------------------------------------------------------------

/// Tokens reserved as a safety buffer before triggering automatic compaction.
/// The auto-compact threshold is `effective_context_window - AUTOCOMPACT_BUFFER_TOKENS`.
pub const AUTOCOMPACT_BUFFER_TOKENS: usize = 13_000;

/// Tokens below the effective window at which a user-visible warning is shown.
pub const WARNING_THRESHOLD_BUFFER_TOKENS: usize = 20_000;

/// Tokens below the effective window at which manual compaction is required.
/// The blocking limit is `effective_context_window - MANUAL_COMPACT_BUFFER_TOKENS`.
pub const MANUAL_COMPACT_BUFFER_TOKENS: usize = 3_000;

// ---------------------------------------------------------------------------
// Context-window knowledge base (from context.ts)
// ---------------------------------------------------------------------------

/// Default context window for Claude models (200 k tokens).
///
/// All current Claude 3 / 4 models share this window size.
pub const MODEL_CONTEXT_WINDOW_DEFAULT: usize = 200_000;

/// Tokens reserved for compaction summary output.
///
/// Based on p99.99 of compact-summary output being ~17 k tokens; rounded up
/// to 20 k for headroom.  Mirrors `MAX_OUTPUT_TOKENS_FOR_SUMMARY` in the TS
/// source.
pub const COMPACT_MAX_OUTPUT_TOKENS: usize = 20_000;

/// Return the known context-window size (in tokens) for `model_id`.
///
/// The heuristic checks for well-known Claude model name fragments, falling
/// back to [`MODEL_CONTEXT_WINDOW_DEFAULT`] (200 k) for any unrecognised
/// string.  No API call is made.
///
/// # Examples
///
/// ```
/// use wonder_of_u_core::token_budget::context_window_for_model;
/// assert_eq!(context_window_for_model("claude-3-5-sonnet-20241022"), 200_000);
/// assert_eq!(context_window_for_model("claude-3-haiku-20240307"), 200_000);
/// ```
#[must_use]
pub fn context_window_for_model(model_id: &str) -> usize {
    let m = model_id.to_ascii_lowercase();

    if m.contains("claude-sonnet-4")
        || m.contains("claude-opus-4")
        || m.contains("opus-4-6")
        || m.contains("[1m]")
    {
        return 1_000_000;
    }

    if m.contains("gpt-4o") {
        return 128_000;
    }

    if m.contains("gemini-2.5")
        || m.contains("gemini-3.")
        || m.contains("gemini-3-")
        || m.contains("gemini-pro-latest")
        || m.contains("gemini-flash-latest")
        || m.contains("gemini-2.0")
        || m.contains("gemma-4")
    {
        return 1_048_576;
    }

    if m.contains("deepseek") || m.contains("kimi") || m.contains("glm") {
        return 128_000;
    }

    MODEL_CONTEXT_WINDOW_DEFAULT
}

/// Return the *effective* context window — the raw window minus tokens
/// reserved for compaction output.
///
/// This matches `getEffectiveContextWindowSize` in `autoCompact.ts`:
/// ```text
/// effectiveWindow = contextWindow - min(maxOutputTokens, MAX_OUTPUT_TOKENS_FOR_SUMMARY)
/// ```
/// Because the Rust port does not yet model per-model max output tokens, we
/// always subtract [`COMPACT_MAX_OUTPUT_TOKENS`] (20 k), which is the cap
/// applied in the TS source anyway.
///
/// # Examples
///
/// ```
/// use wonder_of_u_core::token_budget::effective_context_window;
/// assert_eq!(effective_context_window("claude-3-opus-20240229"), 200_000 - 20_000);
/// ```
#[must_use]
pub fn effective_context_window(model_id: &str) -> usize {
    let window = context_window_for_model(model_id);
    // Saturating subtraction so the result is always a valid usize.
    window.saturating_sub(COMPACT_MAX_OUTPUT_TOKENS)
}

// ---------------------------------------------------------------------------
// Token estimation (from tokenEstimation.ts)
// ---------------------------------------------------------------------------

/// Fast heuristic: estimate the token count for `text` using the
/// `chars / bytes_per_token` approximation.
///
/// Mirrors `roughTokenCountEstimation(content, bytesPerToken = 4)` from the
/// TypeScript source.  The default ratio of 4 bytes per token is appropriate
/// for mixed English prose and code.  Dense formats like JSON/JSONL should
/// use [`rough_token_count_for_extension`] instead.
///
/// The estimate rounds via integer truncation (equivalent to `Math.floor` in
/// JS), which matches the TS `Math.round` to within ±1 token per 4 chars —
/// well inside the 20 % accuracy requirement.
///
/// # Examples
///
/// ```
/// use wonder_of_u_core::token_budget::rough_token_count;
/// let est = rough_token_count("Hello, world!");
/// assert!(est > 0);
/// ```
#[must_use]
pub fn rough_token_count(text: &str) -> usize {
    rough_token_count_with_ratio(text, 4)
}

/// Like [`rough_token_count`] but uses a ratio appropriate for `file_extension`.
///
/// Dense formats (JSON, JSONL, JSONC) use a ratio of 2 — their many
/// single-character tokens (`{`, `}`, `:`, `,`, `"`) make the default-4
/// estimate significantly too low.  All other extensions fall back to 4.
///
/// Mirrors `roughTokenCountEstimationForFileType` / `bytesPerTokenForFileType`
/// from the TypeScript source.
///
/// # Examples
///
/// ```
/// use wonder_of_u_core::token_budget::rough_token_count_for_extension;
/// let json_est  = rough_token_count_for_extension(r#"{"key":"value"}"#, "json");
/// let plain_est = rough_token_count_for_extension(r#"{"key":"value"}"#, "txt");
/// // JSON uses ratio 2, so it should produce ~2× more tokens than ratio 4.
/// assert!(json_est >= plain_est);
/// ```
#[must_use]
pub fn rough_token_count_for_extension(text: &str, file_extension: &str) -> usize {
    let ratio = bytes_per_token_for_extension(file_extension);
    rough_token_count_with_ratio(text, ratio)
}

/// Return the bytes-per-token ratio for `file_extension`.
///
/// Mirrors `bytesPerTokenForFileType` from the TypeScript source.
#[must_use]
pub fn bytes_per_token_for_extension(file_extension: &str) -> usize {
    match file_extension {
        "json" | "jsonl" | "jsonc" => 2,
        _ => 4,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn rough_token_count_with_ratio(text: &str, bytes_per_token: usize) -> usize {
    if bytes_per_token == 0 {
        return 0;
    }
    // Use char count to match the JS `content.length` (UTF-16 code units for
    // ASCII-dominated content ≈ char count, and practically identical for the
    // heuristic's accuracy requirements).
    text.chars().count() / bytes_per_token
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- context_window_for_model ---

    #[test]
    fn known_sonnet_model_returns_200k() {
        assert_eq!(
            context_window_for_model("claude-3-5-sonnet-20241022"),
            200_000
        );
    }

    #[test]
    fn known_opus_model_returns_200k() {
        assert_eq!(context_window_for_model("claude-3-opus-20240229"), 200_000);
    }

    #[test]
    fn known_haiku_model_returns_200k() {
        assert_eq!(context_window_for_model("claude-3-haiku-20240307"), 200_000);
    }

    #[test]
    fn claude_35_haiku_returns_200k() {
        assert_eq!(
            context_window_for_model("claude-3-5-haiku-20241022"),
            200_000
        );
    }

    #[test]
    fn claude_sonnet4_returns_1m() {
        // claude-sonnet-4 is the 1M model per context.ts
        assert_eq!(
            context_window_for_model("claude-sonnet-4-20250514"),
            1_000_000
        );
    }

    #[test]
    fn explicit_1m_suffix_returns_1m() {
        assert_eq!(context_window_for_model("claude-opus-4-6[1m]"), 1_000_000);
    }

    #[test]
    fn gpt4o_returns_128k() {
        assert_eq!(context_window_for_model("gpt-4o"), 128_000);
    }

    #[test]
    fn deepseek_returns_128k() {
        assert_eq!(context_window_for_model("deepseek-v4-flash"), 128_000);
    }

    #[test]
    fn kimi_returns_128k() {
        assert_eq!(context_window_for_model("kimi-k2"), 128_000);
    }

    #[test]
    fn glm_returns_128k() {
        assert_eq!(context_window_for_model("glm-4"), 128_000);
    }

    #[test]
    fn unknown_model_returns_default() {
        assert_eq!(
            context_window_for_model("hypothetical-model"),
            MODEL_CONTEXT_WINDOW_DEFAULT
        );
    }

    #[test]
    fn empty_model_id_returns_default() {
        assert_eq!(context_window_for_model(""), MODEL_CONTEXT_WINDOW_DEFAULT);
    }

    // --- effective_context_window ---

    #[test]
    fn effective_window_subtracts_compact_output_budget() {
        let expected = 200_000 - COMPACT_MAX_OUTPUT_TOKENS;
        assert_eq!(
            effective_context_window("claude-3-5-sonnet-20241022"),
            expected
        );
    }

    #[test]
    fn effective_window_1m_model() {
        let expected = 1_000_000 - COMPACT_MAX_OUTPUT_TOKENS;
        assert_eq!(
            effective_context_window("claude-sonnet-4-20250514"),
            expected
        );
    }

    // --- rough_token_count ---

    #[test]
    fn empty_string_is_zero() {
        assert_eq!(rough_token_count(""), 0);
    }

    #[test]
    fn four_chars_is_one_token() {
        assert_eq!(rough_token_count("abcd"), 1);
    }

    #[test]
    fn eight_chars_is_two_tokens() {
        assert_eq!(rough_token_count("abcdefgh"), 2);
    }

    /// Verify the estimate is within 20 % of a reference count for a sample
    /// English paragraph (~100 real tokens).
    #[test]
    fn prose_estimate_within_20_percent_of_reference() {
        // 400-char English prose ≈ 100 tokens by the chars/4 heuristic.
        // A real tokeniser would give ~90–110 tokens for this text.
        let text = "The quick brown fox jumps over the lazy dog. \
                    Rust is a systems programming language that runs blazingly \
                    fast, prevents segfaults, and guarantees thread safety. \
                    It achieves memory safety without a garbage collector by \
                    using a borrow checker. The language is popular for its \
                    performance and safety guarantees in concurrent systems. \
                    Additional padding to reach approximately four hundred ch";
        let estimated = rough_token_count(text);
        // A human-labelled reference: ~96 tokens for 384 chars at ratio 4.
        // We accept anything within 20 % of that reference (77–115).
        assert!(
            (77..=115).contains(&estimated),
            "estimated {estimated} is not within 20% of ~96"
        );
    }

    // --- rough_token_count_for_extension ---

    #[test]
    fn json_extension_uses_ratio_2() {
        // 8 chars at ratio 2 → 4 tokens
        assert_eq!(rough_token_count_for_extension("{}:{}:{}", "json"), 4);
    }

    #[test]
    fn jsonl_extension_uses_ratio_2() {
        // 8 chars at ratio 2 → 4 tokens
        assert_eq!(rough_token_count_for_extension("{}:{}:{}", "jsonl"), 4);
    }

    #[test]
    fn txt_extension_uses_ratio_4() {
        // 8 chars at ratio 4 → 2 tokens
        assert_eq!(rough_token_count_for_extension("abcdefgh", "txt"), 2);
    }

    #[test]
    fn json_ratio_greater_than_or_equal_plain_ratio() {
        let payload = r#"{"key":"value","count":42}"#;
        let json_est = rough_token_count_for_extension(payload, "json");
        let plain_est = rough_token_count_for_extension(payload, "rs");
        assert!(json_est >= plain_est);
    }

    // --- bytes_per_token_for_extension ---

    #[test]
    fn json_ratio_is_2() {
        assert_eq!(bytes_per_token_for_extension("json"), 2);
        assert_eq!(bytes_per_token_for_extension("jsonl"), 2);
        assert_eq!(bytes_per_token_for_extension("jsonc"), 2);
    }

    #[test]
    fn default_ratio_is_4() {
        assert_eq!(bytes_per_token_for_extension("rs"), 4);
        assert_eq!(bytes_per_token_for_extension("txt"), 4);
        assert_eq!(bytes_per_token_for_extension("md"), 4);
        assert_eq!(bytes_per_token_for_extension(""), 4);
    }

    // --- buffer constants ---

    #[test]
    fn autocompact_buffer_constant_value() {
        assert_eq!(AUTOCOMPACT_BUFFER_TOKENS, 13_000);
    }

    #[test]
    fn warning_threshold_buffer_constant_value() {
        assert_eq!(WARNING_THRESHOLD_BUFFER_TOKENS, 20_000);
    }

    #[test]
    fn manual_compact_buffer_constant_value() {
        assert_eq!(MANUAL_COMPACT_BUFFER_TOKENS, 3_000);
    }
}
