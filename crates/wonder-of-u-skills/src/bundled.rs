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
    /// Handles inline
    #[must_use]
    pub fn inline(
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
                slash_command: Some("workspace-audit".into()),
                slash_aliases: vec!["audit-workspace".into()],
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
    ]
}
