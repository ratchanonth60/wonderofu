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

## Background work

```bash
wonder-of-u agents
wonder-of-u tasks
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
