//! Background memory extraction: after each complete AI turn, a lightweight
//! sub-agent analyses recent messages and saves noteworthy information to the
//! auto-memory directory.
//!
//! The extraction runs in a detached thread so it never blocks the TUI.
//! Failures are silently dropped — extraction is best-effort.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use futures::executor::block_on;
use serde_json::Value;
use wonder_of_u_agent::{
    ProviderRuntime, ProviderToolResultMessage, ProviderToolSpec, ResolvedProviderExecution,
    ToolConversationRound, ToolUseRequest, ToolUseResponse,
};
use wonder_of_u_core::{
    FeatureSet, MessageEnvelope, MessagePayload, PermissionMode, Result, SessionId, ToolContext,
    ToolResult, ToolUseId, resolve_path,
};
use wonder_of_u_storage::memdir;

fn tool_spec_to_provider_tool(spec: wonder_of_u_core::ToolSpec) -> ProviderToolSpec {
    ProviderToolSpec {
        name: spec.name,
        description: spec.description,
        input_schema: spec.input_schema,
    }
}

/// Minimum new model-visible messages since last extraction before we run again.
const MIN_NEW_MESSAGES: usize = 4;
/// Maximum recent messages to include as context for the sub-agent.
const MAX_CONTEXT_MESSAGES: usize = 20;
/// Maximum tool-loop rounds the sub-agent may use.
const MAX_EXTRACTION_ROUNDS: usize = 4;

// ─── Handle ──────────────────────────────────────────────────────────────────

/// Tracks the running state of the background extraction thread.
#[derive(Debug)]
pub(super) struct ExtractionHandle {
    active: Arc<AtomicBool>,
    pub(super) last_count: usize,
}

impl ExtractionHandle {
    pub(super) fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(false)),
            last_count: 0,
        }
    }

    /// Returns `true` if conditions are met to start a new extraction.
    pub(super) fn should_run(&self, current_count: usize) -> bool {
        !self.active.load(Ordering::Relaxed) && current_count >= self.last_count + MIN_NEW_MESSAGES
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

/// Spawn a background extraction thread if conditions are met.
///
/// Takes a snapshot of recent messages and relevant paths, then detaches.
/// The `ExtractionHandle` is updated: `last_count` is bumped here, and the
/// `active` flag is cleared when the thread finishes.
pub(super) fn maybe_spawn_extract_memories(
    handle: &mut ExtractionHandle,
    messages: &[MessageEnvelope],
    resolved: ResolvedProviderExecution,
    cwd: &Path,
    storage_dir: Option<&Path>,
) {
    let count = count_model_visible(messages);
    if !handle.should_run(count) {
        return;
    }

    let new_msgs = count.saturating_sub(handle.last_count);
    handle.last_count = count;

    let conversation = build_conversation_text(messages);
    let mem_dir = memdir::auto_mem_dir(cwd, storage_dir);

    // Ensure memory dir exists — create silently if not.
    let _ = std::fs::create_dir_all(&mem_dir);

    let active = Arc::clone(&handle.active);
    active.store(true, Ordering::Relaxed);

    let cwd_buf = cwd.to_path_buf();
    let storage_dir_buf = storage_dir.map(Path::to_path_buf);
    std::thread::spawn(move || {
        let existing = memdir::read_auto_mem_entrypoint(&cwd_buf, storage_dir_buf.as_deref());
        let _ = run_extraction(
            &conversation,
            new_msgs,
            &resolved,
            &mem_dir,
            existing.as_deref(),
            storage_dir_buf.as_deref(),
        );
        active.store(false, Ordering::Relaxed);
    });
}

// ─── Core mini tool loop ──────────────────────────────────────────────────────

fn run_extraction(
    conversation: &str,
    new_message_count: usize,
    resolved: &ResolvedProviderExecution,
    mem_dir: &Path,
    existing_memories: Option<&str>,
    storage_dir: Option<&Path>,
) -> Result<()> {
    let registry = build_limited_registry(storage_dir)?;
    let context = build_extraction_context(mem_dir);
    let system = build_extraction_system_prompt(new_message_count, existing_memories, mem_dir);

    let provider_tools: Vec<ProviderToolSpec> = registry
        .all_specs()
        .into_iter()
        .filter(|s| is_allowed_tool(&s.name))
        .map(tool_spec_to_provider_tool)
        .collect();

    let mut rounds: Vec<ToolConversationRound> = Vec::new();

    for _ in 0..MAX_EXTRACTION_ROUNDS {
        let request = ToolUseRequest {
            prompt: conversation.to_string(),
            system_prompt: Some(system.clone()),
            max_output_tokens: Some(4096),
            temperature: None,
            tools: provider_tools.clone(),
            rounds: rounds.clone(),
            effort_level: None,
            images: Vec::new(),
        };

        match ProviderRuntime::new().complete_with_tool_use(resolved, &request)? {
            ToolUseResponse::Final(_) => break,
            ToolUseResponse::ToolCalls(batch) => {
                let mut results = Vec::new();
                for call in &batch.calls {
                    let use_id = ToolUseId::new();
                    let result =
                        execute_extraction_tool(&registry, &context, call, use_id, mem_dir);
                    let content = if result.success {
                        result.content.clone()
                    } else {
                        format!("ERROR: {}", result.content)
                    };
                    results.push(ProviderToolResultMessage {
                        call_id: call.call_id.clone(),
                        content,
                    });
                }
                rounds.push(ToolConversationRound {
                    assistant_text: batch.assistant_text,
                    calls: batch.calls,
                    results,
                });
            }
        }
    }
    Ok(())
}

// ─── Tool execution (restricted) ─────────────────────────────────────────────

fn is_allowed_tool(name: &str) -> bool {
    matches!(
        name,
        "file_read" | "file_write" | "file_edit" | "glob" | "grep"
    )
}

fn execute_extraction_tool(
    registry: &wonder_of_u_core::ToolRegistry,
    context: &ToolContext,
    call: &wonder_of_u_agent::ProviderToolCall,
    use_id: ToolUseId,
    mem_dir: &Path,
) -> ToolResult {
    let Some(tool) = registry.resolve(&call.tool_name) else {
        return ToolResult::failure(use_id, format!("unknown tool: {}", call.tool_name));
    };

    if !is_allowed_tool(tool.spec().name.as_str()) {
        return ToolResult::failure(
            use_id,
            format!(
                "tool '{}' is not available to the memory agent",
                call.tool_name
            ),
        );
    }

    // For write tools, restrict target path to the memory directory.
    if matches!(tool.spec().name.as_str(), "file_write" | "file_edit") {
        if let Some(path_str) = call.arguments.get("path").and_then(Value::as_str) {
            let resolved = resolve_path(Path::new(path_str), &context.cwd);
            if !resolved.starts_with(mem_dir) {
                return ToolResult::failure(
                    use_id,
                    format!(
                        "write denied: path '{}' is outside the memory directory",
                        path_str
                    ),
                );
            }
        }
    }

    match block_on(tool.execute(context.clone(), use_id, call.arguments.clone())) {
        Ok(result) => result,
        Err(e) => ToolResult::failure(use_id, format!("tool error: {e}")),
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn count_model_visible(messages: &[MessageEnvelope]) -> usize {
    messages
        .iter()
        .filter(|m| {
            matches!(
                m.payload,
                MessagePayload::UserText { .. } | MessagePayload::AssistantText { .. }
            )
        })
        .count()
}

fn build_conversation_text(messages: &[MessageEnvelope]) -> String {
    let visible: Vec<_> = messages
        .iter()
        .filter(|m| {
            matches!(
                m.payload,
                MessagePayload::UserText { .. } | MessagePayload::AssistantText { .. }
            )
        })
        .collect();

    // Take last MAX_CONTEXT_MESSAGES
    let start = visible.len().saturating_sub(MAX_CONTEXT_MESSAGES);
    let recent = &visible[start..];

    let mut parts = Vec::new();
    for msg in recent {
        match &msg.payload {
            MessagePayload::UserText { content } => {
                parts.push(format!("User: {}", content.trim()));
            }
            MessagePayload::AssistantText { content } => {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    parts.push(format!("Assistant: {trimmed}"));
                }
            }
            _ => {}
        }
    }
    parts.join("\n\n")
}

fn build_limited_registry(storage_dir: Option<&Path>) -> Result<wonder_of_u_core::ToolRegistry> {
    let full = match storage_dir {
        Some(root) => wonder_of_u_tools::builtin_registry_with_mcp_catalog(root)?,
        None => wonder_of_u_tools::builtin_registry()?,
    };
    let mut limited = wonder_of_u_core::ToolRegistry::new();
    for spec in full.all_specs() {
        if is_allowed_tool(&spec.name) {
            if let Some(tool) = full.resolve(&spec.name) {
                let _ = limited.register(tool);
            }
        }
    }
    Ok(limited)
}

fn build_extraction_context(mem_dir: &Path) -> ToolContext {
    ToolContext {
        session_id: SessionId::new(),
        cwd: mem_dir.to_path_buf(),
        session_worktree: None,
        permission_mode: PermissionMode::BypassPermissions,
        additional_working_directories: Vec::new(),
        provider: None,
        model: None,
        permission_rules: Vec::new(),
        features: FeatureSet::first_release(),
        bash_session_store: None,
        progress_tx: None,
        interaction_rx: None,
        fork_context: None,
    }
}

fn build_extraction_system_prompt(
    new_message_count: usize,
    existing_memories: Option<&str>,
    mem_dir: &Path,
) -> String {
    let mem_dir_str = mem_dir.display();
    let existing_section = match existing_memories.filter(|s| !s.trim().is_empty()) {
        Some(content) => format!(
            "\n\n## Existing memory index (MEMORY.md)\n\n{content}\n\nCheck this before writing — update an existing file rather than creating a duplicate."
        ),
        None => String::new(),
    };

    format!(
        r#"You are the memory extraction agent for wonder-of-u.

Analyse the most recent ~{new_message_count} messages in the conversation above and save any noteworthy information to the memory directory: `{mem_dir_str}`

## What to save

Save facts that are non-obvious, surprising, or that would be useful in future conversations:

- **user**: Who the user is, their role, goals, expertise, preferences
- **feedback**: How they want you to work — corrections AND confirmations of non-obvious approaches
- **project**: Ongoing work, decisions, deadlines, architectural constraints
- **reference**: Pointers to external systems (repos, dashboards, issue trackers)

## What NOT to save

- Code patterns, conventions, or file paths that are readable from the source
- Git history or recent changes (git log/blame are authoritative)
- Ephemeral task details or current conversation context
- Anything already in CLAUDE.md files

## How to save memories

**Step 1** — write each memory to its own file (e.g. `user_role.md`) with this frontmatter:
```
---
name: short-kebab-case-slug
description: one-line summary used to decide relevance in future conversations
metadata:
  type: user|feedback|project|reference
---

[memory body — for feedback/project: lead with the rule/fact, then **Why:** and **How to apply:** lines]
```

**Step 2** — add a one-line pointer to `MEMORY.md`:
`- [Title](file.md) — one-line hook (under ~150 chars)`

`MEMORY.md` is an index, not a memory — no frontmatter, no content, just pointers.{existing_section}

## Efficiency

You have a limited turn budget. Efficient strategy:
- Turn 1: read all files you might update (parallel file_read calls)
- Turn 2: write all changes (parallel file_write/file_edit calls)

Only use content from the recent messages. Do not investigate further — no grepping source files, no reading code.

If nothing is worth saving, respond with a short explanation and stop."#
    )
}
