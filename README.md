# wonder-of-u

`wonder-of-u` is a fast Rust agent CLI with an interactive terminal UI, persistent
sessions, provider login, tool execution, plugins, skills, and background tasks.

It is built as a multi-crate Rust workspace and ships one binary:

```bash
wonder-of-u
```

## Highlights

| Area | What you get |
| --- | --- |
| Terminal UI | ratatui/crossterm shell with rounded panels, slash autocomplete, model/theme/memory pickers, Vim mode, history search, notifications, and permission dialogs |
| Providers | Anthropic, OpenAI, and GitHub Copilot OAuth flows |
| Sessions | JSONL-backed transcripts, snapshots, resume, export, rename, tag, clear, and compact |
| Tools | Native file/search/shell-style tool runtime with permission checks and path validation |
| Extensions | MCP stdio discovery, plugin manifests, local skills, agents, and background task management |
| Automation | Non-interactive `prompt`, local agent subprocesses, review/security-review prompt flows, diagnostics, usage/cost/status commands |

## Install

### One-command local install

```bash
./install.sh
```

By default, the script installs `wonder-of-u` into the first usable prefix:

1. `$WONDER_OF_U_INSTALL_PREFIX/bin`
2. `$HOME/.local/bin`
3. `/usr/local/bin` when writable

Install somewhere specific:

```bash
./install.sh --prefix "$HOME/.local"
```

Uninstall:

```bash
./install.sh --uninstall
```

More details: [docs/INSTALLATION.md](docs/INSTALLATION.md).

## Quick start

```bash
# Start the interactive TUI
wonder-of-u

# Run startup diagnostics
wonder-of-u doctor

# Login with a provider
wonder-of-u login --provider copilot
wonder-of-u login --provider anthropic --api-key "$ANTHROPIC_API_KEY"
wonder-of-u login --provider openai --api-key "$OPENAI_API_KEY"

# Run a one-shot prompt
wonder-of-u prompt "Summarize this repository"

# Resume an interactive session
wonder-of-u session list
wonder-of-u resume <session-id>
```

Inside the TUI, type `/` to open slash-command autocomplete.

## Common commands

| Command | Purpose |
| --- | --- |
| `wonder-of-u` / `wonder-of-u tui` | Launch the live TUI |
| `wonder-of-u prompt "..."` | Execute a non-interactive model prompt |
| `wonder-of-u status` | Show runtime, storage, provider, and session status |
| `wonder-of-u doctor` | Run lightweight environment diagnostics |
| `wonder-of-u model show` | Show active provider/model and built-in options |
| `wonder-of-u config show` | Inspect persisted provider settings |
| `wonder-of-u session` | Manage persisted sessions |
| `wonder-of-u mcp` | Inspect MCP config and server discovery |
| `wonder-of-u plugin` | Inspect plugin manifests and trust state |
| `wonder-of-u skills` | List and run local/plugin skills |
| `wonder-of-u agents` / `wonder-of-u tasks` | Manage persisted background work |

See [docs/USAGE.md](docs/USAGE.md) for a fuller walkthrough.

## Configuration

Storage defaults to the platform config/data directory used by the CLI. For
isolated runs and tests, pass:

```bash
wonder-of-u --storage-dir ./tmp/wonder-of-u status
```

Provider credentials can be saved with `login` or supplied through environment
variables:

```bash
export ANTHROPIC_API_KEY=...
export OPENAI_API_KEY=...
```

See [docs/CONFIGURATION.md](docs/CONFIGURATION.md).

## Development

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p wonder-of-u-tui
cargo test -p wonder-of-u-tools
cargo test -p wonder-of-u-cli -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
```

Workspace layout:

| Crate | Responsibility |
| --- | --- |
| `wonder-of-u-cli` | CLI entrypoint, slash commands, TUI runtime |
| `wonder-of-u-tui` | Renderer, prompt widgets, panels, notifications |
| `wonder-of-u-agent` | Provider resolution and model execution |
| `wonder-of-u-core` | Shared state, messages, command registry, errors |
| `wonder-of-u-storage` | Session metadata, transcripts, snapshots |
| `wonder-of-u-tools` | Native tool implementations and permission context |
| `wonder-of-u-mcp` | MCP config/discovery support |
| `wonder-of-u-plugins` | Plugin manifest catalog/runtime glue |
| `wonder-of-u-skills` | Skill catalog and invocation support |

See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Documentation

- [Installation](docs/INSTALLATION.md)
- [Usage](docs/USAGE.md)
- [Configuration](docs/CONFIGURATION.md)
- [Development](docs/DEVELOPMENT.md)
- [Changelog](CHANGELOG.md)

## License

MIT
