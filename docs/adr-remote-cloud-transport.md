# ADR: Remote / Cloud WebSocket Transport

| Field      | Value                                      |
|------------|--------------------------------------------|
| **Status** | **Blocked / Deferred**                     |
| **Date**   | 2025-01-01                                 |
| **Crate**  | `wonder-of-u-core`, `wonder-of-u-cli`      |
| **Flags**  | `FeatureFlag::RemoteTriggers`              |
| **Ledger** | `docs/parity-ledger.csv` rows tagged `external-service-blocked` |

---

## Context

The upstream TypeScript implementation ships several modules that connect the
CLI to Anthropic's Cloud Code Runner (CCR) backend via WebSocket:

| Upstream file | Role |
|---|---|
| `remote/SessionsWebSocket.ts` | CCR WebSocket session lifecycle |
| `remote/RemoteSessionManager.ts` | Manages multiple CCR sessions |
| `remote/remotePermissionBridge.ts` | Forwards permission grants over CCR |
| `remote/sdkMessageAdapter.ts` | Serialises/deserialises CCR wire messages |
| `cli/transports/WebSocketTransport.ts` | CCR-specific WebSocket transport layer |
| `cli/transports/ccrClient.ts` | CCR OAuth client handshake |

These are **not** general-purpose SSE or in-process WebSocket helpers; they
depend on a proprietary CCR OAuth/session infrastructure and a CCR backend
endpoint that is Anthropic-operated and not publicly specified.

## Decision

**Do not implement real remote/cloud WebSocket transport at this time.**

The following hard blockers exist:

1. **No `tokio`/`tokio-tungstenite` WebSocket dependency** — the workspace
   deliberately keeps async-runtime surface minimal to avoid binary bloat and
   to preserve compatibility with environments that cannot spawn async runtimes.
   Adding a full WebSocket stack is a separate, reviewable dependency decision.

2. **No CCR backend access** — the CCR service is Anthropic-operated.  There
   is no public API contract, no test environment, and no credentials available
   to this runtime.

3. **No CCR OAuth / session infrastructure** — `ccrClient.ts` implements a
   bespoke OAuth flow tied to `~/.claude/credentials`.  Replicating this
   without the server-side contract would produce an untestable stub.

4. **Missing wire-protocol spec** — `sdkMessageAdapter.ts` encodes/decodes
   messages in an undocumented format.  Any Rust equivalent would be
   speculative and unverifiable.

## Current honest stubs

`wonder-of-u-core` models the deferred state explicitly rather than silently
omitting it.  Any call-site that would need a live transport gets a
`TaskBackendSupport::Deferred` value with a machine-readable reason string:

```rust
// In RemoteTaskState::deferred():
TaskBackendState::deferred(
    TaskBackendFlow::Transport,
    "{task_label} tasks require a remote session transport \
     that is not implemented in this Rust runtime",
)
```

`RemoteTaskType` covers all upstream task kinds (`RemoteAgent`, `BackgroundPr`,
`AutofixPr`, `Ultraplan`, `Ultrareview`) so the domain model is complete — only
the live transport is absent.

The CLI commands `remote-env` and `remote-setup` surface honest error messages
(see `wonder-of-u-cli/src/commands/extras.rs`) rather than pretending to work.

## Future implementation gate

Real remote transport **must not** be activated until **all** of the following
are true:

- [ ] `FeatureFlag::RemoteTriggers` is added to `FeatureSet::first_release()`
      (currently absent — this is the canonical gate).
- [ ] `tokio-tungstenite` (or equivalent) has been reviewed and approved as a
      workspace dependency via the normal dep-addition process.
- [ ] A CCR endpoint spec (or OpenAPI schema) is available and pinned in
      `docs/`.
- [ ] CCR OAuth flow can be integration-tested against a sandbox environment.
- [ ] `remote/sdkMessageAdapter.ts` wire format is documented and a Rust
      `serde` codec is validated against the upstream test corpus.

Until every gate is checked, any pull request that sets
`TaskBackendSupport::Supported` on a CCR transport **must be rejected**.

## Consequences

* **Parity ledger** — `remote/SessionsWebSocket.ts`, `remote/RemoteSessionManager.ts`,
  `remote/remotePermissionBridge.ts`, `remote/sdkMessageAdapter.ts`, and
  `cli/transports/WebSocketTransport.ts` are marked `external-service-blocked`
  in `docs/parity-ledger.csv`, referencing this ADR.
* **Tests** — `wonder-of-u-core` tests anchor three guarantees:
  1. `RemoteTaskState::deferred(RemoteTaskType::RemoteAgent, None)` produces
     `TaskBackendSupport::Deferred` and a reason string that mentions
     "not implemented".
  2. Every `RemoteTaskType::label()` value is unique and stable.
  3. `FeatureSet::first_release()` does **not** contain
     `FeatureFlag::RemoteTriggers`.
* **No binary size regression** — the absence of a WebSocket dependency means
  release builds are not bloated by an async TLS/WebSocket stack.
