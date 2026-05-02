# Development

## Workspace

`wonder-of-u` is a Rust workspace:

```text
crates/
  wonder-of-u-agent        provider execution and tool-use loops
  wonder-of-u-cli          binary entrypoint, commands, TUI runtime
  wonder-of-u-core         shared state, registry, messages, errors
  wonder-of-u-mcp          MCP config/discovery
  wonder-of-u-plugins      plugin manifests and catalogs
  wonder-of-u-skills       skill catalogs and invocation support
  wonder-of-u-storage      session metadata, transcripts, snapshots
  wonder-of-u-test-support test fixtures/helpers
  wonder-of-u-tools        built-in tool runtime
  wonder-of-u-tui          ratatui renderer and UI components
```

## Validation

Run the same checks used during development:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p wonder-of-u-tui
cargo test -p wonder-of-u-tools
cargo test -p wonder-of-u-cli -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
```

The CLI tests use a single test thread because several cases exercise process,
filesystem, and git-backed flows.

## Running locally

```bash
cargo run -p wonder-of-u-cli -- doctor
cargo run -p wonder-of-u-cli -- tui
cargo run -p wonder-of-u-cli -- prompt "Hello"
```

Use an isolated storage dir while testing:

```bash
cargo run -p wonder-of-u-cli -- --storage-dir ./tmp/dev status
```

## TUI renderer snapshots

The TUI renderer is tested through deterministic frame snapshots in
`crates/wonder-of-u-tui/src/render.rs`. If you change layout, update the snapshot
expectations intentionally and keep cursor helpers in
`crates/wonder-of-u-cli/src/tui_runtime/helpers.rs` aligned with prompt geometry.

## Parity ledger

`docs/parity-ledger.csv` records the historical porting audit. Keep it only for
traceability; new feature work should be documented in README/docs and covered by
tests.

