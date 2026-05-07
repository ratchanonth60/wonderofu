//! Pure helpers for tool-surface orchestration.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use wonder_of_u_core::{
    FeatureSet, PermissionDecision, PermissionDecisionReason, PermissionRequest, ToolContext,
    ToolProgress, ToolRegistry, ToolSpec,
};

/// Default per-tool inline result budget before a runtime should persist output.
pub const DEFAULT_MAX_RESULT_SIZE_CHARS: usize = 50_000;
/// Global token cap used to estimate oversized tool results.
pub const MAX_TOOL_RESULT_TOKENS: usize = 100_000;
/// Conservative byte-per-token estimate used for result-size heuristics.
pub const BYTES_PER_TOKEN: usize = 4;
/// Global byte cap derived from the token budget.
pub const MAX_TOOL_RESULT_BYTES: usize = MAX_TOOL_RESULT_TOKENS * BYTES_PER_TOKEN;
/// Aggregate inline budget for one batch of tool results.
pub const MAX_TOOL_RESULTS_PER_MESSAGE_CHARS: usize = 200_000;
/// Maximum display length for compact tool summaries.
pub const TOOL_SUMMARY_MAX_LENGTH: usize = 50;
/// Default max parallelism for tools marked as concurrency-safe.
pub const DEFAULT_MAX_CONCURRENT_TOOL_USES: usize = 10;
/// XML tag used by persisted-output notices.
pub const PERSISTED_OUTPUT_TAG: &str = "<persisted-output>";
/// Closing XML tag used by persisted-output notices.
pub const PERSISTED_OUTPUT_CLOSING_TAG: &str = "</persisted-output>";
/// Placeholder emitted when old persisted tool output was cleared.
pub const TOOL_RESULT_CLEARED_MESSAGE: &str = "[Old tool result content cleared]";
/// Default preview size for persisted tool-result notices.
pub const PREVIEW_SIZE_BYTES: usize = 2_000;

/// Merges tool specs while preserving the first visible definition for any name or alias.
#[must_use]
pub fn merge_tool_specs(
    preferred: impl IntoIterator<Item = ToolSpec>,
    fallback: impl IntoIterator<Item = ToolSpec>,
) -> Vec<ToolSpec> {
    let mut seen = BTreeSet::new();
    let mut merged = Vec::new();

    for spec in preferred.into_iter().chain(fallback) {
        let keys = tool_keys(&spec);
        if keys.iter().any(|key| seen.contains(key)) {
            continue;
        }
        seen.extend(keys);
        merged.push(spec);
    }

    merged
}

/// Filters tool specs for a concrete runtime context without reordering them.
#[must_use]
pub fn filter_tool_specs(
    specs: impl IntoIterator<Item = ToolSpec>,
    features: &FeatureSet,
    allowed_tools: Option<&BTreeSet<String>>,
    context: Option<&ToolContext>,
) -> Vec<ToolSpec> {
    specs
        .into_iter()
        .filter(|spec| spec.is_enabled(features))
        .filter(|spec| tool_is_allowed(spec, allowed_tools))
        .filter(|spec| !context.is_some_and(|context| statically_denied_by_rule(spec, context)))
        .collect()
}

/// Returns the tool specs visible to the provider loop for a context.
#[must_use]
pub fn provider_tool_specs(
    registry: &ToolRegistry,
    context: &ToolContext,
    allowed_tools: Option<&BTreeSet<String>>,
) -> Vec<ToolSpec> {
    filter_tool_specs(
        registry.all_specs(),
        &context.features,
        allowed_tools,
        Some(context),
    )
}

/// Returns whether a tool survives allowed-tool filtering.
#[must_use]
pub fn tool_is_allowed(spec: &ToolSpec, allowed_tools: Option<&BTreeSet<String>>) -> bool {
    let Some(allowed_tools) = allowed_tools else {
        return true;
    };

    let allowed = allowed_tools
        .iter()
        .filter_map(|name| normalize_tool_name(name))
        .collect::<BTreeSet<_>>();

    tool_keys(spec)
        .into_iter()
        .any(|key| allowed.contains(&key))
}

/// Returns true when a rule-only permission decision hides the tool from discovery.
#[must_use]
pub fn statically_denied_by_rule(spec: &ToolSpec, context: &ToolContext) -> bool {
    let request = PermissionRequest::new(spec.name.clone())
        .with_aliases(spec.aliases.clone())
        .read_only(spec.read_only)
        .destructive(spec.destructive);

    matches!(
        context.permission_context().evaluate(&request),
        PermissionDecision::Deny {
            reason: PermissionDecisionReason::Rule { .. }
        }
    )
}

/// Coarse concurrency class used by orchestration surfaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolConcurrencyClass {
    /// Represents exclusive
    Exclusive,
    /// Represents parallel safe
    ParallelSafe,
}

/// Static concurrency metadata derived from a tool spec.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolConcurrencyMetadata {
    /// Stores the class
    pub class: ToolConcurrencyClass,
    /// Stores the max parallelism
    pub max_parallelism: usize,
}

impl ToolConcurrencyMetadata {
    /// Handles from spec
    #[must_use]
    pub fn from_spec(spec: &ToolSpec) -> Self {
        if spec.concurrency_safe {
            Self {
                class: ToolConcurrencyClass::ParallelSafe,
                max_parallelism: DEFAULT_MAX_CONCURRENT_TOOL_USES,
            }
        } else {
            Self {
                class: ToolConcurrencyClass::Exclusive,
                max_parallelism: 1,
            }
        }
    }
}

/// Runtime-wide tool result limits surfaced to orchestration code.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolRuntimeLimits {
    /// Stores the max result size chars
    pub max_result_size_chars: usize,
    /// Stores the max result tokens
    pub max_result_tokens: usize,
    /// Stores the max result bytes
    pub max_result_bytes: usize,
    /// Stores the max results per message chars
    pub max_results_per_message_chars: usize,
}

impl Default for ToolRuntimeLimits {
    fn default() -> Self {
        Self {
            max_result_size_chars: DEFAULT_MAX_RESULT_SIZE_CHARS,
            max_result_tokens: MAX_TOOL_RESULT_TOKENS,
            max_result_bytes: MAX_TOOL_RESULT_BYTES,
            max_results_per_message_chars: MAX_TOOL_RESULTS_PER_MESSAGE_CHARS,
        }
    }
}

/// High-level lifecycle state for a single tool call.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionState {
    /// Represents queued
    Queued,
    /// Represents executing
    Executing,
    /// Represents completed
    Completed,
    /// Represents yielded
    Yielded,
    /// Represents cancelled
    Cancelled,
}

/// Reason a running tool call stopped before normal completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCancellationReason {
    /// Represents user interrupted
    UserInterrupted,
    /// Represents sibling error
    SiblingError,
    /// Represents streaming fallback
    StreamingFallback,
}

/// Mutable progress snapshot suitable for orchestration UIs and transcript summaries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolProgressState {
    /// Stores the state
    pub state: ToolExecutionState,
    /// Stores the message
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Stores the percent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f32>,
    /// Stores the cancellation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<ToolCancellationReason>,
}

impl ToolProgressState {
    /// Handles queued
    #[must_use]
    pub fn queued() -> Self {
        Self {
            state: ToolExecutionState::Queued,
            ..Self::default()
        }
    }

    /// Handles apply progress
    pub fn apply_progress(&mut self, progress: &ToolProgress) {
        self.state = ToolExecutionState::Executing;
        self.message = Some(progress.message.clone());
        self.percent = progress.percent.map(|percent| percent.clamp(0.0, 100.0));
        self.cancellation = None;
    }

    /// Handles mark completed
    pub fn mark_completed(&mut self) {
        self.state = ToolExecutionState::Completed;
        self.percent = Some(100.0);
        self.cancellation = None;
    }

    /// Handles mark yielded
    pub fn mark_yielded(&mut self) {
        self.state = ToolExecutionState::Yielded;
        self.cancellation = None;
    }

    /// Handles cancel
    pub fn cancel(&mut self, reason: ToolCancellationReason) {
        self.state = ToolExecutionState::Cancelled;
        self.cancellation = Some(reason);
    }
}

impl Default for ToolProgressState {
    fn default() -> Self {
        Self {
            state: ToolExecutionState::Queued,
            message: None,
            percent: None,
            cancellation: None,
        }
    }
}

/// Summary of a tool result whose full output was persisted elsewhere.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistedToolResultSummary {
    /// Stores the filepath
    pub filepath: String,
    /// Stores the original size bytes
    pub original_size_bytes: usize,
    /// Stores whether json
    pub is_json: bool,
    /// Stores the preview
    pub preview: String,
    /// Stores whether more
    pub has_more: bool,
}

impl PersistedToolResultSummary {
    /// Handles from content
    #[must_use]
    pub fn from_content(
        filepath: impl Into<String>,
        content: &str,
        is_json: bool,
        preview_size_bytes: usize,
    ) -> Self {
        let (preview, has_more) = preview_text(content, preview_size_bytes);
        Self {
            filepath: filepath.into(),
            original_size_bytes: content.len(),
            is_json,
            preview,
            has_more,
        }
    }
    /// Handles format notice
    #[must_use]
    pub fn format_notice(&self, preview_size_bytes: usize) -> String {
        let mut message = String::new();
        message.push_str(PERSISTED_OUTPUT_TAG);
        message.push('\n');
        message.push_str(&format!(
            "Output too large ({}). Full output saved to: {}\n\n",
            format_bytes(self.original_size_bytes),
            self.filepath
        ));
        message.push_str(&format!(
            "Preview (first {}):\n",
            format_bytes(preview_size_bytes)
        ));
        message.push_str(&self.preview);
        if self.has_more {
            message.push_str("\n...\n");
        } else {
            message.push('\n');
        }
        message.push_str(PERSISTED_OUTPUT_CLOSING_TAG);
        message
    }
}

fn tool_keys(spec: &ToolSpec) -> BTreeSet<String> {
    std::iter::once(&spec.name)
        .chain(spec.aliases.iter())
        .filter_map(|name| normalize_tool_name(name))
        .collect()
}

fn normalize_tool_name(name: &str) -> Option<String> {
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn preview_text(content: &str, limit_bytes: usize) -> (String, bool) {
    if content.len() <= limit_bytes {
        return (content.to_string(), false);
    }

    let mut end = 0usize;
    for (index, ch) in content.char_indices() {
        let next = index + ch.len_utf8();
        if next > limit_bytes {
            break;
        }
        end = next;
    }

    if end == 0 {
        return (String::new(), !content.is_empty());
    }

    (content[..end].to_string(), true)
}

fn format_bytes(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;

    let bytes = bytes as f64;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{} B", bytes as usize)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use wonder_of_u_core::{
        FeatureFlag, PermissionMode, PermissionRule, PermissionRuleBehavior, PermissionRuleSource,
        SessionId, ToolKind, ToolUseId,
    };

    use super::*;

    fn spec(name: &str) -> ToolSpec {
        ToolSpec::new(name, format!("{name} description"), ToolKind::Search)
    }

    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new(),
            cwd: PathBuf::from("/workspace"),
            permission_mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            permission_rules: Vec::new(),
            features: FeatureSet::first_release(),
            bash_session_store: None,
        }
    }

    #[test]
    fn merge_prefers_first_seen_name_or_alias() {
        let mut file_read = spec("file_read");
        file_read.aliases.push("Read".into());
        let mut read_alias_collision = spec("alternate_read");
        read_alias_collision.aliases.push("read".into());
        let file_write = spec("file_write");

        let merged = merge_tool_specs(
            vec![file_read.clone()],
            vec![read_alias_collision, file_write.clone()],
        );

        assert_eq!(
            merged
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            vec!["file_read", "file_write"]
        );
        assert_eq!(merged[0], file_read);
        assert_eq!(merged[1], file_write);
    }

    #[test]
    fn filter_tool_specs_applies_feature_flags_and_allowed_names() {
        let mut file_read = spec("file_read");
        file_read.aliases.push("Read".into());
        let mut remote_trigger = spec("remote_trigger");
        remote_trigger
            .required_features
            .insert(FeatureFlag::RemoteTriggers);
        remote_trigger.aliases.push("RemoteTrigger".into());

        let allowed = BTreeSet::from(["read".to_string(), "remote_trigger".to_string()]);
        let filtered = filter_tool_specs(
            vec![file_read.clone(), remote_trigger.clone()],
            &FeatureSet::first_release(),
            Some(&allowed),
            None,
        );

        assert_eq!(filtered, vec![file_read.clone()]);

        let mut features = FeatureSet::first_release();
        features.enable(FeatureFlag::RemoteTriggers);
        let filtered = filter_tool_specs(
            vec![file_read.clone(), remote_trigger.clone()],
            &features,
            Some(&allowed),
            None,
        );

        assert_eq!(filtered, vec![file_read, remote_trigger]);
    }

    #[test]
    fn filter_tool_specs_hides_rule_denied_tools_only() {
        let mut ctx = context();
        ctx.permission_rules.push(PermissionRule::new(
            "read",
            PermissionRuleBehavior::Deny,
            PermissionRuleSource::CliArg,
        ));
        ctx.permission_rules.push(PermissionRule::new(
            "web_fetch",
            PermissionRuleBehavior::Ask,
            PermissionRuleSource::CliArg,
        ));

        let mut file_read = spec("file_read");
        file_read.aliases.push("Read".into());
        let web_fetch = spec("web_fetch");

        let filtered = filter_tool_specs(
            vec![file_read, web_fetch.clone()],
            &ctx.features,
            None,
            Some(&ctx),
        );

        assert_eq!(filtered, vec![web_fetch]);
    }

    #[test]
    fn provider_tool_specs_preserve_registry_order() {
        let mut registry = ToolRegistry::new();

        struct StaticTool(ToolSpec);

        #[async_trait::async_trait]
        impl wonder_of_u_core::Tool for StaticTool {
            fn spec(&self) -> ToolSpec {
                self.0.clone()
            }

            async fn execute(
                &self,
                _context: ToolContext,
                use_id: ToolUseId,
                _input: serde_json::Value,
            ) -> wonder_of_u_core::Result<wonder_of_u_core::ToolResult> {
                Ok(wonder_of_u_core::ToolResult::success(use_id, "ok"))
            }
        }

        let mut file_read = spec("file_read");
        file_read.aliases.push("Read".into());
        let file_write = spec("file_write");
        registry
            .register(std::sync::Arc::new(StaticTool(file_read)))
            .expect("register file_read");
        registry
            .register(std::sync::Arc::new(StaticTool(file_write)))
            .expect("register file_write");

        let allowed = BTreeSet::from([
            "write".to_string(),
            "read".to_string(),
            "file_write".to_string(),
        ]);
        let specs = provider_tool_specs(&registry, &context(), Some(&allowed));

        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            vec!["file_read", "file_write"]
        );
    }

    #[test]
    fn concurrency_metadata_reflects_spec_safety() {
        let read = ToolSpec {
            concurrency_safe: true,
            ..spec("file_read")
        };
        let write = ToolSpec {
            concurrency_safe: false,
            ..spec("file_write")
        };

        assert_eq!(
            ToolConcurrencyMetadata::from_spec(&read),
            ToolConcurrencyMetadata {
                class: ToolConcurrencyClass::ParallelSafe,
                max_parallelism: DEFAULT_MAX_CONCURRENT_TOOL_USES,
            }
        );
        assert_eq!(
            ToolConcurrencyMetadata::from_spec(&write),
            ToolConcurrencyMetadata {
                class: ToolConcurrencyClass::Exclusive,
                max_parallelism: 1,
            }
        );
    }

    #[test]
    fn progress_state_tracks_updates_and_cancellation() {
        let use_id = ToolUseId::new();
        let mut state = ToolProgressState::queued();
        state.apply_progress(&ToolProgress {
            use_id,
            message: "working".into(),
            percent: Some(150.0),
        });

        assert_eq!(state.state, ToolExecutionState::Executing);
        assert_eq!(state.message.as_deref(), Some("working"));
        assert_eq!(state.percent, Some(100.0));

        state.cancel(ToolCancellationReason::SiblingError);
        assert_eq!(state.state, ToolExecutionState::Cancelled);
        assert_eq!(
            state.cancellation,
            Some(ToolCancellationReason::SiblingError)
        );

        state.mark_yielded();
        assert_eq!(state.state, ToolExecutionState::Yielded);
        assert_eq!(state.cancellation, None);

        state.mark_completed();
        assert_eq!(state.state, ToolExecutionState::Completed);
        assert_eq!(state.percent, Some(100.0));
    }

    #[test]
    fn persisted_result_summary_truncates_utf8_preview_and_formats_notice() {
        let summary = PersistedToolResultSummary::from_content(
            "tool-results/demo.txt",
            "héllo world",
            false,
            3,
        );

        assert_eq!(summary.preview, "hé");
        assert!(summary.has_more);
        let notice = summary.format_notice(3);
        assert!(notice.starts_with(PERSISTED_OUTPUT_TAG));
        assert!(notice.contains("tool-results/demo.txt"));
        assert!(notice.contains("Preview (first 3 B):"));
        assert!(notice.contains("hé"));
        assert!(notice.ends_with(PERSISTED_OUTPUT_CLOSING_TAG));
    }

    #[test]
    fn runtime_limits_match_public_constants() {
        assert_eq!(
            ToolRuntimeLimits::default(),
            ToolRuntimeLimits {
                max_result_size_chars: DEFAULT_MAX_RESULT_SIZE_CHARS,
                max_result_tokens: MAX_TOOL_RESULT_TOKENS,
                max_result_bytes: MAX_TOOL_RESULT_BYTES,
                max_results_per_message_chars: MAX_TOOL_RESULTS_PER_MESSAGE_CHARS,
            }
        );
    }

    #[test]
    fn static_deny_filter_ignores_non_rule_denials() {
        let mut ctx = context();
        ctx.permission_mode = PermissionMode::DontAsk;
        let spec = ToolSpec {
            destructive: true,
            ..spec("file_write")
        };

        assert!(!statically_denied_by_rule(&spec, &ctx));
        assert!(matches!(
            ctx.permission_context()
                .evaluate(&PermissionRequest::new("file_write").destructive(true)),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn merge_and_filter_helpers_leave_alias_matching_case_insensitive() {
        let mut spec = spec("todo");
        spec.aliases.push("TodoWrite".into());

        assert!(tool_is_allowed(
            &spec,
            Some(&BTreeSet::from(["todowrite".to_string()]))
        ));
        assert!(tool_is_allowed(
            &spec,
            Some(&BTreeSet::from(["/TODO".to_string()]))
        ));
    }

    #[test]
    fn preview_text_keeps_short_content_inline() {
        let summary = PersistedToolResultSummary::from_content(
            "tool-results/demo.json",
            &json!({"ok": true}).to_string(),
            true,
            PREVIEW_SIZE_BYTES,
        );

        assert!(!summary.has_more);
        assert!(summary.is_json);
    }
}
