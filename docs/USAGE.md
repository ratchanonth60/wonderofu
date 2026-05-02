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
