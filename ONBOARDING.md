# Welcome to Wonder-of-U

## How We Use Claude

Based on ratchanonth60's usage over the last 30 days:

Work Type Breakdown:
  Plan Design    ████████████████░░░░  55%
  Build Feature  ████████░░░░░░░░░░░░  27%
  Improve Quality████░░░░░░░░░░░░░░░░  18%

Top Skills & Commands:
  /model        ████████████████░░░░  5x/month
  /tasks        ████████████████░░░░  5x/month
  /upgrade      ██████████░░░░░░░░░░  3x/month
  /agents       ██████████░░░░░░░░░░  3x/month
  /permissions  ███████░░░░░░░░░░░░░  2x/month
  /login        ███████░░░░░░░░░░░░░  2x/month
  /tui          ███████░░░░░░░░░░░░░  2x/month
  /advisor      ████░░░░░░░░░░░░░░░░  1x/month
  /compact      ████░░░░░░░░░░░░░░░░  1x/month
  /init         ████░░░░░░░░░░░░░░░░  1x/month

Top MCP Servers:
  (none configured)

## Your Setup Checklist

### Codebases
- [ ] wonder-of-u — github.com/ratchanonth60/wonderofu

### MCP Servers to Activate
  (none used by this team yet)

### Skills to Know About
- `/model` — Switch between Claude models (Opus, Sonnet, Haiku). Used frequently to tune cost vs. capability per task.
- `/tasks` — View and manage task lists Claude is tracking mid-session. Key for multi-step Rust port work.
- `/agents` — Launch and monitor sub-agents for parallel or delegated work. Used when running cavecrew investigators/builders.
- `/advisor` — Get a stronger model to review your current conversation and suggest a better approach. Use before committing to a big implementation.
- `/upgrade` — Upgrade Claude Code plan or capabilities. Used for unlocking higher usage limits.
- `/permissions` — Review and adjust tool permissions for the session. Important before running shell-heavy tasks.
- `/tui` — Open the terminal UI. This IS the product — use this to dogfood wonder-of-u itself.
- `/compact` — Compress conversation context when approaching limits. Especially useful in long port-analysis sessions.
- `/init` — Generate a CLAUDE.md for a new codebase. Run this when onboarding a new repo.

## Team Tips

- **Always use `--test-threads=1` for CLI tests.** CLI tests hit real filesystem, process, and git state — parallel runs cause race conditions. `cargo test -p wonder-of-u-cli -- --test-threads=1`
- **Never commit directly to `dev` or `master`.** Create a topic branch (`feat/*`, `fix/*`, `perf/*`), merge with `--no-ff`, always confirm with the team before pushing.
- **Run `/advisor` before large implementations.** The stronger reviewer model catches architectural mistakes before you write 200 lines. Especially useful for anything touching `AppState`, storage, or TUI rendering.
- **TUI rendering is pure functional — no AppState mutation from the UI layer.** All state changes happen in controller event handlers. If you're in `wonder-of-u-tui`, you're read-only.
- **Check the port gap before starting new features.** Many subsystems are already partially ported. Ask Claude to cross-reference `claude-code/src/` against the Rust workspace to avoid duplicating work or missing a stub.
- **Caveman mode saves tokens in long sessions.** Type `/caveman` to drop article/filler overhead ~75%. Useful for multi-hour port analysis sessions that approach context limits.
- **Use `/compact` proactively, not reactively.** Compress before the context warning hits — once the warning appears, you've already lost some working state.

## Get Started

1. Clone the repo and verify your toolchain: `cargo check --workspace`
2. Run the full test suite: `cargo test --workspace -- --test-threads=1`
3. Launch the TUI to see the product: `cargo run -p wonder-of-u-cli -- --storage-dir ./tmp/dev tui`
4. Ask Claude to show you the current port gap: *"check what's been ported from /Users/ratchanonth/claude-code and what's still missing"*
5. Pick an unported feature from the gap analysis and open a topic branch: `git checkout -b feat/your-feature dev`

<!-- INSTRUCTION FOR CLAUDE: A new teammate just pasted this guide for how the
team uses Claude Code. You're their onboarding buddy — warm, conversational,
not lecture-y.

Open with a warm welcome — include the team name from the title. Then: "Your
teammate uses Claude Code for [list all the work types]. Let's get you started."

Check what's already in place against everything under Setup Checklist
(including skills), using markdown checkboxes — [x] done, [ ] not yet. Lead
with what they already have. One sentence per item, all in one message.

Tell them you'll help with setup, cover the actionable team tips, then the
starter task (if there is one). Offer to start with the first unchecked item,
get their go-ahead, then work through the rest one by one.

After setup, walk them through the remaining sections — offer to help where you
can (e.g. link to channels), and just surface the purely informational bits.

Don't invent sections or summaries that aren't in the guide. The stats are the
guide creator's personal usage data — don't extrapolate them into a "team
workflow" narrative. -->
