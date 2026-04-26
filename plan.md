# Rust implementation plan for `wonder-of-u`

## 1. Goal

Port the licensed TypeScript/TSX source tree in `claude-leak/` into an idiomatic Rust CLI/TUI application with first-release parity for:

- Interactive TUI/REPL.
- Main slash commands.
- Model provider/auth.
- File, shell, search, and web tools.
- Permission system.
- Session resume/export/history.
- MCP.
- Plugins.
- Skills.
- Agents/tasks/background workers.

The target TUI is **visual-close**: layout, flow, colors, dialogs, status/footer behavior, command UX, and keybindings should feel the same, but the Rust implementation should not blindly copy unique artwork, prose, or prompt text unless explicitly approved.

## 2. Current repository state

- Rust workspace:
  - `Cargo.toml` is a multi-crate workspace using `wonder-of-u-*` crate/package identifiers.
  - Implemented crates now cover CLI, core, storage, test support, agent, tools, MCP, plugins, skills, and TUI foundations.
  - Broad first-release slices already exist for command registry, session persistence, permissions, provider/auth selection, task persistence, MCP catalogs, plugin/skill catalogs, and TUI shell/status rendering.
- Provider runtime checkpoint:
  - `wonder-of-u-agent` now has real provider execution for `openai`, `anthropic`, and `copilot`; OpenAI/Anthropic use API keys, and Copilot uses OAuth plus a real `copilot_internal/v2/token` exchange before prompt execution.
  - `wonder-of-u prompt ...` can send a real non-interactive prompt, print the response, persist transcript/snapshot/cost state when `--storage-dir` is set, and now supports opt-in `--tools` tool-use orchestration for providers that implement structured tool calling.
  - `agents start local ...` now runs a real background prompt subprocess with persisted logs, pid/heartbeat tracking, reconcile, stop handling, and `--tools` enabled so provider-backed local agents can use the built-in tool registry.
  - `skills run <skill> ...` now executes real prompt-based skill invocations and reuses the same transcript/snapshot/cost persistence path; `skills run --tools` enables manifest-restricted built-in tool execution using each skill's `allowed_tools` list.
  - `plugin run <plugin> <command> ...` now executes trusted ready plugin command entries as one-shot subprocesses with auth/readiness checks, and plugin manifest commands can now also be resolved dynamically from slash-command lookup in the TUI/CLI slash transport.
  - `wonder-of-u tui` now launches a live interactive shell with prompt editing, slash commands, streamed provider-backed prompt turns, persisted session updates, and a bounded built-in tool-use loop when the selected provider supports structured tool calling.
  - Default no-command launch now enters the TUI in interactive terminals, while non-interactive runs still fall back to `doctor`.
  - `resume <id>` now routes into the live TUI in interactive terminals and keeps summary output for non-interactive use.
  - OpenAI now supports real structured tool-use orchestration in the live TUI with persisted `AssistantToolUse` and `ToolResult` messages executed through the native built-in tool registry.
  - Anthropic native models now support structured tool-use orchestration in the shared provider runtime, including persisted assistant tool-use batches and tool-result continuation rounds.
  - Copilot GPT-style and Claude-style models now both support the same structured tool-use loop in TUI and non-interactive prompt/agent flows, with Claude-style models using the Anthropic-compatible `/v1/messages` tool block protocol after the Copilot token exchange.
  - `wonder-of-u login --provider copilot` now runs a real GitHub device OAuth flow, persists OAuth expiry/refresh metadata when the provider returns it, and the runtime now refreshes expiring Copilot OAuth credentials before prompt execution when a refresh token is available.
  - Copilot prompt execution now supports both GPT-style and Claude-style Copilot models, including streaming in the live TUI.
  - TUI parity was extended with notice dialogs for task completions and permission blockers, inline allow/deny/resume permission flow with snapshot-backed resume support, confirm-before-exit behavior when the session has unsent input/history/tasks, resume-time reconstruction of transient UI state, real vim editing, Ctrl-R prompt-history recall/cycling, dynamic plugin slash-command execution, richer `/plan` flow parity (`/plan` enter/show, `/plan <prompt>` queued execution, and live `/plan open` editor suspend/resume handling), live `/clear` and `/compact` view reload parity, and `/model [selection]` shorthand that no longer requires the explicit `set` subcommand.
  - Session-scoped additional working directories now persist in `AppState`, flow into tool permission contexts, and are exposed through a real `/add-dir` command that mutates the live TUI session.
  - The previously missing `/context` and `/memory` command surfaces now exist in Rust: `/context` renders a session/snapshot-based context summary and opens a live notice dialog in the TUI, while `/memory` now supports a live picker plus `/memory open {project|user}` external-editor flows from both CLI and TUI.
  - Additional parity command surfaces now exist for `/cost`, `/stats`, `/vim`, `/copy`, `/init`, `/keybindings`, `/theme`, `/tag`, and `/version`; `/stats`, `/theme show`, `/version`, and `/keybindings` open live TUI notice/picker flows, `/copy` pulls recent assistant responses from snapshot/transcript storage with clipboard best-effort plus temp-file fallback, `/init` scaffolds project `CLAUDE.md`, `/keybindings open` creates a loadable override template, TUI theme selection is now session-persisted with real renderer theme changes, and `/tag` now toggles a persisted searchable session tag with live remove-confirmation flow in the TUI.
  - Still deferred in this area: deeper interactive parity (richer dialog/picker flows and remaining task/plugin sandbox/daemon UX mismatches).
- Reference source tree:
  - `claude-leak/` contains roughly 1,800+ TypeScript/TSX files.
  - Main areas: `commands`, `components`, `hooks`, `ink`, `tools`, `services`, `utils`, `state`, `tasks`, `bridge`, `server`, `skills`, `plugins`, `keybindings`, `screens`, `vim`, `voice`.
  - `package-lock.json` is effectively empty and no usable `package.json` was found, so this tree is reference material rather than a runnable package in the current repo.

## 2.1 Progress update and execution workflow

- Latest parity work completed:
  - Added `/hooks` config/view parity plus `/hooks open` editor flow.
  - Added `/privacy-settings` fallback/web flow.
  - Added `/usage` and hidden deprecated `/output-style`.
  - Added real session-scoped `/color` state wired into the Rust TUI footer/renderer.
  - Added `/brief` session parity:
    - empty `/brief` toggles concise-response mode for the current session,
    - `/brief show` opens a live notice dialog,
    - the setting persists in session snapshots and is visible in the TUI footer/status path,
    - prompt execution now injects a concise system prompt when brief mode is on,
    - the leak's stricter SendUserMessage-only / hidden-plain-text behavior is still called out as not fully replicated yet.
  - Added `/review` as a local queued-prompt flow that mirrors the leak's gh-based PR review semantics.
  - Added `/statusline` as a local queued-prompt setup flow instead of pretending the leak's remote statusline subagent exists in the Rust port.
  - Added `/upgrade` with browser handoff plus a TUI notice flow pointing at the Claude upgrade page and telling users to rerun `/login` afterward.
  - Added `/insights` as a local-storage-backed queued-prompt report flow using persisted session metadata instead of the leak's heavier remote/facet pipeline.
  - Added `/release-notes` with local `CHANGELOG.md` parsing when available and repository releases-page fallback otherwise.
  - Added `/feedback` plus `/bug` alias with repository-issues fallback notice flow.
  - Added `/effort` as real settings + session-state parity:
    - normalizes shorthand like `/effort high`,
    - persists the chosen effort level into agent settings,
    - restores and shows it in the TUI footer/status path,
    - opens a live `Effort` notice dialog,
    - explicitly notes that provider-specific inference mapping is still not wired yet.
  - Added truthful `/fast` parity:
    - empty `/fast` toggles fast mode and `/fast show` opens a live `Fast` notice dialog,
    - fast mode now persists in agent settings and session snapshots,
    - the ratatui footer/status flow now shows fast-mode state,
    - provider resolution remaps default model selection to real built-in fast models when a provider exposes one (`mini`/`haiku` style mappings today),
    - explicit model overrides still win,
    - the leak's entitlement/quota/cooldown/billing-aware fast-mode semantics are still explicitly out of scope until the Rust runtime can implement them honestly.
- TUI parity note:
  - The reference UI uses **Ink/React**, while `wonder-of-u` uses a custom Rust state/controller loop on top of **ratatui/crossterm**.
  - Because of that, parity work must focus on reproducing flow/state/dialog behavior instead of assuming the same component lifecycle or repaint model.
  - Any TUI that still feels "strange" should be treated as a parity bug, not as expected behavior.
  - Validation hardening also matters for parity work: cwd-sensitive CLI tests are now serialized so command parity changes do not leave the suite with unrelated flaky failures.
- Execution workflow from this point forward:
  - Update progress in this project-level `plan.md` at each meaningful milestone.
  - Use `dev` as the integration branch.
  - `main` now reflects the merged parity snapshot from `dev`.
  - Current feature branch: `feat/fast-mode-parity`.
  - For each new feature slice, branch from `dev` into `feat/<slice>`, finish the slice, then merge back into `dev`.
  - Keep files grouped by domain (`commands/status.rs` for informational commands, `commands/workflow.rs` for interactive workflow commands, `tui_runtime.rs` for shell/controller behavior) and avoid scattering feature logic across unrelated modules.
- Current high-priority parity queue:
  - Tighten ratatui parity for inline permission approvals, notice/picker behavior, repaint timing, and status/footer transitions that still feel off versus Ink.
  - Harden resume/live-state reconstruction so restored sessions preserve more interactive shell/controller context.
  - Continue auditing the remaining user-facing slash-command surfaces from `claude-leak/commands/*`, especially the ones that can be implemented as honest local flows rather than remote-only placeholders.
  - Only claim feature parity when the Rust path has real runtime behavior or an explicit fallback that matches the reference command semantics.

## 3. First-release scope and backlog

### In scope for first release

- Full interactive REPL with streaming messages, prompt input, message history, status/footer, dialogs, background task UI, and command handling.
- Main command registry and slash commands needed for everyday use:
  - `help`, `init`, `config`, `login`, `logout`, `model`, `status`, `doctor`, `exit`.
  - `resume`, `session`, `rename`, `tag`, `clear`, `compact`, `export`.
  - `add-dir`, `context`, `files`, `memory`, `copy`, `branch`, `diff`.
  - `plan`, `permissions`, `agents`, `tasks`, `skills`.
  - `mcp`, `plugin`, `reload-plugins`.
- Provider/auth:
  - Direct API key provider.
  - OAuth-style provider flow if required by product configuration.
  - Provider abstraction ready for Bedrock, Vertex, Azure/custom gateway.
- Tools:
  - Bash, file read/edit/write, glob, grep, web fetch/search, todo, ask-user, plan tools, task output/stop, agent tool, MCP resource list/read.
- Permissions:
  - Permission modes, rules, sources, prompt UI, path validation, shell safety checks.
- Session persistence:
  - JSONL transcript, metadata, cost/token tracking, paste store, resume/export/history.
- MCP:
  - Config loading, stdio transport first, tool/resource wrapping, permission integration.
- Plugins/skills:
  - Manifest parsing, local plugin loading, command/skill registration, bundled/user skills.
- Agents/tasks:
  - Local shell tasks, local agent/background tasks, output logs, status updates, cancellation.

### Backlog after first release

- GitHub App installation, PR review/comments, advanced security review automation.
- IDE integration, diagnostics, LSP recommendations beyond base LSP/tool support.
- Hooks beyond the minimum needed for tool/session integration.
- Remote/bridge modes, daemon/server modes, WebSocket/SSE remote session transports.
- Voice, desktop/mobile/chrome handoff, buddy/assistant modes.
- Internal/debug/dogfood commands, heapdump/perf/cache/reset tooling.

## 4. Reference files to map before implementation

### Entrypoint and initialization

- `claude-leak/main.tsx`: startup flow, feature gates, settings/auth/model initialization, command/tool assembly, session bootstrapping.
- `claude-leak/replLauncher.tsx`: launches app wrapper and REPL screen.
- `claude-leak/entrypoints/init.ts`: initialization helpers.
- `claude-leak/bootstrap/state.ts`: process/session bootstrap state.

### Commands

- `claude-leak/commands.ts`: built-in command registry, feature-gated commands, availability filtering, dynamic skills/plugins/workflows insertion.
- `claude-leak/types/command.ts`: command type model.
- `claude-leak/utils/processUserInput/processSlashCommand.tsx`: slash command parsing and dispatch.
- Command directories under `claude-leak/commands/`, especially:
  - `help`, `config`, `login`, `logout`, `model`, `status`, `doctor`.
  - `resume`, `session`, `clear`, `compact`, `export`, `rename`, `tag`.
  - `context`, `files`, `memory`, `copy`, `branch`, `diff`.
  - `plan`, `permissions`, `agents`, `tasks`, `skills`.
  - `mcp`, `plugin`, `reload-plugins`.

### Tools and permissions

- `claude-leak/Tool.ts`: tool trait/interface, tool context, permission context.
- `claude-leak/tools.ts`: tool registry and filtering.
- `claude-leak/services/tools/toolExecution.ts`: execution orchestration.
- `claude-leak/services/tools/StreamingToolExecutor.ts`: streaming execution.
- `claude-leak/types/permissions.ts`: permission modes/rules/decisions.
- `claude-leak/utils/permissions/permissions.ts`: permission decision engine.
- `claude-leak/utils/permissions/permissionSetup.ts`: permission mode/rule loading.
- `claude-leak/utils/permissions/pathValidation.ts`: workspace scope enforcement.
- `claude-leak/tools/BashTool/*`: shell validation, safety, progress UI.
- `claude-leak/tools/FileReadTool/*`, `FileEditTool/*`, `FileWriteTool/*`, `GlobTool/*`, `GrepTool/*`, `WebFetchTool/*`, `WebSearchTool/*`.

### TUI and REPL

- `claude-leak/screens/REPL.tsx`: main REPL event loop, turn state machine, queues, message integration.
- `claude-leak/ink/*`: custom terminal rendering system, key parsing, screen/cell grid, ANSI emission.
- `claude-leak/components/App.tsx`: app state wrappers.
- `claude-leak/components/PromptInput/PromptInput.tsx`: prompt input, queued commands, input modes.
- `claude-leak/types/textInputTypes.ts`: prompt/vim/queue types.
- `claude-leak/hooks/useVimInput.ts`, `claude-leak/vim/*`: vim mode state machine.
- `claude-leak/keybindings/*`: keybinding defaults, parser, resolver, schema.
- `claude-leak/components/Message.tsx` and `components/messages/*`: message renderers.
- `claude-leak/components/StatusLine.tsx`, `Spinner.tsx`, `TaskListV2.tsx`, `components/permissions/*`, `components/diff/*`, `components/design-system/*`.
- `claude-leak/context/modalContext.tsx`, `overlayContext.tsx`, `promptOverlayContext.tsx`, `QueuedMessageContext.tsx`.

### State, storage, auth, services

- `claude-leak/state/AppStateStore.ts`, `state/AppState.tsx`, `state/store.ts`: application state model and store pattern.
- `claude-leak/utils/sessionStorage.ts`: transcripts, metadata, paste store, session recovery.
- `claude-leak/types/logs.ts`: serialized log/message structures.
- `claude-leak/history.ts`: command history, pasted content expansion, reference parsing.
- `claude-leak/cost-tracker.ts`, `costHook.ts`: cost/token state.
- `claude-leak/services/api/client.ts`: model/provider client creation.
- `claude-leak/services/oauth/*`, `utils/auth.ts`, `utils/secureStorage/*`: auth, token refresh, secure storage.
- `claude-leak/utils/model/*`: model identifiers, aliases, context windows, cost tiers.
- `claude-leak/services/mcp/*`, `tools/MCPTool/*`: MCP config/client/tool/resource surfaces.
- `claude-leak/types/plugin.ts`, `services/plugins/*`, `utils/plugins/*`, `plugins/*`: plugin manifest/lifecycle/loading.
- `claude-leak/skills/*`: bundled/user/plugin/dynamic skills.
- `claude-leak/Task.ts`, `tasks/types.ts`, `tasks/*`, `tools/AgentTool/*`, `tools/Task*Tool/*`: agents and background tasks.

## 5. Proposed Rust workspace

Use a Rust workspace with clear crate boundaries:

- `crates/wonder-of-u-cli`
  - `clap` entrypoint, non-interactive invocation, command dispatch, exit codes.
- `crates/wonder-of-u-core`
  - App/session/message types, command trait, tool trait, permission model, feature flags, shared errors.
- `crates/wonder-of-u-tui`
  - `ratatui` + `crossterm` renderer, event loop, widgets, dialogs, message rendering, prompt input, keybindings, vim mode.
- `crates/wonder-of-u-agent`
  - Provider-neutral model client, streaming response loop, context assembly, tool-use loop, compaction, token/cost accounting.
- `crates/wonder-of-u-tools`
  - Built-in Bash/file/search/web/todo/plan/task/agent tools and permission checks.
- `crates/wonder-of-u-mcp`
  - MCP config, transports, client lifecycle, tool/resource wrapping.
- `crates/wonder-of-u-plugins`
  - Plugin manifests, trust/cache/reload lifecycle, command/skill registration.
- `crates/wonder-of-u-skills`
  - Bundled/user/plugin/dynamic skills and SkillTool integration.
- `crates/wonder-of-u-storage`
  - Config, transcript, metadata, paste store, cost store, secure storage abstraction, migrations.
- `crates/wonder-of-u-services`
  - Git helpers, notifications, update checks, integration scaffolding.
- `crates/wonder-of-u-test-support`
  - Fake providers, fake tools, terminal snapshot harness, temp project fixtures.

## 6. Dependency candidates

- CLI/config: `clap`, `serde`, `serde_json`, `serde_yaml`, `toml`, `schemars`, `dirs`, `figment` or a custom config loader.
- Async/runtime: `tokio`, `futures`, `tokio-util`, `async-trait`.
- TUI: `ratatui`, `crossterm`, `unicode-width`, `textwrap`, `ropey`, `syntect` or `tree-sitter-highlight`.
- HTTP/streaming: `reqwest`, `eventsource-stream`, `tokio-tungstenite` if WebSocket is promoted.
- Files/search: `ignore`, `globset`, `grep`, `regex`, `walkdir`, `notify`.
- Processes: `tokio::process`, `shell-words`, platform-specific process-group handling.
- Git/GitHub: shell-backed `git` first or `gix`; optional `octocrab`.
- Security/storage: `keyring`, `zeroize`, `secrecy`, encrypted fallback storage if required.
- Testing: `insta`, `assert_cmd`, `predicates`, `tempfile`, `wiremock`, `pretty_assertions`, `proptest`.
- Observability: `tracing`, `tracing-subscriber`.

## 7. Core data model

### App state

Implement a central `AppState` plus smaller selectors/substates:

- Settings/config.
- Message history.
- Current model and provider.
- Permission context.
- Current prompt input and input mode.
- Vim state.
- Queued commands.
- Modal/overlay stack.
- Notifications.
- Background tasks.
- Session metadata.
- Cost/token state.
- Remote connection fields as backlog-compatible placeholders.

Recommended pattern:

- `StateStore<T>` with `get`, `update`, and subscription methods.
- Internal `Arc<RwLock<AppState>>` or single-threaded event-loop ownership with channels for updates.
- Avoid holding locks across `.await`.
- Prefer event messages over direct concurrent mutation where possible.

### Message model

Represent messages as a Rust enum that covers at least:

- User text.
- User image/reference/file attachment placeholder.
- Assistant text.
- Assistant thinking/collapsible thinking.
- Assistant tool use.
- Tool result.
- Bash output.
- System.
- Progress/status.
- Command input/output.
- Hook result.
- Compact boundary.
- Agent/task messages.
- Permission/plan approval messages.

Every persisted message needs:

- Stable UUID.
- Session ID.
- Timestamp.
- CWD and optional Git branch.
- Message-specific payload.
- Version/schema marker.

### Command model

Define command variants matching the reference concepts:

- Prompt command: produces prompt/content for the model.
- Local command: executes Rust logic and returns structured output.
- TUI command: opens an interactive widget/dialog/flow.
- Non-interactive command: usable from command-line invocation.
- Resume entrypoint command: can reconstruct session/app state.

Fields:

- Name and aliases.
- Description/help.
- Argument parser.
- Availability requirements.
- Feature gates.
- Source: built-in, skill, plugin, workflow, MCP/dynamic.
- Hidden/internal marker.

### Tool model

Define a `Tool` trait with:

- Name and aliases.
- JSON schema or typed input schema.
- Description generation from context.
- Enablement check.
- Read-only/destructive/concurrency-safe metadata.
- Input validation.
- Permission check.
- Async execution with progress events.
- Structured result and optional context modifier.
- Source: native, MCP, plugin, skill/agent generated.

## 8. Command registry plan

### Registration flow

Implement `CommandRegistry` that loads commands in this order:

1. Bundled skills.
2. Built-in plugin skills.
3. User skill directory commands.
4. Workflow commands if enabled later.
5. Plugin commands.
6. Plugin skills.
7. Dynamic skills discovered during file/context operations.
8. Built-in commands.

Filter every command through:

- Availability requirements.
- Feature gates.
- Auth/provider state.
- Internal/debug visibility.
- Remote/session mode restrictions.

Dedupe by name/alias with deterministic precedence.

### First-release command groups

| Group           | Commands                                                    | Notes                                                   |
| --------------- | ----------------------------------------------------------- | ------------------------------------------------------- |
| Core            | `help`, `init`, `exit`, `doctor`, `status`                  | Must work before provider is configured where possible. |
| Auth/model      | `login`, `logout`, `model`, `config`                        | Drives provider selection and command availability.     |
| Session         | `resume`, `session`, `rename`, `clear`, `compact`, `export` | Depends on storage and message model.                   |
| Context/project | `add-dir`, `context`, `files`, `memory`, `copy`, `branch`, `diff` | Requires path scope and file/search tools.              |
| Workflow        | `plan`, `permissions`                                       | Tied to permission mode and plan-mode state.            |
| Extension       | `mcp`, `plugin`, `reload-plugins`, `skills`                 | Requires extension registries.                          |
| Agents/tasks    | `agents`, `tasks`                                           | Requires task manager and background UI.                |

### Slash command execution flow

1. Input starts with `/`.
2. Parse command name and args.
3. Resolve against registry names and aliases.
4. Check command availability and enablement.
5. Dispatch based on command kind.
6. Append command input/output messages.
7. Persist transcript entries when command affects session.
8. For prompt commands, enqueue a model turn.

## 9. Tool runtime and permission plan

### Tool registry

First-release native tools:

- Shell: Bash.
- Files: read, edit, write.
- Search: glob, grep.
- Web: fetch, search.
- Planning/tasks: todo write, enter plan mode, exit plan mode, task output, task stop, task create/get/update/list if task v2 is in scope.
- Agents: agent launch/resume, send message if needed by first-release agents.
- Interaction: ask user.
- Skills: skill catalog/invocation.
- MCP: list resources, read resources, MCP tool wrapper.

Feature-gated/backlog tools:

- PowerShell.
- Notebook edit.
- LSP.
- Worktree enter/exit.
- Browser/terminal capture.
- Cron/remote trigger/monitor/workflow.
- Team create/delete and advanced collaboration.

### Permission model

Implement:

- External modes: `default`, `acceptEdits`, `bypassPermissions`, `dontAsk`, `plan`.
- Internal modes if needed: `auto`, `bubble`.
- Rule behavior: allow, deny, ask.
- Rule source: policy, local, project, user, CLI arg, command, session runtime.
- Rule content: optional path/pattern/tool-specific constraint.
- Decision: allow, ask, deny with reason.

Decision reasons should include:

- Matched rule.
- Current mode.
- Hook decision.
- Classifier/safety check.
- Working directory/path scope.
- Sandbox override.
- Aggregated subcommand result.

### Rule resolution

1. Normalize tool name and aliases.
2. Load all applicable rules from policy/local/project/user/CLI/session.
3. Apply deterministic precedence with policy never weakened by lower sources.
4. Evaluate rule content against tool input.
5. Apply mode defaults.
6. Run tool-specific safety checks.
7. If ask is required, show permission dialog and persist runtime/session decision if user chooses always/never.
8. Record denials for repeated-denial handling.

### Tool execution flow

1. Model emits tool use or command invokes tool.
2. Tool executor resolves tool from filtered registry.
3. Validate schema/input.
4. Run permission check.
5. Run pre-tool hooks if included in first-release minimum.
6. Execute tool with progress channel.
7. Stream progress to REPL/task UI.
8. Run post-tool hooks if included.
9. Append structured tool result message.
10. Persist transcript.
11. Resume model loop with tool results.

### Shell safety

For Bash:

- Parse command intent as much as possible.
- Block or ask on destructive operations.
- Detect dangerous shell constructs, redirects, recursive deletes, privilege escalation, command substitution risk, and path escapes.
- Respect working directory and additional directories.
- Track stdout/stderr separately.
- Support cancellation and timeouts.
- Surface exact errors, not silent failures.

### File safety

For file tools:

- Normalize and canonicalize paths.
- Enforce CWD/additional-directory boundaries.
- Detect binary/large files and apply limits.
- Use atomic writes where possible.
- Show diffs for edits.
- Preserve user changes and avoid broad overwrites.

## 10. TUI/REPL plan

### Renderer architecture

Use Rust-native `ratatui`/`crossterm`, not a React clone. Recreate visible behavior with explicit widgets:

- Terminal wrapper:
  - Raw mode.
  - Alternate screen where appropriate.
  - Cursor show/hide.
  - Resize/focus events.
  - Bracketed paste.
  - Optional mouse support.
  - Safe terminal restore on panic/error.
- Render loop:
  - Single rendering owner.
  - Dirty-state or fixed tick rendering for animations.
  - No concurrent terminal writes.
  - Width-aware layout and truncation.
- UI layout:
  - Message viewport.
  - Prompt input area.
  - Status line.
  - Footer/hints.
  - Modal/overlay layer.
  - Background task panel.

### Main REPL event loop

Use one async event loop with `tokio::select!` over:

- Keyboard/paste events.
- Terminal resize/focus.
- Model stream events.
- Tool progress events.
- Task status events.
- Timer ticks for spinner/animation.
- Interrupt/shutdown signals.
- Extension reload events.

Turn states:

- Idle.
- Editing input.
- Command queued.
- Model request active.
- Tool permission pending.
- Tool executing.
- Streaming response.
- Interrupted.
- Completed.

Queued commands:

- `now`: interrupt/current-priority action.
- `next`: run after current tool or message segment.
- `later`: run after current turn.

Track origin: human, slash command, hook, task/agent, MCP/plugin.

### Prompt input

Implement:

- Prompt/chat mode.
- Bash/shell mode.
- Permission-pending input mode.
- Task notification mode.
- Multiline editing.
- Cursor movement by grapheme/word/line.
- History navigation.
- Reference parsing and pasted content expansion.
- Large paste handling.
- External editor integration if required for parity.
- Typeahead/suggestions if included in main command UX.

Use `ropey` or a similar rope structure for multiline editing.

### Keybindings

Implement a keybinding resolver with:

- Context-specific maps:
  - Global.
  - Prompt input.
  - Confirmation/dialog.
  - History search.
  - Vim insert/normal.
  - Message viewport.
  - Task panel.
- User override config.
- Reserved key validation for critical keys.
- Chords and modifier combinations where terminal support exists.
- Platform fallbacks for terminals lacking specific sequences.

Essential bindings:

- Ctrl-C/Ctrl-D interrupt or exit.
- Ctrl-L redraw.
- Ctrl-R prompt history recall/cycle.
- Ctrl-O transcript/expanded view toggle if in scope.
- Shift-Tab mode cycle if terminal supports.
- Ctrl-T task/todo toggle if in scope.
- Arrow/Page navigation.
- Esc modal cancel or vim normal mode, depending context.

### Vim mode

Implement:

- Insert and normal modes.
- Motions: `h`, `j`, `k`, `l`, `w`, `b`, `e`, `ge`, line boundaries.
- Operators: `d`, `c`, `y`.
- Counts for motions/operators.
- Find/till motions if present in reference behavior.
- Pending operator state and timeout.
- Boundary-safe operations on empty buffers and EOF.
- Repeat state if required by parity tests.

Add property tests and transition-table tests for operator/motion combinations.

### Message rendering

Implement a `Message` renderer dispatch for all first-release message variants:

- Assistant text.
- Assistant thinking/collapsed thinking.
- Assistant tool use.
- User text.
- User image/file placeholders.
- User bash output.
- Tool result.
- System.
- Progress.
- Task/agent.
- Plan/permission.
- Command input/output.
- Compact boundary.

Rendering requirements:

- Unicode width correctness.
- Word wrapping.
- Code block highlighting.
- Tool output truncation with expand affordance.
- Streaming text buffering.
- Collapsed groups.
- Virtual scrolling.
- Height cache invalidation on terminal width changes.

### Dialogs and modals

Implement modal stack:

- Active modal captures input first.
- Background content can be dimmed or visually de-emphasized.
- Esc/Ctrl-C close/cancel according to modal type.
- Tab cycles focus within fields.
- Enter confirms default action.

First-release modal/dialog types:

- Permission request.
- Diff/file edit approval.
- Model picker.
- Theme/config picker if included in main commands.
- Quick-open/context file picker.
- Confirm dialog.
- Plugin/MCP trust/approval dialog.
- Ask-user form/dialog.

### Status, footer, and background UI

Status line:

- Model name.
- Permission mode badge.
- Session name/title.
- Context/token percentage.
- CWD abbreviation.
- Optional cost indicator.

Footer:

- Input mode indicator.
- Keybinding hints.
- Queued command preview.
- Truncation based on width.

Background task UI:

- Tree/list of running agents/tasks.
- Spinner/progress/elapsed status.
- Nested subtask indentation.
- Output availability indicator.
- Task navigation keybindings.

## 11. State, storage, and persistence plan

### Directory layout

Use a configurable base directory, defaulting to the equivalent of the reference user config directory:

- `sessions/{session_id}.jsonl`: append-only transcript.
- `sessions/.metadata`: session index/metadata.
- `sessions/{session_id}.costs`: token/cost accounting.
- `pastes/{sha256}`: large pasted content.
- `tasks/{task_id}.log`: background task output.
- `plugins/`: user plugins.
- `skills/`: user skills.
- `mcp.json` or settings-backed MCP config.
- Settings files for user/project/local scopes.
- Secure storage references for credentials.

### Transcript format

Use JSONL/NDJSON:

- One serialized message envelope per line.
- Append-only writes.
- Include schema version for migrations.
- Include session ID, timestamp, cwd, optional git branch, entrypoint/version.
- Use fsync or durability policy for critical writes.
- Tolerate partial/corrupt trailing lines with warnings and recovery.

### Metadata and indexes

Maintain:

- Session title/name.
- Updated timestamp.
- Message count.
- Tags/custom title if supported.
- CWD/Git branch.
- Cost summary.
- Resume pointers and worktree state if in scope.

Write metadata atomically via temp file + rename.

### Paste store

- Hash large pasted content by SHA-256.
- Store raw content separately.
- Reference hashes in transcript messages.
- Expand references on resume/export.
- Enforce size limits.

### Cost/token tracking

- Track input/output/cache tokens per request.
- Track total session cost where provider pricing is known.
- Persist per-session cost file.
- Surface in status/cost commands.
- Do not block the main loop on cost writes.

### Resume flow

1. Resolve session ID or choose from session list.
2. Load transcript JSONL.
3. Validate and deserialize messages.
4. Reconstruct app state, cwd, branch, title, cost state.
5. Rebuild message height cache lazily.
6. If context window would overflow, trigger compaction/recovery behavior.
7. Render resumed history and accept new input.

### Export flow

- Export full transcript or selected range.
- Support plain text and structured JSON where required.
- Resolve paste references.
- Include tool outputs according to settings.
- Preserve chronological order.

## 12. Auth and model provider plan

### Provider abstraction

Define `ModelProvider` trait:

- Resolve model metadata.
- Count or estimate tokens.
- Stream chat/messages.
- Support tool definitions and tool-use blocks.
- Normalize provider errors.
- Expose rate-limit/quota information if available.

Initial providers:

- Direct API-key compatible provider.
- OAuth-backed provider if required.

Provider-ready architecture for:

- AWS Bedrock.
- Vertex AI.
- Azure Foundry.
- Custom gateway/proxy.

### Auth flow

Credential sources:

- Environment variables.
- Secure storage.
- Config file references.
- Interactive login.

OAuth-style flow:

1. Start local callback listener on ephemeral port.
2. Generate state and PKCE challenge.
3. Open browser or display URL.
4. Receive callback.
5. Exchange code for tokens.
6. Store refresh/access token securely.
7. Refresh on expiry.

Secure storage:

- macOS Keychain.
- Linux Secret Service.
- Windows Credential Manager/DPAPI.
- Encrypted fallback only if explicitly accepted.

### Model loop

1. Build request context from system prompt, memory, project context, messages, tools, MCP resources, and provider config.
2. Stream provider events.
3. Append assistant content chunks to current message.
4. Detect tool-use blocks.
5. Execute tools through permission-aware executor.
6. Feed tool results back to provider.
7. Repeat until end turn.
8. Persist messages and cost metrics.

## 13. MCP plan

### Config and scopes

Support:

- User/global MCP config.
- Project MCP config.
- Managed/policy config if needed.
- Session runtime additions where allowed.

Server types:

- `stdio` first.
- `sse`, `http`, `ws` as follow-up if in scope or already required by configs.
- In-process SDK only if required later.

### Lifecycle

1. Load and validate configs.
2. Create transport.
3. Initialize server and capture capabilities.
4. Cache tools/resources/prompts.
5. Register MCP tools into tool registry with namespacing.
6. Register resources into resource registry.
7. Reconnect with backoff on failure.
8. Surface server status/errors in UI and `/mcp`.

### Permissions

- MCP tools go through the same permission engine as native tools.
- Support server-wide allow/deny.
- Include server/tool name in permission prompt.
- Prevent server name collisions with native tools.

## 14. Plugins and skills plan

### Plugin model

Plugin manifest fields:

- Name, version, description.
- Commands.
- Skills.
- MCP servers.
- Hooks.
- Required permissions/trust level.

Scopes:

- Built-in.
- User.
- Project.
- Managed/enterprise.

Lifecycle:

1. Discover plugin directories.
2. Parse and validate manifests.
3. Check trust policy.
4. Load skills.
5. Load commands.
6. Load MCP server configs.
7. Register hooks if included.
8. Cache loaded plugin metadata.
9. Support reload and error reporting.

### Skill model

Skill definition:

- Name.
- Description.
- Input/output schema if structured.
- Prompt/system context.
- Allowed tools.
- Source and trust level.

Skill surfaces:

- Slash command when appropriate.
- SkillTool catalog for model use.
- Plugin-sourced skills.
- Bundled skills.
- User skill directory.
- Dynamic skills discovered during work.

## 15. Agents, tasks, and background workers plan

### Task model

Task types for first release:

- Local shell task.
- Local agent task.
- In-process teammate/worker if needed by agent parity.
- Local workflow task if needed by task UI.

Backlog:

- Remote agent.
- Monitor MCP.
- Dream/proactive/background inference.
- Scheduled/cron tasks.

Task status:

- Pending.
- Running.
- Completed.
- Failed.
- Killed/cancelled.

Task state fields:

- ID.
- Type.
- Status.
- Description.
- Start/end timestamps.
- Output log path.
- Output offset for incremental reads.
- Parent/child relation.
- Agent name/color/model if applicable.

### Task manager

Responsibilities:

- Spawn tasks.
- Stream output and progress through channels.
- Persist output append-only.
- Update app state.
- Cancel/kill tasks safely.
- Restore known task state on resume where applicable.
- Expose task list to TUI and commands.

### Agent tool

Implement:

- Built-in agent definitions.
- Custom agent definitions from disk.
- Agent tool filtering.
- Agent-specific permission restrictions.
- Background execution.
- Transcript/log capture.
- Resume/inspect output.

## 16. Implementation phases

### Phase 0: Feature inventory and acceptance matrix

- Build a parity matrix from the reference source:
  - Commands and aliases.
  - Tool names, schemas, and permission behavior.
  - Message variants.
  - TUI screens/dialogs/status/footer.
  - Keybindings and vim behavior.
  - Storage files and formats.
  - Provider/auth modes.
  - MCP/plugin/skill/agent/task surfaces.
- Mark each item as first-release or backlog.
- Define golden cases for TUI and transcript behavior.

### Phase 1: Workspace foundation

- Convert repo to workspace layout.
- Add core error/result type.
- Add tracing/logging.
- Add config directory resolver.
- Add feature flag abstraction.
- Add test support crate.
- Add CI-compatible commands for format, lint, build, and tests.

### Phase 2: Core types and storage

- Implement IDs and branded newtypes.
- Implement message enum and serialized envelopes.
- Implement AppState and store/update pattern.
- Implement session JSONL storage.
- Implement metadata, paste store, cost store.
- Implement settings schema and source precedence.
- Add migrations/versioning.

### Phase 3: Command and tool traits

- Implement command trait and command registry.
- Implement slash command parser.
- Implement tool trait and tool registry.
- Implement feature/availability filtering.
- Add first core built-in commands with placeholder UIs where dependent systems are not ready.
- Add schema validation for tool inputs.

### Phase 4: Permissions

- Implement permission modes.
- Implement rule loading and precedence.
- Implement path validation.
- Implement shell/file safety checks.
- Implement permission result model.
- Implement runtime/session rule updates.
- Add denial tracking.
- Add permission unit tests covering precedence and dangerous inputs.

### Phase 5: TUI foundation

- Implement terminal lifecycle and safe restore.
- Implement base layout: messages, prompt, status line, footer.
- Implement event loop skeleton.
- Implement text input.
- Implement key event normalization.
- Implement theme/color system.
- Add snapshot harness for terminal frames.

### Phase 6: Prompt input, keybindings, and vim

- Implement multiline editor.
- Implement history navigation/search.
- Implement keybinding resolver and user overrides.
- Implement vim insert/normal state machine.
- Implement operators/motions/counts.
- Implement paste/reference handling.
- Add property and snapshot tests.

### Phase 7: Message rendering and dialogs

- Implement message renderers for first-release message variants.
- Implement virtual scrolling and height cache.
- Implement streaming message updates.
- Implement modal stack.
- Implement permission, diff, picker, confirm, and ask-user dialogs.
- Implement spinner/progress/status/footer updates.
- Implement background task panel.

### Phase 8: Model provider and REPL loop

- Implement provider trait.
- Implement first provider.
- Implement auth credential resolution.
- Implement streaming request/response handling.
- Implement tool-use loop.
- Implement interruption/cancellation.
- Implement context assembly and compaction triggers.
- Persist messages and cost metrics.

### Phase 9: Built-in tools

- Implement Bash tool.
- Implement file read/edit/write.
- Implement glob/grep.
- Implement web fetch/search.
- Implement todo/plan/task tools.
- Implement ask-user tool.
- Implement tool result rendering.
- Add integration tests with fake projects and fake provider tool calls.

### Phase 10: First-release commands

- Implement core, auth/model, session, context/project, workflow, extension, and agents/tasks command groups.
- Ensure command output works in interactive and non-interactive contexts where applicable.
- Ensure help lists only enabled/available commands.
- Add command snapshot tests.

### Phase 11: MCP

- Implement MCP config parser.
- Implement stdio transport.
- Implement initialization and capability discovery.
- Register MCP tools/resources.
- Implement MCP resource list/read tools.
- Add status/errors to `/mcp` and TUI.
- Add fake MCP server integration tests.

### Phase 12: Plugins and skills

- Implement plugin manifest parsing and validation.
- Implement plugin discovery and trust prompts.
- Implement command and skill registration from plugins.
- Implement user/bundled skill loading.
- Implement SkillTool catalog.
- Implement reload flow and error reporting.

### Phase 13: Agents and background tasks

- Implement TaskManager.
- Implement local shell tasks.
- Implement local agent tasks.
- Implement task output logs and task output tool.
- Implement task cancellation.
- Implement agents/tasks commands and TUI panel.
- Add tests for output streaming and cancellation.

### Phase 14: Hardening and parity closure

- Complete parity matrix items marked first-release.
- Add cross-platform terminal smoke tests.
- Add transcript round-trip tests.
- Add permission/security regression tests.
- Add provider error handling tests.
- Add plugin/MCP isolation tests.
- Add packaging and release artifacts.

## 17. Acceptance criteria

### Functional

- `wonder-of-u` starts an interactive TUI from the current repo.
- Prompt input supports multiline editing, history, paste handling, slash commands, and vim mode.
- Main commands listed in first-release scope work and show helpful errors when unavailable.
- Streaming model responses render incrementally without blocking input/timer rendering.
- Tool use flows through permission checks and renders tool progress/result messages.
- Bash/file/search/web tools work inside allowed directories and reject unsafe operations correctly.
- Permission modes and rules behave consistently across commands, model tool use, MCP tools, and agent tasks.
- Sessions persist to JSONL and can be resumed/exported.
- MCP stdio servers can expose tools/resources through the Rust tool registry.
- Plugins can register commands/skills from validated manifests.
- Skills can be listed and invoked.
- Agents/tasks can run in background, stream output, show status, and be cancelled.

### TUI parity

- Layout includes message viewport, prompt input, status line, footer, dialogs, and background task UI.
- Visual style is close to the reference: spacing, colors, badges, progress/spinner behavior, and dialog flow.
- Keybindings match the reference for global, input, dialog, search, task, and vim contexts.
- Messages wrap correctly at terminal width and handle Unicode width.
- Resize, Ctrl-C, Esc, paste, and focus changes are handled without corrupting terminal state.
- Permission and diff dialogs are clear and keyboard-navigable.

### Storage and recovery

- Transcript append is crash-tolerant enough to recover all complete lines.
- Metadata writes are atomic.
- Large paste references resolve on resume/export.
- Corrupt trailing transcript lines are reported and skipped without blocking valid history.
- Session resume reconstructs messages, cwd/branch metadata, cost state, and visible history.

### Security and reliability

- No tool bypasses the permission engine.
- Path validation prevents escaping allowed roots.
- Shell safety catches high-risk commands and asks or denies according to mode/rules.
- Plugin and MCP tools are namespaced and permission-scoped.
- Secrets are stored through secure storage and never written into transcripts/logs.
- Errors are surfaced explicitly; no silent success-shaped fallbacks.

## 18. Test strategy

### Unit tests

- Command parsing and alias resolution.
- Command availability and feature gating.
- Tool schema validation.
- Permission precedence and decision reasons.
- Path normalization and boundary enforcement.
- Bash safety classification.
- Text editor operations.
- Vim transitions/operators/motions.
- Message serialization/deserialization.
- Settings source precedence.

### Snapshot/golden tests

- Help output.
- Command lists by availability state.
- TUI frames for:
  - empty REPL,
  - active prompt,
  - streaming assistant text,
  - tool use/result,
  - permission dialog,
  - diff dialog,
  - model picker,
  - task panel,
  - resumed session.
- Transcript export output.

### Integration tests

- Fake provider streaming text and tool-use events.
- Fake tool executor producing progress and results.
- Session save/resume/export round trip.
- MCP fake stdio server with tool and resource.
- Plugin manifest load/reload failure and success.
- Skill registration and invocation.
- Background task spawn/output/cancel.

### Cross-platform checks

- Linux/macOS/Windows terminal setup and restore.
- Key event normalization differences.
- Secure storage backends.
- Process cancellation behavior.
- Path canonicalization and separators.

## 19. Risks and mitigations

| Risk                                       | Impact               | Mitigation                                                                                     |
| ------------------------------------------ | -------------------- | ---------------------------------------------------------------------------------------------- |
| TUI behavior is broad and subtle           | Parity gaps          | Build a parity matrix and snapshot every major screen/dialog early.                            |
| Vim mode edge cases                        | Input bugs           | Use explicit transition table plus property tests for operators/motions.                       |
| Concurrent state deadlocks                 | Hung REPL            | Prefer single event-loop ownership and channels; avoid nested locks and locks across await.    |
| Terminal corruption                        | Bad UX               | Single renderer owner, panic-safe restore, no direct stdout writes while TUI active.           |
| Permission precedence bugs                 | Security issue       | Exhaustive rule-precedence tests and deny-by-default behavior for ambiguous high-risk actions. |
| Shell classifier misses dangerous commands | Security issue       | Layer classifier with explicit deny patterns, path scope checks, and user prompts.             |
| Transcript corruption                      | Lost history         | Append JSONL, tolerate partial trailing lines, atomic metadata writes.                         |
| Large transcript performance               | Lag/OOM              | Virtual scrolling, lazy loading, height cache, compaction thresholds.                          |
| MCP server hangs                           | Blocked UI/tool loop | Transport timeouts, cancellation, backoff, isolated subprocess management.                     |
| Plugin trust boundary                      | Security issue       | Manifest validation, explicit trust prompts, permission-scoped capabilities.                   |
| Provider API drift                         | Runtime failures     | Provider abstraction, fake provider tests, error normalization.                                |
| Cross-platform key/storage differences     | Platform bugs        | Test platform-specific modules separately and keep fallback behavior explicit.                 |

## 20. Todo list

1. Create full feature parity matrix from reference files.
2. Convert Rust crate to workspace.
3. Implement core types, state store, IDs, errors, settings, and feature flags.
4. Implement session storage, metadata, paste store, cost store, and migrations.
5. Implement command and tool traits plus registries.
6. Implement permission model, rule loading, path validation, and shell/file safety.
7. Implement terminal lifecycle, TUI layout, event loop, and snapshot harness.
8. Implement prompt input, keybindings, history, paste/reference handling, and vim mode.
9. Implement message rendering, modal stack, dialogs, status/footer, spinner, and task panel.
10. Implement provider/auth layer and streaming model loop.
11. Implement core tools and tool execution pipeline.
12. Implement first-release slash commands.
13. Implement MCP config/client/tools/resources.
14. Implement plugin and skill loading/registration.
15. Implement agents/tasks/background workers.
16. Complete parity tests, integration tests, security hardening, and packaging.

## 21. Notes for implementation

- Port behavior and semantics, not TypeScript structure one-to-one.
- Prefer Rust enums and traits over React-like dynamic component trees.
- Keep the permission engine central; tools, commands, MCP, plugins, and agents must all route through it.
- Keep terminal rendering single-owner to prevent corruption.
- Build fake-provider/fake-tool infrastructure early so REPL and TUI can be developed without real API calls.
- Treat first-release backlog boundaries strictly to avoid blocking the core port on remote/voice/IDE/internal features.
