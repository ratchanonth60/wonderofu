# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Overview

`wonder-of-u` is a Rust port of the Claude interactive CLI with a full ratatui TUI. It's a multi-crate workspace (edition 2024, MSRV 1.85) that ships a single binary for interactive AI agent conversations with tool execution, provider management, and persistent sessions.

**Active direction: codex-rs port.** The project is being rebuilt (in phases, behind the `wonder-of-u-async` cargo feature) to mirror the architecture of openai/codex `codex-rs`: a tokio Submission-Queue / Event-Queue core, a TUI with composer + transcript + inline approvals + slash popups, and a multitool clap CLI. The legacy blocking build remains the default and must keep passing while the port lands. See `docs/codex-port.md` for the phased plan.

## Codex Port — Async Foundation (Phase 0+)

This port reverses the legacy "blocking runtime" decision. The new architecture is a tokio event loop speaking a **Submission Queue (SQ) / Event Queue (EQ) protocol**:

- **TUI/CLI → core**: send `Op`s over `mpsc` (e.g. `Op::UserInput`, `Op::Interrupt`, `Op::ExecApproval`, `Op::Compact`).
- **Core → TUI/CLI**: stream `Event { id, msg: EventMsg }` back (e.g. `AgentMessage`, `ExecCommandBegin`, `ExecCommandOutputDelta`, `ExecCommandEnd`, `ExecApprovalRequest`, `TokenCount`).
- **Channels**: app-events + per-thread event channel + terminal events + app-server events, multiplexed via `tokio::select!`.

### Feature flag: `wonder-of-u-async`

The async path is opt-in via a workspace cargo feature. Default is OFF so the existing blocking build keeps passing throughout the port:

```bash
# Default build (legacy blocking path)
cargo build --workspace

# Async path (incremental, crate-by-crate as the port lands)
cargo build -p wonder-of-u-protocol --features wonder-of-u-async
cargo build -p wonder-of-u-core --features wonder-of-u-async
# ...etc
```

When the port stabilizes, the legacy blocking path will be removed and `wonder-of-u-async` becomes the default. Until then, both must coexist.

### `wonder-of-u-protocol` crate (Phase 0)

New crate at `crates/wonder-of-u-protocol/`. Holds the wire contract:
- `Submission { id, op, ... }` — SQ entry
- `Op` — user→agent operations
- `Event { id, msg }` — EQ entry
- `EventMsg` — agent→user events

No behavior, no I/O — pure data types with serde. Phase 0 must round-trip every `Op`/`EventMsg` variant via serde JSON.

### Phased rollout (full plan in `docs/codex-port.md`)

| Phase | Scope | Status |
| --- | --- | --- |
| 0 | `wonder-of-u-protocol` crate + tokio stub + feature flag | in progress |
| 1 | Core agent loop on SQ/EQ (reqwest async SSE, turn driver, interrupt, approval) | pending |
| 2 | CLI multitool rebuild (exec, login, mcp, session cmds, doctor, completion) | pending |
| 3 | TUI rewrite (codex look, no sidebar, composer + transcript + popups) | pending |
| 4 | Slash commands (codex set minus pets/app/cloud/feedback/personality) | pending |
| 5 | State / threads / sessions (thread-store + rollout) | pending |
| 6 | MCP server + polish | pending |

Branch: `feat/codex-port`. The blocking path is the merge blocker — every phase must keep `cargo build --workspace`, `cargo test --workspace -- --test-threads=1`, and the existing TUI passing without the feature.

## Build, Test, and Development Commands

### Building and Running

```bash
# Format check
cargo fmt --all -- --check

# Type check
cargo check --workspace

# Run clippy (must pass with no warnings)
cargo clippy --workspace --all-targets -- -D warnings

# Build release binary
cargo build --release

# Run locally with isolated storage
cargo run -p wonder-of-u-cli -- --storage-dir ./tmp/dev status
cargo run -p wonder-of-u-cli -- doctor
cargo run -p wonder-of-u-cli -- tui
```

### Testing

Tests must be run in specific ways due to filesystem/process/git interactions:

```bash
# TUI tests (parallel safe)
cargo test -p wonder-of-u-tui

# Tools tests (parallel safe)
cargo test -p wonder-of-u-tools

# CLI integration tests (MUST use single thread)
cargo test -p wonder-of-u-cli -- --test-threads=1

# Run all workspace tests
cargo test --workspace -- --test-threads=1
```

**CRITICAL**: CLI tests MUST use `--test-threads=1` because they exercise process, filesystem, and git-backed flows that conflict when parallelized.

### Running a Single Test

```bash
# Run specific test in a crate
cargo test -p wonder-of-u-cli <test-name> -- --test-threads=1

# Run with output visible
cargo test -p wonder-of-u-tools <test-name> -- --nocapture
```

### Installation

```bash
# Install to default location
./install.sh

# Install to specific prefix
./install.sh --prefix "$HOME/.local"

# Uninstall
./install.sh --uninstall
```

## High-Level Architecture

### Crate Organization (Layered)

The workspace follows a strict layered architecture. Lower layers must never depend on upper layers:

**Foundation Layer:**
- `wonder-of-u-protocol`: SQ/EQ wire contract (Submission/Op, Event/EventMsg) — pure data types with serde. No I/O. Used by every upper layer.
- `wonder-of-u-core`: Framework-neutral contracts, data models (messages, sessions, tools, permissions, providers), shared state types, error definitions
- `wonder-of-u-storage`: Persistence layer with append-only JSONL transcripts + atomic JSON snapshots for sessions, tasks, costs, and pastes

**Domain Layer:**
- `wonder-of-u-agent`: Provider integration (Anthropic/OpenAI/Copilot HTTP runtime), tool execution orchestration, streaming SSE responses
- `wonder-of-u-tools`: Tool implementations (74+ tools: bash, file ops, web, MCP, agents, tasks), permission evaluation
- `wonder-of-u-mcp`: MCP config/discovery support
- `wonder-of-u-plugins`: Plugin manifest catalog/runtime glue
- `wonder-of-u-skills`: Skill catalog and invocation support

**Presentation Layer:**
- `wonder-of-u-tui`: Pure UI components (ratatui widgets, event handling, rendering pipeline) - **no business logic**
- `wonder-of-u-cli`: Application orchestration (TUI controller, command handlers, main binary entrypoint)

**Support:**
- `wonder-of-u-test-support`: Test fixtures and helpers

### Key Architectural Patterns

#### State Management
- **Central State**: `AppState` (in `wonder-of-u-core/src/app.rs`) contains: SessionState, messages, queued_commands, background_tasks, provider/model config, auth, costs, permission_mode
- **TUI Controller**: Owns AppState directly during event loop (no locks). Maintains separate ephemeral UI state (scroll position, sidebar_visible, active_suggestions) that is never persisted
- **Non-TUI Contexts**: `StateStore` with `Arc<RwLock<AppState>>` for testing and CLI commands

#### Event Loop (TUI Runtime)
Located in `crates/wonder-of-u-cli/src/tui_runtime/`:
```
EventLoop → Controller.handle_event() → render_tui() → mark_rendered()
```
- Events: Key, Paste, Tick, FocusGained/Lost, Resize, Mouse
- Tick events (500ms) drive polling: OAuth status, task updates, notification TTL
- Controller state is split: persistent (AppState) + ephemeral (scroll, overlays, pickers)

#### Pure Functional Rendering
All TUI rendering in `wonder-of-u-tui` is pure functional:
- Components receive state, return measurements
- No mutation of AppState from UI layer
- Pattern: `render_shell(frame, state, controller_state) -> FrameBuffer`
- Snapshot testing via `render_snapshot()` in `crates/wonder-of-u-tui/src/render.rs`

**IMPORTANT**: If you change TUI layout, update snapshot expectations intentionally and keep cursor helpers in `crates/wonder-of-u-cli/src/tui_runtime/helpers.rs` aligned with prompt geometry.

#### Persistence & Storage
- **Messages**: Append-only JSONL at `sessions/{session_id}.jsonl`
- **Metadata**: Atomic JSON at `sessions/.metadata/{session_id}.json`
- **Snapshots**: Full AppState at `sessions/.snapshots/{session_id}.json` (for O(1) resume)
- **Pattern**: Atomic writes via `.next` temp files + rename, fsync on critical paths
- **Schema Versioning**: All persisted structs carry `schema_version: u16`, reject unsupported versions explicitly
- **Error Recovery**: Corrupt trailing JSONL lines are tolerated (skip during load)

#### Provider Integration
Located in `crates/wonder-of-u-agent/src/`:
- `ProviderRegistry` + `ProviderResolver` pattern
- Supported: Anthropic (Claude), OpenAI (GPT-4), GitHub Copilot (OAuth device flow)
- `ProviderRuntime` trait for HTTP request/response (blocking via `ureq`, timeout: 60s)
- Streaming SSE responses with tool call batching
- OAuth token refresh with 60s skew for Copilot

#### Tool Execution & Permissions
Located in `crates/wonder-of-u-tools/` and `crates/wonder-of-u-core/src/tool.rs`:

**Tool Trait**:
```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn permission_decision(&self, context, input) -> PermissionDecision;
    async fn execute(&self, context, use_id, input) -> Result<ToolResult>;
}
```

**Permission System** (`core/permission.rs`):
- Modes: Default, AcceptEdits, BypassPermissions, DontAsk, Plan
- Three outcomes: Allow, Deny, Ask (with reason)
- Rule precedence: Deny(0) < Ask(1) < Allow(2)
- Source precedence: Policy(0) ... User(6)
- Shell safety checks: detect `${cmd@P}`, `eval`, etc.

**Feature Gating** (`core/feature.rs`):
- FeatureSet: BTreeSet<FeatureFlag>
- Flags: Tools, BackgroundTasks, Agents, Skills, MCP, WebTools, RemoteTriggers, TestTools
- Tools filtered by `required_features` at runtime

**Tool Registry**:
- 74+ built-in tools registered at startup (`builtin_registry()`)
- Aliases resolved (e.g., "Read" → "file_read", "Bash" → "bash")
- MAX_TOOL_LOOP_ITERATIONS = 6

#### Error Handling
- Custom `WonderError` enum: NotFound, Validation, Internal, Json, Io
- `Result<T>` = `std::result::Result<T, WonderError>`
- `thiserror` for `derive(Error)` across all crates

#### Async Patterns

The runtime is in transition. There are two paths; the legacy blocking path is the default today, and the async path is being ported in phases.

**Legacy blocking path (default):**
- Tools use `#[async_trait]` but the runtime is blocking (`futures::executor::block_on`)
- HTTP via ureq (blocking, single-threaded, 60s timeout)
- Streaming SSE responses are assembled into a finished string before returning

**Async path (`wonder-of-u-async` feature):**
- Tokio runtime (`#[tokio::main]`) driving an async event loop
- `ConversationManager` owns the thread and exposes a Submission Queue (SQ) / Event Queue (EQ) protocol
- TUI/CLI send `Op`s (e.g. `UserInput`, `Interrupt`, `ExecApproval`, `Compact`) over `mpsc`
- Core streams back `Event { id, msg: EventMsg }` (e.g. `AgentMessage`, `ExecCommandBegin`, `ExecCommandOutputDelta`, `ExecCommandEnd`, `ExecApprovalRequest`, `TokenCount`)
- Channels multiplexed via `tokio::select!`: app-events + per-thread event channel + terminal events + app-server events
- HTTP via `reqwest` (async) + `tokio` SSE for streaming deltas

Both paths must coexist during the port. The async path is opt-in via the `wonder-of-u-async` feature; do not enable it on crates that haven't been ported.

## Git Flow and Branching

**IMPORTANT**: This project uses a strict branching model:

- **Main branch**: `master` (updated only via release flow)
- **Development branch**: `dev` (primary integration branch)
- **Topic branches**: `feat/*`, `fix/*`, `perf/*`, `test/*`, `refactor/*`

### Workflow
1. **Before starting work**: Ensure `dev` is up to date
   ```bash
   git fetch && git checkout dev && git pull --ff-only
   ```

2. **Create topic branch from `dev`**:
   ```bash
   git checkout -b feat/short-description
   ```

3. **Make focused commits** using Conventional Commits:
   - `feat(core): add session tagging support`
   - `fix(cli): resolve transcript corruption on crash`
   - `test(storage): cover snapshot recovery edge cases`
   - `perf(tools): reduce allocations in file_read`

4. **Merge into `dev`** (only after verification and user confirmation):
   ```bash
   git checkout dev && git pull --ff-only
   git merge --no-ff <branch> -m "merge: <branch> into dev"
   ```

5. **Never commit directly to `dev` or `master`**. Always use topic branches.

6. **Never push or merge without explicit user confirmation**. Show the planned commands first.

## Code Conventions

### Comments and Documentation
- `//!` for module-level docs
- `///` for public item docs (prefer summary + examples when helpful)
- `//` for inline implementation notes explaining **why**, not obvious mechanics
- Avoid filler comments that restate the code

Example:
```rust
//! This module provides session persistence.

/// Loads a session snapshot from disk.
///
/// # Examples
///
/// ```
/// let snapshot = load_snapshot(&store, session_id)?;
/// ```
pub fn load_snapshot(store: &Store, id: Uuid) -> Result<Snapshot> {
    // Try snapshot first (O(1)) before falling back to transcript replay
    ...
}
```

### Rust Style
- Prefer iterators over loops, `?` for error propagation
- Use `&str`/`&[T]` over owned types where possible
- Add `#[must_use]` on builder methods
- Avoid `unsafe` unless explicitly approved (requires `// SAFETY:` comment)
- Check workspace dependencies first before adding new crates (prefer existing: `serde`, `thiserror`, `ratatui`, `crossterm`, `ureq`)

### Tool Concurrency
- Sequential execution (no parallel tool calls)
- `ToolConcurrencyClass`: Sequential, Concurrent, Exclusive
- `read_only + concurrency_safe` flags on ToolSpec

## Key Files and Entry Points

### Main Binary
- `crates/wonder-of-u-cli/src/main.rs` - CLI entrypoint with clap commands
- `crates/wonder-of-u-cli/src/tui_runtime/mod.rs` - TUI event loop orchestration
- `crates/wonder-of-u-cli/src/tui_runtime/controller.rs` - Controller owning state + ephemeral UI state

### State and Messages
- `crates/wonder-of-u-core/src/app.rs` - Central AppState definition
- `crates/wonder-of-u-core/src/message.rs` - MessageEnvelope and payload variants
- `crates/wonder-of-u-storage/src/transcript.rs` - JSONL transcript storage
- `crates/wonder-of-u-storage/src/snapshot.rs` - Session snapshot persistence

### Provider and Tool Runtime
- `crates/wonder-of-u-agent/src/provider.rs` - Provider registry and resolution
- `crates/wonder-of-u-agent/src/runtime.rs` - HTTP execution and streaming
- `crates/wonder-of-u-tools/src/registry.rs` - Tool registration
- `crates/wonder-of-u-tools/src/orchestration.rs` - Tool use loop (MAX_TOOL_LOOP_ITERATIONS)
- `crates/wonder-of-u-core/src/permission.rs` - Permission evaluation logic

### TUI Components
- `crates/wonder-of-u-tui/src/render.rs` - Main rendering pipeline and snapshot tests
- `crates/wonder-of-u-tui/src/shell_view.rs` - Chat transcript view
- `crates/wonder-of-u-tui/src/sidebar_view.rs` - Right-side context panel
- `crates/wonder-of-u-cli/src/tui_runtime/event.rs` - Event types and normalization

## Common Pitfalls

1. **Don't use the async core path without the `wonder-of-u-async` cargo feature** - The async runtime (tokio, ConversationManager, SQ/EQ host, async provider) is feature-gated. With the feature off, the legacy blocking path remains the only runnable code. Toggle the feature only on crates that opt in (`wonder-of-u-protocol`, `wonder-of-u-core`, `wonder-of-u-agent`, `wonder-of-u-tui`, `wonder-of-u-cli`). Leaving it off by default keeps `cargo build` and existing tests green while the port lands.

2. **Don't mutate AppState from TUI layer** - All mutations happen in controller event handlers. TUI components are pure functions.

3. **Don't skip `--test-threads=1` for CLI tests** - They will fail with race conditions.

4. **Don't add tools without feature gates** - Check if the tool should be behind a FeatureFlag.

5. **Don't forget schema versioning** - All persisted types need `schema_version: u16`.

6. **Don't commit to `dev` directly** - Always use topic branches and merge with `--no-ff`.

7. **Don't claim optimization works without measurement** - Use criterion, `cargo test --release`, or `/usr/bin/time`.

## Testing Philosophy

- **Unit tests**: Co-locate in `#[cfg(test)] mod tests` within implementation files
- **Integration tests**: Place under `<crate>/tests/`
- **Test support**: Reuse helpers from `wonder-of-u-test-support`
- **Coverage**: Happy path + at least one failure/edge case per feature
- **TUI snapshots**: Deterministic frame snapshots in `wonder-of-u-tui/src/render.rs`

## Coordinator Modes

From `crates/wonder-of-u-core/src/coordinator.rs`:
- **Direct** (default): Local tool execution
- **Local**: Local subprocess coordination
- **Cloud**: (Unsupported in Rust port - no backend)
- **External**: (Deferred in Rust port)

## Session Memory

Located in `crates/wonder-of-u-core/src/session_memory.rs`:
- `SessionMemoryIndex` extracts searchable text from messages
- Deduplicates by SHA256 of normalized text
- Tracks occurrences, first/last message IDs, timestamps

## Verification Checklist

Before marking work complete:
1. `cargo fmt --all -- --check`
2. `cargo check --workspace`
3. `cargo test -p <modified-crate>` (with `--test-threads=1` for CLI)
4. `cargo clippy --workspace --all-targets -- -D warnings`
5. Manual TUI testing if UI changed: `cargo run -p wonder-of-u-cli -- --storage-dir ./tmp/dev tui`
6. Update TUI snapshot tests if layout changed

## Key Architectural Insights

1. **State is centralized** in AppState but controller owns ephemeral UI state separately
2. **Persistence is split**: Messages in append-only JSONL, state in atomic snapshots
3. **Provider abstraction is minimal**: Direct HTTP calls via ureq, no complex middleware
4. **Tool execution is synchronous** despite async trait (blocking executor)
5. **Permission system is rule-based** with precedence ordering
6. **TUI is pure functional**: Controllers handle events, components render state
7. **No global state**: StateStore uses Arc<RwLock> only for non-TUI contexts
8. **Schema versioning is explicit**: All persisted types carry version numbers
9. **Error recovery is built-in**: Corrupt trailing transcript lines are tolerated
10. **Feature gating is pervasive**: Tools/commands filtered by FeatureSet at runtime

This architecture prioritizes **local-first operation**, **append-only durability**, **permission safety**, and **pure functional rendering** - making it suitable for interactive CLI agents with full audit trails.
