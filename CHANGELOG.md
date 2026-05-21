# Changelog

All notable changes are documented here.

## [Unreleased]

## [0.1.3] — 2026-05-21

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
- Regression tests for TUI setup/scroll/OAuth/provider-form (todo: tui-setup-scroll-tests ✓ done): `action_for_item_id` all 9 known IDs + unknown fallback; `SetupOverlayState::clamp_selection`; `ProviderFormState::selected_display_name` + `display_value` bullet-masking; `CopilotOAuthFlowState` Debug redaction; `ConsoleOAuthFlowView::instructions` with/without expiry; `OnboardingView::render_lines` completed/pending/footer/empty
- Expanded command parity with `/x402`, `statusline`, command aliases, setup/status honesty, and reference parity tests.
- Live TodoTaskStore-backed sidebar todos with `todos.md` fallback.
- MCP project `.mcp.json` discovery/merge, environment expansion, and reusable MCP session pool.
- Native Gemini and Vertex Gemini tool-use request/response handling.
- Advanced Vim prompt editing: yank, paste, undo/redo, visual selection, find, and repeat-find.
- Hook condition evaluation with explicit reporting for unsupported Prompt, Agent, and HTTP actions.
- Skill runtime prompt composition for bundled, local, and plugin-provided skills.
- Thinking/loading animation now continues during blocking provider requests, streaming setup, and tool execution.

### Changed
- Provider tool-use capability helpers now match the actual runtime wire-protocol support.
- TUI prompt hotkeys include model picker, thinking toggle, fast-mode toggle, and tool-output expansion.
- Plugin command execution and skill catalogs now surface clearer runtime/trust/readiness metadata.

### Fixed
- Prevented the thinking spinner from freezing by running blocking provider/tool work behind a worker-channel progress boundary.
- Tightened MCP resource dispatch, content limits, env handling, and project/global config precedence.
- Improved hook, permission, command, and provider tests to cover parity edge cases.

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
