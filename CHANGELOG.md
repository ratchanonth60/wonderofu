# Changelog

All notable changes are documented here.

## [Unreleased]

### Added
- `install.sh` for local release/debug installs and uninstalling the binary.
- User-facing documentation for installation, usage, configuration, and development.
- Ctrl+R incremental history search overlay
- Picker preview panel for model/theme/memory/permission choosers
- Task panel lifecycle polish with auto-dismiss notices
- Single-owner modal/overlay coordination (Esc routing)
- `/ide` honest browser-handoff notice
- `/brief` strict system-prompt injection parity
- Provider-specific effort/reasoning parameter mapping
- Transcript round-trip and recovery tests
- Permission/path-validation regression tests
- MCP namespace isolation and plugin reload tests
- `/setup` hub overlay: provider API-key entry, API-base override, Copilot OAuth device-code flow, model/theme/permission/memory/terminal/keybindings items; auto-opens on first launch when no provider is configured
- Chat transcript scroll: `PageUp`/`PageDown` (one page), `Ctrl+Home`/`Ctrl+End` (jump to top/live tail), mouse wheel over transcript area; all scroll controls suppressed while any overlay is active
- Docs: README, USAGE.md, and CONFIGURATION.md updated for TUI setup hub and chat scroll UX

## [0.1.0] — Initial parity release

### Added
- Full ratatui TUI shell
- 60+ slash commands
- Anthropic, OpenAI, Copilot OAuth providers with streaming
- Permission engine with path validation and shell safety
- Session JSONL persistence, resume, export, tag, rename
- MCP stdio transport, plugin manifest loading, skill execution
- Background agents/tasks with cancellation
- Vim editing mode
