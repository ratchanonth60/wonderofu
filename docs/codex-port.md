# Codex-rs Port — Architecture & Phased Plan

This document captures the active direction: rebuilding `wonder-of-u` to mirror
[openai/codex `codex-rs`](https://github.com/openai/codex/tree/main/codex-rs).
It is the canonical reference for the port; `CLAUDE.md` summarizes and links
here.

## Why

Codex-rs is the canonical example of a modern, local-first, async CLI agent:

- Tokio event loop driving a Submission Queue (SQ) / Event Queue (EQ) protocol.
- Inline approvals, streaming tool output, mid-turn interrupts.
- Multitool clap CLI with `exec`, `login`, `mcp`, session commands, doctor,
  completion, and a default subcommand that drops into a full TUI.
- TUI as a codex-style transcript + composer + popups — no sidebar.

`wonder-of-u` already has much of the surface area (providers, tools,
permissions, snapshots). The port reworks the **architecture** to match codex's
async core, then layers a codex-shaped UX on top.

## Load-bearing decision: tokio in core

The legacy blocking runtime (ureq + `futures::executor::block_on`) is the merge
blocker for every other phase. Phase 0 lifts the "no tokio in core" rule from
`CLAUDE.md` and gates the new async path behind a `wonder-of-u-async` cargo
feature. Default is OFF so `cargo build --workspace` keeps passing throughout
the port.

## Wire contract: SQ / EQ

- **TUI/CLI → core** send `Submission { id, op }` over `mpsc`.
- **Core → TUI/CLI** stream `Event { id, msg: EventMsg }` back.
- Channels multiplexed via `tokio::select!`:
  - App-events MPSC.
  - Per-thread event channel.
  - Terminal events (crossterm).
  - App-server events (Phase 2+).

`Op` and `EventMsg` enums are pure data — no behavior, no I/O — in the
`wonder-of-u-protocol` crate (added in Phase 0). Every variant must round-trip
through serde JSON.

## Target crate layout

Codex has ~40 micro-crates. We consolidate to ~14 meaningful boundaries; no
behavior is gained from the wider split, and the maintenance cost is real.

| New / changed crate       | From codex                  | From wonder-of-u           | Purpose                                                |
| ------------------------- | --------------------------- | -------------------------- | ------------------------------------------------------ |
| `wonder-of-u-protocol`    | `protocol`                  | —                          | Submission/Op, Event/EventMsg, IDs. The contract.      |
| `wonder-of-u-core`        | `core` + `core-api`         | `agent` + parts of `core`  | Async agent loop, ConversationManager, SQ/EQ host.     |
| `wonder-of-u-model-provider` | `model-provider`, `ollama`, `lmstudio` | `agent/provider.rs`, `runtime.rs` | Async streaming providers. ureq → reqwest/tokio SSE.   |
| `wonder-of-u-login`       | `login`, `keyring-store`    | OAuth bits in `agent`      | Provider auth / OAuth device flow, token store.        |
| `wonder-of-u-exec`        | `exec`, `execpolicy`, sandboxing | `tools/orchestration.rs`, `permission` | Tool/shell execution, approval policy, sandbox.         |
| `wonder-of-u-tools`       | (tool defs)                 | `tools`                    | Tool impls, now emitting `Event`s instead of blobs.    |
| `wonder-of-u-mcp`         | `mcp-client`, `rmcp-client` | `mcp`                      | MCP discovery + client.                                |
| `wonder-of-u-mcp-server`  | `mcp-server`                | —                          | Run wonder-of-u as an MCP stdio server.                |
| `wonder-of-u-state`       | `state`, `thread-store`, `message-history`, `memories` | `storage` | Sessions, threads, rollout, history, memory. Schema versioning preserved. |
| `wonder-of-u-plugins` / `wonder-of-u-skills` | `core-plugins`, `core-skills` | `plugins`, `skills` | Unchanged role. |
| `wonder-of-u-tui`         | `tui`                       | `tui` + `cli/tui_runtime`  | Full rewrite (codex look).                             |
| `wonder-of-u-cli`         | `cli`, exec entry           | `cli`                      | Multitool clap entrypoint.                             |
| `wonder-of-u-common`      | `common`                    | (shared types)             | Shared helpers.                                        |
| `wonder-of-u-test-support` | —                          | `test-support`             | Fixtures.                                              |

Naming `wou-*` vs `wonder-of-u-*` is cosmetic; keep `wonder-of-u-*` to avoid
rename churn.

## Phased rollout

Each phase is independently buildable and testable. The legacy blocking build
must keep passing at every merge point.

### Phase 0 — Protocol + async foundation

- New `wonder-of-u-protocol` crate: port `Submission`/`Op` and `Event`/`EventMsg`
  enums from codex `protocol/src/protocol.rs`. Keep only Ops we will implement;
  drop realtime-audio/voice and other codex-only stubs.
- Add tokio behind `wonder-of-u-async`. Update `CLAUDE.md` pitfall #1.
- Define `ConversationManager` + channel wiring stub (no behavior yet).
- Tests: round-trip every `Op`/`EventMsg` variant via serde JSON.

### Phase 1 — Core agent loop on SQ/EQ

- Rewrite `wonder-of-u-agent` provider runtime async (`reqwest` + tokio SSE),
  emitting `EventMsg` deltas instead of returning a finished string.
- Turn driver: consume `Op::UserInput`, run model→tool loop, stream
  `AgentMessage` / `ExecCommandBegin` / `OutputDelta` / `End`, honor
  `Op::Interrupt`.
- Approval flow: emit `ExecApprovalRequest`, await `Op::ExecApproval`.
- Reuse existing `core/permission.rs` behind `wonder-of-u-exec`.
- Tests: headless turn over a mock provider; assert event sequence + interrupt.

### Phase 2 — CLI multitool

Rebuild `crates/wonder-of-u-cli/src/lib.rs` clap tree to codex's `MultitoolCli`:

- `exec` (non-interactive), `login`/`logout`, `mcp`, `mcp-server`,
  `resume`/`fork`/`archive`/`delete`, `apply`, `doctor`, `completion`,
  `debug` (`models`, `prompt-input`).
- Default (no subcommand) → TUI, optional `PROMPT`.
- Skip codex-only: `app` (desktop), `cloud`, `remote-control`, `update`.
- Map current commands onto these (e.g. current `prompt` → `exec`,
  `session`/`resume`/`rename` → codex session cmds).
- Tests: integration `--test-threads=1`.

### Phase 3 — TUI rewrite (codex look, no sidebar)

Replace `wonder-of-u-tui` + `cli/tui_runtime`:

- `App` struct: `tokio::select!` over app-events / thread-events / terminal /
  app-server, per codex `tui/src/app.rs`.
- `ChatWidget` owns: `Composer` (bottom input, external-editor support),
  transcript cells (deferred-reflow history lines), `Overlay`/`Pager`
  (diff/approval/selection modals), status line (hints + token usage).
- Drop `SidebarView` and the three-band layout entirely.
- Popups: slash-command popup, `@` file-mention popup.
- Inline approval modal driven by `ExecApprovalRequest` events.
- Keep snapshot testing (`render_snapshot()`); rewrite all snapshots.
- Tests: deterministic frame snapshots for composer, transcript, approval
  modal, slash popup.

### Phase 4 — Slash commands (codex set minus irrelevant)

Port codex `tui/src/slash_command.rs` set. Include: `model`, `permissions`,
`keymap`, `vim`, `skills`, `import`, `hooks`, `review`, `rename`, `new`,
`archive`, `delete`, `resume`, `fork`, `init`, `compact`, `plan`, `copy`, `raw`,
`diff`, `mention`, `status`, `usage`, `theme`, `mcp`, `plugins`, `logout`,
`quit`/`exit`, `clear`, `ps`, `stop`. Skip: `pets`, `app`, `cloud`/`cloud-tasks`,
`feedback`, `rollout`, `personality`, `realtime`/`side`/`btw`,
`statusline`/`title` (optional later).

Tests: each command parses + dispatches to its `Op`/handler.

### Phase 5 — State / threads / sessions

- `wonder-of-u-storage` → thread-store + rollout model (codex
  `resume`/`fork`/`archive`).
- Preserve append-only JSONL + atomic snapshots + schema versioning (existing
  strengths — keep them).
- Tests: resume + fork + rollback round-trips.

### Phase 6 — MCP server + polish

- `wonder-of-u-mcp-server` (stdio).
- `import` from Claude Code transcripts (reuse existing `commands::import`).
- `doctor` / `completion`.

## Cross-cutting

- Branch: `feat/codex-port` from `dev`. Merge blocker: legacy blocking path
  must keep passing every phase.
- Feature flag: `wonder-of-u-async` (workspace), default OFF. Apply crate by
  crate as the port lands.
- Drop `SidebarView`, three-band layout, and `tui_runtime` as Phase 3 lands.

## Verification

Per `CLAUDE.md` checklist, every phase:

1. `cargo fmt --all -- --check`
2. `cargo check --workspace`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. `cargo test -p <crate>` (CLI: `--test-threads=1`)
5. TUI snapshot tests updated intentionally.
6. Manual: `cargo run -p wonder-of-u-cli -- --storage-dir ./tmp/dev tui` and
   `... exec "prompt"`.

End-to-end smoke after Phase 3:

- Launch TUI, send a prompt, watch streamed `AgentMessage`.
- Trigger a shell tool → inline approval modal → approve → live
  `ExecCommandOutputDelta` renders.
- `Ctrl+C` interrupts mid-turn.

## Risks / honest notes

- Async reversal is irreversible-ish churn touching every crate. Biggest risk.
- TUI is a from-scratch rewrite, not a refactor — all snapshots discarded.
- Full 40-crate split is not recommended; consolidated to ~14. The plan
  recommends the consolidated layout; say so if literal 1:1 crate count is
  required.
- Estimate: weeks–months. Phase 0+1 land behind the existing blocking path
  first (feature-flagged) so dev stays shippable.