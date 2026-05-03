# Configuration

## Storage directory

Most commands use a persistent storage directory for sessions, settings,
snapshots, transcripts, plugin catalogs, and provider credentials.

Override it for isolated runs:

```bash
wonder-of-u --storage-dir ./tmp/wonder-of-u status
```

This is useful for tests, demos, or multiple local profiles.

## Providers

### Copilot

```bash
wonder-of-u login --provider copilot
```

The CLI runs a GitHub device authorization flow and stores refresh metadata when
the provider returns it.

### Anthropic

```bash
export ANTHROPIC_API_KEY=...
wonder-of-u login --provider anthropic --api-key "$ANTHROPIC_API_KEY"
```

### OpenAI

```bash
export OPENAI_API_KEY=...
wonder-of-u login --provider openai --api-key "$OPENAI_API_KEY"
```

## Model selection

```bash
wonder-of-u model show
wonder-of-u model set --provider anthropic --model claude-3-7-sonnet-latest
wonder-of-u model set --provider openai --model gpt-4.1
```

The active provider/model is reflected in the TUI footer and session state.

## API base overrides

```bash
wonder-of-u config set-api-base --provider openai --api-base https://api.openai.com/v1
wonder-of-u config clear-api-base --provider openai
wonder-of-u config show
```

## TUI preferences

Inside the TUI, open the **setup hub** with `/setup` to access provider login,
model selection, theme, permissions, memory, terminal settings, and keybindings
from a single menu.  The hub opens automatically on first launch when no provider
is configured.

Individual slash commands are also available directly:

```text
/setup          open the settings hub (provider login, model, theme, …)
/theme          change the colour theme
/color          toggle colour rendering
/vim            toggle Vim editing mode
/brief          toggle concise response mode
/fast           toggle fast (low-latency) mode
/effort         set reasoning effort level
/permissions    open the permission manager
```

Most settings are persisted in snapshots/settings so resumed sessions keep the
same behavior.

