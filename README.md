# wonder-of-u

A Rust port of the Claude CLI with full ratatui TUI parity.

## Features
- Interactive TUI shell powered by **ratatui** / crossterm
- Full slash-command suite (60+ commands)
- Multi-provider support: Anthropic, OpenAI, Copilot OAuth
- Session persistence, resume, export
- Permission engine with path validation and shell safety
- MCP (Model Context Protocol) stdio transport
- Plugins, skills, and background agents/tasks
- Vim editing mode, Ctrl+R history search, picker overlays

## Prerequisites
- Rust toolchain ≥ 1.75

## Build
```bash
cargo build --release
# binary is at target/release/wonder-of-u
```

## Install
```bash
cargo install --path crates/wonder-of-u-cli
```

## Usage
```bash
wonder-of-u
wonder-of-u prompt "hello"
wonder-of-u resume <session-id>
wonder-of-u --help
```

## Configuration
- Set `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or run `wonder-of-u login --provider copilot`
- Config dir: `~/.config/wonder-of-u/`

## Development
```bash
cargo test --workspace
cargo fmt --all
cargo clippy --workspace
```

## License
MIT
