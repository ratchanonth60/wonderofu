# Usage

`wonder-of-u` can run as an interactive terminal app or as a regular CLI.

## Interactive TUI

```bash
wonder-of-u
# or
wonder-of-u tui
```

The TUI includes:

- Slash autocomplete: type `/`
- Prompt history search: `Ctrl+R`
- Vim editing mode via `/vim`
- Model, theme, memory, and permission pickers
- Background task and notification panels
- Permission dialogs for tool execution
- `/setup` hub for provider and preference configuration
- Scrollable chat transcript with keyboard and mouse controls

## Chat history and prompt input

The transcript is the main AI conversation surface. Assistant output, tool/runtime
updates, and provider errors are shown inline in history so failures remain visible
after transient status text changes.

| Key | Effect |
| --- | --- |
| `Enter` | Submit the current prompt |
| `Shift+Enter` | Insert a newline for a multiline prompt |

The prompt can grow across lines, but its height is capped so the transcript remains
usable while composing longer messages.

## Setup hub (`/setup`)

Type `/setup` inside the TUI (or press Enter when the auto-open prompt appears on first
launch) to open the settings hub. The hub lists the following items:

| Item | What it does |
| --- | --- |
| **Provider API key** (`login`) | Two-step form: pick a provider (Anthropic, OpenAI, …), then type the API key. The key is masked with bullets while you type. |
| **API base override** (`api-base`) | Two-step form: pick a provider, then enter a custom base URL (e.g. a proxy or local endpoint). |
| **Copilot OAuth** (`copilot-oauth`) | Starts the GitHub device-code flow: displays the user code, opens the browser, then polls for the access token. |
| **Model** (`model`) | Opens the model picker (`/model`). |
| **Theme** (`theme`) | Opens the theme picker (`/theme`). |
| **Permissions** (`permissions`) | Opens the permission manager (`/permissions`). |
| **Memory** (`memory`) | Opens the memory viewer (`/memory`). |
| **Terminal setup** (`terminal-setup`) | Opens terminal settings (`/terminal-setup`). |
| **Keybindings** (`keybindings`) | Opens the keybindings viewer (`/keybindings`). |

The overlay closes with `Esc`.  If no provider is configured when the TUI starts,
the setup hub opens automatically so you can log in without leaving the TUI.

## Chat transcript scroll

When no overlay is open, the following controls scroll the message transcript:

| Key / action | Effect |
| --- | --- |
| `PageUp` | Scroll up one viewport page |
| `PageDown` | Scroll down one viewport page |
| `Ctrl+Home` | Jump to the very top of the transcript |
| `Ctrl+End` | Return to live tail (follow-tail mode) |
| Mouse wheel up / down | Scroll a few lines; only active when the pointer is over the transcript area |

> **Note** — scroll controls are suppressed while any overlay is active (picker,
> dialog, history search).  Those overlays handle their own navigation.

## Sidebar

The sidebar is a right-side panel that shows live session context alongside the chat
transcript.

| Section | What it shows |
| --- | --- |
| **Session** | Session ID, title, and start time |
| **Context** | Active working directory and git branch |
| **Providers** | Configured providers and auth state |
| **Status** | Current agent/tool execution state |
| **Controls** | Key hint reference |
| **Workspace** | Workspace root and loaded config |
| **Tasks** | Running and recently completed background tasks |

The active model is displayed as `model(provider)` with a `◈` prefix, e.g.
`◈ claude-3-5-sonnet(anthropic)`.

### Toggling the sidebar

| Method | Effect |
| --- | --- |
| `Ctrl+B` | Toggle sidebar on / off |
| `/sidebar` | Toggle sidebar (same as `Ctrl+B`) |
| `/sidebar on` | Force sidebar visible |
| `/sidebar off` | Force sidebar hidden |
| `/sidebar toggle` | Explicit toggle |

When the sidebar is visible the bottom status bar compacts to key hints only to
preserve vertical space.

### Narrow terminal behaviour

The sidebar is hidden automatically when the terminal is narrower than **100 columns**
and will not reappear until the terminal is wide enough again.

### Ephemeral state

Sidebar visibility is not persisted.  It resets to the default (auto-shown on wide
terminals) each time you start a new TUI session.

## Provider login

GitHub Copilot device flow:

```bash
wonder-of-u login --provider copilot
```

API-key providers:

```bash
wonder-of-u login --provider anthropic --api-key "$ANTHROPIC_API_KEY"
wonder-of-u login --provider openai --api-key "$OPENAI_API_KEY"
```

Inspect stored provider state:

```bash
wonder-of-u config show
wonder-of-u model show
```

## One-shot prompts

```bash
wonder-of-u prompt "Explain the workspace layout"
wonder-of-u prompt --provider openai --model gpt-4.1 "Write a release note"
wonder-of-u prompt --tools "Inspect this repository and summarize risks"
```

## Sessions

```bash
wonder-of-u session
wonder-of-u resume <session-id>
wonder-of-u rename <session-id> "New title"
wonder-of-u export <session-id> --format text
wonder-of-u compact <session-id> --keep-last 8
wonder-of-u clear <session-id>
```

## Project helpers

```bash
wonder-of-u files
wonder-of-u branch
wonder-of-u diff
wonder-of-u diff --staged
wonder-of-u slash /add-dir ../another-worktree
wonder-of-u slash /context
wonder-of-u slash /memory
```

## Extension surfaces

```bash
wonder-of-u mcp
wonder-of-u plugin
wonder-of-u reload-plugins
wonder-of-u skills
```

## Background work and fleet orchestration

### Monitoring background tasks

`/tasks` (bare) or `/tasks status` prints a live summary of every persisted
background task — shell tasks, local agent sub-processes, and fleet members:

```bash
wonder-of-u tasks              # summary view
wonder-of-u tasks list         # full list (most-recent first, capped at 20)
wonder-of-u tasks show <id>    # detail + log tail for one task
```

The sidebar **Tasks** panel (visible in the TUI when `Ctrl+B` is on) mirrors
this data inline without leaving the chat view.

### Cleaning up tasks

Remove a single finished task (its state file and output log are deleted):

```bash
wonder-of-u tasks remove <task-id>
# --force removes an active (pending/running) task without stopping it first
wonder-of-u tasks remove --force <task-id>
```

Bulk-prune all terminal tasks at once:

```bash
wonder-of-u tasks prune                  # removes completed + failed + cancelled
wonder-of-u tasks prune --completed      # removes only exit-code-0 tasks
wonder-of-u tasks prune --terminal       # explicit alias for the default behaviour
```

### Fleet: parallel sub-agent orchestration

`/fleet` orchestrates groups of local agent tasks under a single fleet run.

#### Direct prompt (quickest path)

Pass a freeform prompt and the fleet command queues parallel sub-agents
automatically — no subcommand needed:

```bash
wonder-of-u slash /fleet "refactor the auth module and add rate limiting"

# Multi-part prompts (separated by semicolons, numbered list, or bullets)
# queue one sub-agent per part:
wonder-of-u slash /fleet "extract utils; write docs; update changelog"
```

After queuing, drive execution with:

```bash
wonder-of-u slash /fleet reconcile <fleet_id>   # launch ready members
wonder-of-u slash /fleet wait <fleet_id>         # block until terminal
```

#### Steering an active fleet

Send a mid-run instruction to an in-progress fleet (rejected for terminal runs):

```bash
wonder-of-u slash /fleet steer <fleet_id> "focus only on the payment module"
```

The steering message is visible in `fleet show <fleet_id>` under `steering_count`
and `steering_latest`.

#### Monitoring and results

```bash
wonder-of-u slash /fleet status              # aggregate across all runs
wonder-of-u slash /fleet show <fleet_id>     # per-member task statuses + log tail
wonder-of-u slash /fleet results <fleet_id>  # aggregated output excerpts
```

#### Full fleet lifecycle example

```bash
# 1. Queue work via direct prompt
wonder-of-u slash /fleet "add OpenAPI docs to the auth service"
#    → fleet_id=<uuid>  queued_members=1  mode=direct_prompt

# 2. Run members
wonder-of-u slash /fleet reconcile <fleet_id>

# 3. Steer mid-run
wonder-of-u slash /fleet steer <fleet_id> "also cover the refresh-token endpoint"

# 4. Wait for completion
wonder-of-u slash /fleet wait <fleet_id> --timeout-secs 600

# 5. View results
wonder-of-u slash /fleet results <fleet_id> --include-logs

# 6. Clean up finished tasks
wonder-of-u tasks prune --completed
```

## Slash command transport

Every TUI slash command is also reachable from the command registry:

```bash
wonder-of-u slash /status
wonder-of-u slash /theme show
wonder-of-u slash /permissions
```

## Useful diagnostics

```bash
wonder-of-u doctor
wonder-of-u status
wonder-of-u slash /diagnostics
wonder-of-u --version
wonder-of-u slash /usage
wonder-of-u slash /cost
```
