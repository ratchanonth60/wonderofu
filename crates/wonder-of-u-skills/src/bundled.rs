use std::path::PathBuf;

use crate::manifest::SkillManifest;
/// Represents bundled skill
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundledSkill {
    /// Stores the manifest
    pub manifest: SkillManifest,
    /// Stores the prompt
    pub prompt: String,
}

impl BundledSkill {
    /// Handles inline — registers a bundled skill with an explicit slash command.
    #[must_use]
    pub fn inline(
        name: impl Into<String>,
        description: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        let name_str: String = name.into();
        let slash_cmd = name_str.clone();
        Self {
            manifest: SkillManifest {
                schema_version: 1,
                name: name_str,
                description: description.into(),
                prompt: None,
                prompt_path: Some(PathBuf::from("bundled-inline")),
                allowed_tools: vec!["glob".into(), "grep".into(), "file_read".into()],
                slash_command: Some(slash_cmd),
                slash_aliases: vec![],
            },
            prompt: prompt.into(),
        }
    }

    /// Handles inline without a slash command (for non-user-invocable skills).
    #[must_use]
    pub fn inline_no_command(
        name: impl Into<String>,
        description: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            manifest: SkillManifest {
                schema_version: 1,
                name: name.into(),
                description: description.into(),
                prompt: None,
                prompt_path: Some(PathBuf::from("bundled-inline")),
                allowed_tools: vec!["glob".into(), "grep".into(), "file_read".into()],
                slash_command: None,
                slash_aliases: vec![],
            },
            prompt: prompt.into(),
        }
    }
}

const LOREM_IPSUM_PROMPT: &str = r#"# Lorem Ipsum Generator

Generate placeholder text using single-token English words for token-accurate testing.

Generate exactly the number of tokens requested. Use common English words that each tokenize as a single token: articles (the, a, an), pronouns (I, you, he, she, it, we, they), prepositions (in, on, at, to, for, of, with, by), adjectives (big, small, new, old, good, bad, long, short), and nouns (day, time, way, man, year, hand, part, place, case, week, point, house, world, room, fact, lot, right, thing, kind, door, room).

Produce the text as a single paragraph. Do not number or label the output — just output the plain text."#;

const SIMPLIFY_PROMPT: &str = r#"# Simplify: Code Review and Cleanup

Review all changed files for reuse, quality, and efficiency. Fix any issues found.

## Phase 1: Identify Changes

Run `git diff` (or `git diff HEAD` if there are staged changes) to see what changed. If there are no git changes, review the most recently modified files that the user mentioned or that you edited earlier in this conversation.

## Phase 2: Launch Three Review Agents in Parallel

Use the agent_tool to launch all three agents concurrently in a single message. Pass each agent the full diff so it has the complete context.

### Agent 1: Code Reuse Review
For each change, search for existing utilities and helpers that could replace newly written code. Look for similar patterns elsewhere in the codebase.

### Agent 2: Quality Review
Check for: error handling gaps, missing edge cases, unnecessary complexity, dead code, and naming clarity.

### Agent 3: Efficiency Review
Look for performance issues: unnecessary allocations, redundant I/O, slow algorithms, missing caches.

## Phase 3: Apply Fixes

After all three agents report back, apply the suggested improvements. Prefer surgical edits over full rewrites."#;

const SKILLIFY_PROMPT: &str = r#"# Skillify

You are capturing this session's repeatable process as a reusable skill.

## Goal

Create a skill definition that encodes the step-by-step process used in this conversation so it can be re-run automatically in future sessions with minimal input.

## Steps

1. Summarize the task the user asked you to do in 1–2 sentences.
2. List the exact sequence of actions you took (tool calls, decisions, outputs).
3. Identify what inputs would vary between runs (file paths, names, parameters).
4. Write a skill prompt that parameterises those inputs using `{{variableName}}` placeholders.
5. Suggest a short slash-command name for the skill (lowercase, hyphenated).
6. Output the complete skill YAML frontmatter + prompt body, ready to save as `.claude/skills/<name>.md`."#;

const STUCK_PROMPT: &str = r#"# /stuck — diagnose frozen/slow Claude Code sessions

The user thinks another Claude Code session on this machine is frozen, stuck, or very slow.

## Steps

1. List all Claude Code processes: `ps -axo pid=,pcpu=,rss=,etime=,state=,comm=,command= | grep -E '(claude|cli)' | grep -v grep`
2. For each suspicious process (high CPU ≥90%, state D/T/Z, RSS ≥4GB):
   - Check child processes: `pgrep -lP <pid>`
   - Sample CPU twice 2s apart to confirm it's not transient
3. Report: PID, CPU%, RSS, uptime, state, and a 1-sentence diagnosis
4. Suggest: kill, wait, or investigate further"#;

const BATCH_PROMPT: &str = r#"# Batch: Parallel Work Orchestration

You are orchestrating a large, parallelizable change across this codebase.

## Phase 1: Research and Plan (Plan Mode)

Enter plan mode, then:

1. **Understand the scope.** Launch one or more subagents to deeply research what the instruction touches. Find all the files, patterns, and call sites that need to change. Understand the existing conventions so the migration is consistent.

2. **Decompose into independent units.** Break the work into 5–30 self-contained units. Each unit must:
   - Be independently implementable in an isolated git worktree (no shared state with sibling units)
   - Be mergeable on its own without depending on another unit's PR landing first
   - Be roughly uniform in size (split large units, merge trivial ones)

3. **Determine the e2e test recipe.** Figure out how a worker can verify its change actually works end-to-end. Look for browser automation, CLI verification, or dev-server patterns. If you cannot find a concrete e2e path, ask the user.

4. **Write the plan.** Include a summary of research findings, a numbered list of work units, the e2e test recipe, and the exact worker instructions for each agent.

5. Present the plan for approval before proceeding.

## Phase 2: Spawn Workers (After Plan Approval)

Spawn one background agent per work unit using isolation: "worktree" and run_in_background: true. Launch all in a single message so they run in parallel. Each agent prompt must be fully self-contained.

After each worker finishes:
1. **Simplify** — invoke the `simplify` skill to review and clean up changes.
2. **Run unit tests** — run the project's test suite.
3. **Test end-to-end** — follow the e2e test recipe from the coordinator's prompt.
4. **Commit and push** — commit all changes with a clear message, push the branch, and create a PR with `gh pr create`.
5. **Report** — end with a single line: `PR: <url>` so the coordinator can track it.

## Phase 3: Track Progress

Render a status table as background-agent completion notifications arrive, parsing the `PR: <url>` line from each agent's result. Keep a brief failure note for any agent that did not produce a PR."#;

const LOOP_PROMPT: &str = r#"# /loop — schedule a recurring prompt

Parse the input into `[interval] <prompt…>` and schedule it as a recurring cron job.

## Parsing (in priority order)

1. **Leading token**: if the first whitespace-delimited token matches `^\d+[smhd]$` (e.g. `5m`, `2h`), that's the interval; the rest is the prompt.
2. **Trailing "every" clause**: otherwise, if the input ends with `every <N><unit>` or `every <N> <unit-word>`, extract that as the interval and strip it from the prompt.
3. **Default**: otherwise, interval is `10m` and the entire input is the prompt.

If the resulting prompt is empty, show usage `/loop [interval] <prompt>` and stop.

## Interval → cron

| Interval pattern     | Cron expression   | Notes                                     |
|----------------------|-------------------|-------------------------------------------|
| `Nm` where N ≤ 59    | `*/N * * * *`     | every N minutes                           |
| `Nm` where N ≥ 60    | `0 */H * * *`     | round to hours (H = N/60)                |
| `Nh` where N ≤ 23    | `0 */N * * *`     | every N hours                             |
| `Nd`                 | `0 0 */N * *`     | every N days at midnight                  |
| `Ns`                 | treat as `ceil(N/60)m` | cron minimum granularity is 1 minute |

If the interval doesn't cleanly divide its unit, pick the nearest clean interval and tell the user.

## Action

1. Schedule the recurring cron job with the parsed interval and prompt.
2. Briefly confirm: what's scheduled, the cron expression, the human-readable cadence, and that recurring tasks auto-expire after 30 days.
3. Then immediately execute the parsed prompt now — don't wait for the first cron fire."#;

const REMEMBER_PROMPT: &str = r#"# Memory Review

## Goal
Review the user's memory landscape and produce a clear report of proposed changes, grouped by action type. Do NOT apply changes — present proposals for user approval.

## Steps

### 1. Gather all memory layers
Read CLAUDE.md and CLAUDE.local.md from the project root (if they exist). Your auto-memory content is already in your system prompt — review it there. Note which team memory sections exist, if any.

### 2. Classify each auto-memory entry
For each substantive entry in auto-memory, determine the best destination:

| Destination | What belongs there | Examples |
|---|---|---|
| **CLAUDE.md** | Project conventions and instructions for Claude that all contributors should follow | "use bun not npm", "API routes use kebab-case", "test command is bun test", "prefer functional style" |
| **CLAUDE.local.md** | Personal instructions for Claude specific to this user, not applicable to other contributors | "I prefer concise responses", "always explain trade-offs", "don't auto-commit", "run tests before committing" |
| **Team memory** | Org-wide knowledge that applies across repositories (only if team memory is configured) | "deploy PRs go through #deploy-queue", "staging is at staging.internal", "platform team owns infra" |
| **Stay in auto-memory** | Working notes, temporary context, or entries that don't clearly fit elsewhere | Session-specific observations, uncertain patterns |

**Important distinctions:**
- CLAUDE.md and CLAUDE.local.md contain instructions for Claude, not user preferences for external tools
- Workflow practices (PR conventions, merge strategies, branch naming) are ambiguous — ask whether they're personal or team-wide
- When unsure, ask rather than guess

### 3. Identify cleanup opportunities
Scan across all layers for:
- **Duplicates**: Auto-memory entries already captured in CLAUDE.md or CLAUDE.local.md → propose removing from auto-memory
- **Outdated**: CLAUDE.md or CLAUDE.local.md entries contradicted by newer auto-memory entries → propose updating the older layer
- **Conflicts**: Contradictions between any two layers → propose resolution, noting which is more recent

### 4. Present the report
Output a structured report grouped by action type:
1. **Promotions** — entries to move, with destination and rationale
2. **Cleanup** — duplicates, outdated entries, conflicts to resolve
3. **Ambiguous** — entries where you need the user's input on destination
4. **No action needed** — brief note on entries that should stay put

If auto-memory is empty, say so and offer to review CLAUDE.md for cleanup.

## Rules
- Present ALL proposals before making any changes
- Do NOT modify files without explicit user approval
- Do NOT create new files unless the target doesn't exist yet
- Ask about ambiguous entries — don't guess"#;

const VERIFY_PROMPT: &str = r#"# Verify: Confirm a Code Change Works End-to-End

Verify that the most recent code change does what it should by running the app or tests.

## Steps

1. **Identify what changed.** Run `git diff HEAD~1 HEAD --name-only` (or `git diff --name-only` for staged changes) to see which files were modified.

2. **Understand the intent.** Read the changed files and the commit message (if any) to understand what the change is supposed to do.

3. **Choose a verification strategy** based on what changed:
   - **Unit/integration tests** — run the relevant test suite (e.g. `cargo test`, `npm test`, `pytest`)
   - **CLI smoke test** — if a CLI command changed, run it with representative arguments
   - **Server endpoint** — if an API changed, start the server and curl the affected endpoint
   - **UI flow** — if UI changed, describe the manual steps needed to verify visually

4. **Execute verification.** Run the chosen strategy. If tests fail, report which ones and why.

5. **Report the result.** Summarize:
   - What was changed
   - How it was verified
   - Whether it passed or failed
   - Any issues found"#;

const KEYBINDINGS_PROMPT: &str = r#"# Keybindings Help

Create or modify `~/.claude/keybindings.json` to customize keyboard shortcuts.

## CRITICAL: Read Before Write

**Always read `~/.claude/keybindings.json` first** (it may not exist yet). Merge changes with existing bindings — never replace the entire file.

- Use **Edit** tool for modifications to existing files
- Use **Write** tool only if the file does not exist yet

## File Format

```json
{
  "$schema": "https://www.schemastore.org/claude-code-keybindings.json",
  "bindings": [
    {
      "context": "Chat",
      "bindings": {
        "ctrl+e": "chat:externalEditor"
      }
    }
  ]
}
```

Always include the `$schema` field.

## Keystroke Syntax

**Modifiers** (combine with `+`): `ctrl`, `alt`, `shift`, `meta`

**Special keys**: `escape`/`esc`, `enter`/`return`, `tab`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`

**Chords**: Space-separated keystrokes, e.g. `ctrl+k ctrl+s` (1-second timeout between keystrokes)

## Behavioral Rules

1. Only include contexts the user wants to change (minimal overrides)
2. Validate that actions and contexts are from the known lists
3. Warn the user proactively if they choose a key that conflicts with reserved shortcuts (tmux uses `ctrl+b`, screen uses `ctrl+a`, etc.)
4. When adding a new binding for an existing action, the new binding is additive (existing default still works unless explicitly unbound)
5. To fully replace a default binding, unbind the old key (set to `null`) AND add the new one

## Common Patterns

### Rebind a key
```json
{
  "context": "Chat",
  "bindings": {
    "ctrl+g": null,
    "ctrl+e": "chat:externalEditor"
  }
}
```

### Add a chord binding
```json
{
  "context": "Global",
  "bindings": {
    "ctrl+k ctrl+t": "app:toggleTodos"
  }
}
```

## Validation

Run `/doctor` to validate `~/.claude/keybindings.json` — it includes a "Keybinding Configuration Issues" section. A broken settings file silently disables ALL settings from that file."#;

const CLAUDE_API_PROMPT: &str = r#"# Claude API Helper

Help the user build apps with the Claude API or Anthropic SDK.

## When to Use

TRIGGER when: code imports `anthropic`/`@anthropic-ai/sdk`, or the user asks to use the Claude API, Anthropic SDKs, or the Agent SDK.

DO NOT TRIGGER when: code imports `openai` or other AI SDK, for general programming, or ML/data-science tasks.

## Quick Start

### Python
```python
import anthropic

client = anthropic.Anthropic()

message = client.messages.create(
    model="claude-opus-4-7",
    max_tokens=1024,
    messages=[
        {"role": "user", "content": "Hello, Claude"}
    ]
)
print(message.content)
```

### TypeScript
```typescript
import Anthropic from '@anthropic-ai/sdk';

const client = new Anthropic();

const message = await client.messages.create({
    model: "claude-opus-4-7",
    max_tokens: 1024,
    messages: [
        { role: "user", content: "Hello, Claude" }
    ]
});
console.log(message.content);
```

## Common Patterns

### Streaming responses
Use `client.messages.stream()` (Python) or `client.messages.stream()` (TypeScript) for real-time streaming.

### Tool use / function calling
Pass `tools` array to `messages.create()`. The model returns `tool_use` blocks when it wants to call a tool. Call the tool, pass back a `tool_result` message, and continue.

### Prompt caching
Add `cache_control: { type: "ephemeral" }` to large static content blocks (system prompts, documents) to reduce cost and latency on repeated calls.

### Batch processing
Use the Batches API (`client.beta.messages.batches`) for non-latency-sensitive workloads. Up to 100x cost reduction.

## Error Handling

Common error types: `AuthenticationError` (401), `PermissionDeniedError` (403), `NotFoundError` (404), `RateLimitError` (429), `APIStatusError` (5xx).

Always implement retry with exponential backoff for `RateLimitError` and transient 5xx errors.

## Links

- API Reference: https://docs.anthropic.com/en/api
- SDK Docs: https://docs.anthropic.com/en/docs/sdks-and-tools
- Models: https://docs.anthropic.com/en/docs/about-claude/models"#;

const DEBUG_PROMPT: &str = r#"# Debug Skill

Help the user debug an issue they're encountering in this current Claude Code session.

## Session Debug Log

The debug log for the current session is at the path shown by `claude --debug`. If debug logging was not previously enabled, it has just been turned on — ask the user to reproduce the issue, then re-read the log.

## Instructions

1. Review the user's issue description
2. Read the debug log file — look for `[ERROR]` and `[WARN]` entries, stack traces, and failure patterns
3. Consider launching a subagent to understand the relevant Claude Code features
4. Explain what you found in plain language
5. Suggest concrete fixes or next steps

## Settings Locations

Remember that settings are in:
- User: `~/.claude/settings.json`
- Project: `.claude/settings.json`
- Local: `.claude/settings.local.json`"#;

const UPDATE_CONFIG_PROMPT: &str = r#"# Update Config Skill

Modify Claude Code configuration by updating settings.json files.

## When Hooks Are Required (Not Memory)

If the user wants something to happen automatically in response to an event, they need a **hook** configured in settings.json. Memory/preferences cannot trigger automated actions.

**These require hooks:**
- "Before compacting, ask me what to preserve" → PreCompact hook
- "After writing files, run prettier" → PostToolUse hook with Write|Edit matcher
- "When I run bash commands, log them" → PreToolUse hook with Bash matcher
- "Always run tests after code changes" → PostToolUse hook

**Hook events:** PreToolUse, PostToolUse, PreCompact, PostCompact, Stop, Notification, SessionStart

## CRITICAL: Read Before Write

**Always read the existing settings file before making changes.** Merge new settings with existing ones — never replace the entire file.

## Settings File Locations

| File | Scope | Git | Use For |
|------|-------|-----|---------|
| `~/.claude/settings.json` | Global | N/A | Personal preferences for all projects |
| `.claude/settings.json` | Project | Commit | Team-wide hooks, permissions, plugins |
| `.claude/settings.local.json` | Project | Gitignore | Personal overrides for this project |

Settings load in order: user → project → local (later overrides earlier).

## Hook Structure

```json
{
  "hooks": {
    "PostToolUse": [{
      "matcher": "Write|Edit",
      "hooks": [{
        "type": "command",
        "command": "your-command-here"
      }]
    }]
  }
}
```

## Merging Arrays (Important!)

When adding to permission or hook arrays, **merge with existing** — don't replace.

## Workflow

1. **Clarify intent** — ask if the request is ambiguous (which scope? which file?)
2. **Read existing file** — use Read tool on the target settings file
3. **Merge carefully** — preserve existing settings, especially arrays
4. **Edit file** — use Edit tool to apply changes
5. **Confirm** — tell the user what was changed"#;

const SCHEDULE_PROMPT: &str = r#"# Schedule Remote Agents

Help the user schedule, update, list, or run **remote** Claude Code agents. These are NOT local cron jobs — each trigger spawns a fully isolated remote session in Anthropic's cloud infrastructure on a cron schedule.

## What You Can Do

- **List** all scheduled triggers
- **Create** a new scheduled trigger
- **Update** an existing trigger
- **Run** a trigger now (on demand)

## Create Workflow

1. **Understand the goal** — Ask what the remote agent should do. What repo(s)? What task? Remind them that the agent runs remotely — it won't have access to their local machine, local files, or local environment variables.

2. **Craft the prompt** — Help them write an effective agent prompt. Good prompts are:
   - Specific about what to do and what success looks like
   - Clear about which files/areas to focus on
   - Explicit about what actions to take (open PRs, commit, just analyze, etc.)

3. **Set the schedule** — Ask when and how often. Cron expressions use UTC. When the user says a local time, convert it to UTC and confirm: "9am local = Xam UTC, so the cron would be `0 X * * 1-5`."

4. **Validate connections** — Infer what services the agent will need. If any required MCP connectors are missing, direct the user to connect them first.

5. **Review and confirm** — Show the full configuration before creating. Let them adjust.

6. **Create it** — Use the RemoteTrigger tool with `action: "create"`.

## Cron Expression Examples

- `0 9 * * 1-5` — Every weekday at 9am UTC
- `0 */2 * * *` — Every 2 hours
- `0 0 * * *` — Daily at midnight UTC
- `0 8 1 * *` — First of every month at 8am UTC

Minimum interval is 1 hour.

## Important Notes

- These are REMOTE agents — they cannot access local files, local services, or local environment variables.
- The prompt is the most important part — spend time getting it right. The remote agent starts with zero context.
- Always convert cron to human-readable when displaying.
- To delete a trigger, direct users to https://claude.ai/code/scheduled"#;

/// Handles bundled skills
#[must_use]
pub fn bundled_skills() -> Vec<BundledSkill> {
    vec![
        BundledSkill::inline(
            "workspace-audit",
            "Summarize the current Rust workspace layout and recommend validation commands.",
            "Inspect the current repository as a Rust workspace. List the main crates, note any plugin or skill metadata that was discovered, and finish by recommending the exact cargo validation commands that should be run before shipping changes.",
        ),
        BundledSkill::inline(
            "lorem-ipsum",
            "Generate token-accurate placeholder text using single-token English words.",
            LOREM_IPSUM_PROMPT,
        ),
        BundledSkill::inline(
            "simplify",
            "Review changed files for reuse, quality, and efficiency issues then fix them.",
            SIMPLIFY_PROMPT,
        ),
        BundledSkill::inline(
            "skillify",
            "Capture the current session's repeatable process as a reusable skill definition.",
            SKILLIFY_PROMPT,
        ),
        BundledSkill::inline(
            "stuck",
            "Diagnose frozen or slow Claude Code sessions by inspecting running processes.",
            STUCK_PROMPT,
        ),
        BundledSkill::inline(
            "batch",
            "Research and plan a large-scale change, then execute it in parallel across 5-30 isolated worktree agents that each open a PR.",
            BATCH_PROMPT,
        ),
        BundledSkill::inline(
            "loop",
            "Run a prompt or slash command on a recurring interval (e.g. /loop 5m /foo, defaults to 10m).",
            LOOP_PROMPT,
        ),
        BundledSkill::inline(
            "remember",
            "Review auto-memory entries and propose promotions to CLAUDE.md, CLAUDE.local.md, or shared memory. Also detects outdated, conflicting, and duplicate entries across memory layers.",
            REMEMBER_PROMPT,
        ),
        BundledSkill::inline(
            "verify",
            "Verify a code change does what it should by running the app or tests end-to-end.",
            VERIFY_PROMPT,
        ),
        // keybindings-help is not user-invocable via slash command
        BundledSkill::inline_no_command(
            "keybindings-help",
            "Use when the user wants to customize keyboard shortcuts, rebind keys, add chord bindings, or modify ~/.claude/keybindings.json.",
            KEYBINDINGS_PROMPT,
        ),
        BundledSkill::inline(
            "claude-api",
            "Build apps with the Claude API or Anthropic SDK. Trigger when code imports `anthropic`/`@anthropic-ai/sdk` or user asks to use Claude API.",
            CLAUDE_API_PROMPT,
        ),
        BundledSkill::inline(
            "debug",
            "Enable debug logging for this session and help diagnose issues.",
            DEBUG_PROMPT,
        ),
        BundledSkill::inline(
            "update-config",
            "Configure Claude Code via settings.json. Use for: hooks, permissions, env vars, or any changes to settings.json files.",
            UPDATE_CONFIG_PROMPT,
        ),
        BundledSkill::inline(
            "schedule",
            "Create, update, list, or run scheduled remote agents (triggers) that execute on a cron schedule.",
            SCHEDULE_PROMPT,
        ),
    ]
}
