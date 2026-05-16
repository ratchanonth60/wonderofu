//! Fleet agent role catalog.
//!
//! A *role* is a named configuration bundle that pre-populates the prompt
//! preamble, optional model preference, and allowed tool names for a fleet
//! member.  Roles are identified by a stable kebab-case id such as
//! `rust-engineer` or `code-reviewer`.
//!
//! # Built-ins
//!
//! Nine built-in roles are always available without any file on disk.  They
//! are returned by [`FleetRoleCatalog::builtin`] and accessible via
//! [`FleetRoleCatalog::get`].
//!
//! # Composing a task prompt
//!
//! Use [`FleetAgentRole::compose_prompt`] to prepend the role preamble before
//! the caller-supplied task prompt:
//!
//! ```
//! use wonder_of_u_core::fleet_roles::FleetRoleCatalog;
//!
//! let catalog = FleetRoleCatalog::builtin();
//! let role = catalog.get("rust-engineer").unwrap();
//! let full = role.compose_prompt("Refactor the auth module.");
//! assert!(full.starts_with("[Role:"));
//! assert!(full.contains("Refactor the auth module."));
//! ```

use serde::{Deserialize, Serialize};

use crate::{Result, WonderError};

// ── Role type ─────────────────────────────────────────────────────────────────

/// A named configuration bundle for a fleet sub-agent.
///
/// Each role carries a stable `id` (lowercase kebab-case), a human-readable
/// `name`, a short `description`, and a `prompt_preamble` that is prepended to
/// the task prompt before the agent is launched.  Optional fields (`default_model`,
/// `allowed_tools`, `tags`) are omitted from JSON when empty.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FleetAgentRole {
    /// Stable kebab-case identifier, e.g. `"rust-engineer"`.
    pub id: String,
    /// Human-readable display name, e.g. `"Rust Engineer"`.
    pub name: String,
    /// One-line description of the role's purpose.
    pub description: String,
    /// Prompt preamble prepended to the user task prompt.
    pub prompt_preamble: String,
    /// Optional preferred model for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Optional whitelist of tool names this agent may use.
    ///
    /// An empty list means *all tools are allowed* (same as not specifying any).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    /// Arbitrary capability/taxonomy tags for filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl FleetAgentRole {
    /// Constructs a new role and validates it immediately.
    ///
    /// Returns a [`WonderError`] if validation fails.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        prompt_preamble: impl Into<String>,
    ) -> Result<Self> {
        let role = Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            prompt_preamble: prompt_preamble.into(),
            default_model: None,
            allowed_tools: Vec::new(),
            tags: Vec::new(),
        };
        role.validate()?;
        Ok(role)
    }

    /// Validates role fields.
    ///
    /// Rules:
    /// * `id` must be non-empty, lowercase, and match `[a-z0-9][a-z0-9-]*`.
    /// * `name`, `description`, and `prompt_preamble` must be non-empty after trimming.
    /// * `allowed_tools` must not contain duplicates.
    pub fn validate(&self) -> Result<()> {
        validate_role_id(&self.id)?;

        for (field, value) in [
            ("name", &self.name),
            ("description", &self.description),
            ("prompt_preamble", &self.prompt_preamble),
        ] {
            if value.trim().is_empty() {
                return Err(WonderError::validation(format!(
                    "fleet role `{}`: `{field}` must not be empty",
                    self.id
                )));
            }
        }

        // Detect duplicate tool names (case-sensitive).
        let mut seen = std::collections::BTreeSet::new();
        for tool in &self.allowed_tools {
            if !seen.insert(tool.as_str()) {
                return Err(WonderError::validation(format!(
                    "fleet role `{}`: duplicate tool name `{tool}` in allowed_tools",
                    self.id
                )));
            }
        }

        Ok(())
    }

    /// Composes the full agent prompt by prepending the role preamble.
    ///
    /// Format: `[Role: {name}]\n{prompt_preamble}\n\nTask:\n{task_prompt}`
    #[must_use]
    pub fn compose_prompt(&self, task_prompt: &str) -> String {
        format!(
            "[Role: {}]\n{}\n\nTask:\n{}",
            self.name, self.prompt_preamble, task_prompt
        )
    }
}

// ── Catalog type ──────────────────────────────────────────────────────────────

/// An ordered, deduplicated collection of [`FleetAgentRole`] values.
///
/// Roles are stored in the order they were added and indexed by `id`.
/// [`FleetRoleCatalog::builtin`] returns the nine built-in roles; callers may
/// extend or override with [`FleetRoleCatalog::merge`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FleetRoleCatalog {
    // BTreeMap for deterministic iteration / lookup.
    roles: std::collections::BTreeMap<String, FleetAgentRole>,
}

impl FleetRoleCatalog {
    /// Creates an empty catalog.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns the catalog of built-in roles.
    ///
    /// The nine roles are always available without any external configuration.
    #[must_use]
    pub fn builtin() -> Self {
        let mut catalog = Self::empty();
        for role in builtin_roles() {
            catalog.insert(role);
        }
        catalog
    }

    /// Inserts or replaces a role.
    ///
    /// If a role with the same `id` already exists it is overwritten.
    pub fn insert(&mut self, role: FleetAgentRole) {
        self.roles.insert(role.id.clone(), role);
    }

    /// Looks up a role by its kebab-case id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&FleetAgentRole> {
        self.roles.get(id)
    }

    /// Returns all roles in deterministic (alphabetical) id order.
    #[must_use]
    pub fn list(&self) -> Vec<&FleetAgentRole> {
        self.roles.values().collect()
    }

    /// Returns `true` when the catalog contains no roles.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roles.is_empty()
    }

    /// Returns the number of roles in the catalog.
    #[must_use]
    pub fn len(&self) -> usize {
        self.roles.len()
    }

    /// Merges another catalog into this one; roles from `other` overwrite
    /// existing entries with the same id.
    pub fn merge(&mut self, other: FleetRoleCatalog) {
        for (id, role) in other.roles {
            self.roles.insert(id, role);
        }
    }

    /// Returns the stable ids of all roles in deterministic order.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.roles.keys().map(String::as_str).collect()
    }

    /// Resolves a possibly-human-friendly label to a canonical role id.
    ///
    /// Accepts:
    /// * An exact role id (`"rust-engineer"`).
    /// * A display name (`"Rust Engineer"`).
    /// * Common aliases (e.g. `"code-review"` → `"code-reviewer"`).
    ///
    /// Returns `None` when no mapping is found.
    #[must_use]
    pub fn resolve_alias(&self, label: &str) -> Option<&FleetAgentRole> {
        // 1. Exact id match.
        if let Some(role) = self.roles.get(label) {
            return Some(role);
        }

        // 2. Case-insensitive display name match.
        let lower = label.to_lowercase();
        for role in self.roles.values() {
            if role.name.to_lowercase() == lower {
                return Some(role);
            }
        }

        // 3. Well-known aliases.
        let alias_id = match label {
            "code-review" | "CodeReview" | "code_review" => "code-reviewer",
            "Rust Engineer" | "rust_engineer" => "rust-engineer",
            "Rust Tester" | "rust_tester" => "rust-tester",
            "Rust Architect" | "rust_architect" => "rust-architect",
            "Rust Refactor" | "rust_refactor" => "rust-refactor",
            "Rust Optimizer" | "rust_optimizer" => "rust-optimizer",
            "Rust Documenter" | "rust_documenter" => "rust-documenter",
            "TUI Designer" | "tui_designer" | "tui-design" => "tui-designer",
            "Rubber Duck" | "rubber_duck" | "rubber-duck-debug" => "rubber-duck",
            _ => return None,
        };
        self.roles.get(alias_id)
    }

    /// Formats a comma-separated list of known role ids for error messages.
    #[must_use]
    pub fn known_ids_display(&self) -> String {
        self.ids().join(", ")
    }
}

// ── Built-in role definitions ─────────────────────────────────────────────────

/// Returns the nine built-in fleet agent roles.
///
/// Panics in debug builds if any role fails validation (they are compile-time
/// constants, so a failure indicates a code bug, not user input).
fn builtin_roles() -> Vec<FleetAgentRole> {
    let specs: &[(&str, &str, &str, &str, &[&str])] = &[
        (
            "rust-engineer",
            "Rust Engineer",
            "Implements idiomatic Rust features and bug fixes in the workspace.",
            "You are a senior Rust engineer working in the wonder-of-u workspace \
             (edition 2024, MSRV 1.85). Write idiomatic, safe, well-tested Rust. \
             Prefer iterators, `?`, `thiserror`, `#[must_use]` on builders, \
             `&str`/`&[T]` over owned types where possible. Run `cargo check` and \
             `cargo test` after every change. Never add `unsafe` without explicit \
             approval and a `// SAFETY:` comment.",
            &["bash", "read_file", "write_file", "search_files"],
        ),
        (
            "rust-tester",
            "Rust Tester",
            "Adds and improves test coverage for Rust crates in the workspace.",
            "You are a Rust test specialist for the wonder-of-u workspace. \
             Your job is to write thorough unit and integration tests. \
             Co-locate unit tests in `#[cfg(test)] mod tests` inside the module \
             under test. Place integration tests under `<crate>/tests/`. \
             Reuse helpers from `wonder-of-u-test-support`. \
             Cover at least one happy path and one failure/edge case per item.",
            &["bash", "read_file", "write_file", "search_files"],
        ),
        (
            "rust-architect",
            "Rust Architect",
            "Designs module boundaries, crate APIs, and data model evolution.",
            "You are a software architect for the wonder-of-u workspace. \
             Focus on clean module boundaries, minimal public API surface, \
             and long-lived data model decisions. Prefer additive changes; \
             flag breaking changes explicitly. \
             Produce a structured design proposal before writing any code.",
            &["bash", "read_file", "search_files"],
        ),
        (
            "rust-refactor",
            "Rust Refactor",
            "Refactors existing Rust code for clarity, idioms, and reduced duplication.",
            "You are a Rust refactoring specialist for the wonder-of-u workspace. \
             Improve code clarity and idiomatic style without changing observable \
             behaviour. Consolidate duplicated logic, extract helpers, and remove \
             dead code. All existing tests must continue to pass after your changes.",
            &["bash", "read_file", "write_file", "search_files"],
        ),
        (
            "rust-optimizer",
            "Rust Optimizer",
            "Profiles and optimises hot paths in Rust code using benchmarks.",
            "You are a Rust performance engineer for the wonder-of-u workspace. \
             Do NOT claim an optimisation works without measurement. \
             Use criterion benchmarks or `cargo test --release` timing. \
             Identify bottlenecks (allocations, clones, lock contention, async \
             stalls), apply the smallest effective change, and report before/after \
             numbers. Never sacrifice correctness or safety for speed.",
            &["bash", "read_file", "write_file", "search_files"],
        ),
        (
            "rust-documenter",
            "Rust Documenter",
            "Writes and improves Rust documentation (doc-comments, examples, READMEs).",
            "You are a technical writer specialising in Rust documentation for \
             the wonder-of-u workspace. Write concise `///` doc-comments with a \
             short summary first, followed by `# Examples` where they add value. \
             Use `//!` for module-level docs. Avoid filler comments that restate \
             the code. Do not change any logic — documentation only.",
            &["bash", "read_file", "write_file", "search_files"],
        ),
        (
            "tui-designer",
            "TUI Designer",
            "Designs and implements terminal UI layouts using the project's TUI stack.",
            "You are a TUI designer for the wonder-of-u workspace which uses a \
             Ratatui-based terminal interface. Design clean, keyboard-navigable \
             layouts. Minimise flicker by batching state changes. Follow the \
             existing widget and theme conventions in `wonder-of-u-tui`. \
             Propose a sketch of the component tree before writing code.",
            &["bash", "read_file", "write_file", "search_files"],
        ),
        (
            "rubber-duck",
            "Rubber Duck",
            "Acts as a silent sounding board: asks clarifying questions and identifies assumptions.",
            "You are a rubber duck debugging companion. Do not write any code \
             unless explicitly asked. Instead, ask clarifying questions, surface \
             hidden assumptions, and help the user reason through the problem. \
             Restate what you understand after each explanation so the user can \
             correct misunderstandings.",
            &["read_file", "search_files"],
        ),
        (
            "code-reviewer",
            "Code Reviewer",
            "Reviews diffs and code for correctness, style, and security issues.",
            "You are a thorough code reviewer for the wonder-of-u workspace. \
             Review the provided diff or files for: correctness, Rust idioms, \
             potential panics or unsafe usage, test coverage gaps, and obvious \
             security issues. Structure your feedback as numbered findings with \
             severity (Critical / Major / Minor / Nit) and suggested fix.",
            &["bash", "read_file", "search_files"],
        ),
    ];

    specs
        .iter()
        .map(|(id, name, desc, preamble, tools)| {
            let mut role = FleetAgentRole {
                id: (*id).to_string(),
                name: (*name).to_string(),
                description: (*desc).to_string(),
                prompt_preamble: (*preamble).to_string(),
                default_model: None,
                allowed_tools: tools.iter().map(|t| (*t).to_string()).collect(),
                tags: Vec::new(),
            };
            // Tag roles by their primary concern.
            role.tags = derive_tags(id);
            debug_assert!(
                role.validate().is_ok(),
                "built-in role `{id}` failed validation: {:?}",
                role.validate()
            );
            role
        })
        .collect()
}

fn derive_tags(id: &str) -> Vec<String> {
    let mut tags = Vec::new();
    if id.starts_with("rust-") {
        tags.push("rust".into());
    }
    if matches!(id, "rust-engineer" | "rust-refactor" | "rust-optimizer") {
        tags.push("implementation".into());
    }
    if matches!(id, "rust-tester" | "code-reviewer") {
        tags.push("quality".into());
    }
    if id == "rust-documenter" {
        tags.push("docs".into());
    }
    if id == "tui-designer" {
        tags.push("tui".into());
        tags.push("design".into());
    }
    if id == "rubber-duck" {
        tags.push("analysis".into());
    }
    tags
}

// ── Validation helpers ────────────────────────────────────────────────────────

/// Validates that a role id is lowercase kebab-case: `[a-z0-9][a-z0-9-]*`.
fn validate_role_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(WonderError::validation("fleet role id must not be empty"));
    }

    let valid = id
        .chars()
        .enumerate()
        .all(|(i, c)| match c {
            'a'..='z' | '0'..='9' => true,
            '-' if i > 0 => true, // no leading dash
            _ => false,
        })
        // No trailing dash.
        && !id.ends_with('-')
        // No consecutive dashes.
        && !id.contains("--");

    if valid {
        Ok(())
    } else {
        Err(WonderError::validation(format!(
            "fleet role id `{id}` is invalid: must match [a-z0-9][a-z0-9-]* with no consecutive or trailing dashes"
        )))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── FleetAgentRole validation ─────────────────────────────────────────────

    #[test]
    fn valid_role_id_accepted() {
        assert!(validate_role_id("rust-engineer").is_ok());
        assert!(validate_role_id("code-reviewer").is_ok());
        assert!(validate_role_id("r2d2").is_ok());
    }

    #[test]
    fn invalid_role_ids_rejected() {
        assert!(validate_role_id("").is_err());
        assert!(validate_role_id("Rust-Engineer").is_err()); // uppercase
        assert!(validate_role_id("-leading").is_err()); // leading dash
        assert!(validate_role_id("trailing-").is_err()); // trailing dash
        assert!(validate_role_id("double--dash").is_err()); // consecutive dashes
        assert!(validate_role_id("has space").is_err()); // space
    }

    #[test]
    fn role_new_validates_fields() {
        let ok = FleetAgentRole::new("my-role", "My Role", "does stuff", "You are helpful.");
        assert!(ok.is_ok());

        let empty_name = FleetAgentRole::new("my-role", "  ", "desc", "preamble");
        assert!(empty_name.is_err());
        assert!(empty_name.unwrap_err().to_string().contains("name"));
    }

    #[test]
    fn role_rejects_duplicate_allowed_tools() {
        let mut role =
            FleetAgentRole::new("my-role", "My Role", "does stuff", "You are helpful.").unwrap();
        role.allowed_tools = vec!["bash".into(), "bash".into()];
        let err = role.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate tool name"));
    }

    #[test]
    fn compose_prompt_includes_role_name_and_task() {
        let role = FleetAgentRole::new("my-role", "My Role", "does stuff", "You are a specialist.")
            .unwrap();
        let composed = role.compose_prompt("Fix the bug.");
        assert!(composed.contains("[Role: My Role]"));
        assert!(composed.contains("You are a specialist."));
        assert!(composed.contains("Fix the bug."));
    }

    // ── FleetRoleCatalog ──────────────────────────────────────────────────────

    #[test]
    fn builtin_catalog_has_nine_roles() {
        let catalog = FleetRoleCatalog::builtin();
        assert_eq!(catalog.len(), 9);
    }

    #[test]
    fn builtin_role_ids_are_stable() {
        let catalog = FleetRoleCatalog::builtin();
        let ids = catalog.ids();
        for expected in &[
            "code-reviewer",
            "rubber-duck",
            "rust-architect",
            "rust-documenter",
            "rust-engineer",
            "rust-optimizer",
            "rust-refactor",
            "rust-tester",
            "tui-designer",
        ] {
            assert!(
                ids.contains(expected),
                "missing expected role id `{expected}`"
            );
        }
    }

    #[test]
    fn catalog_list_is_alphabetically_ordered() {
        let catalog = FleetRoleCatalog::builtin();
        let ids: Vec<&str> = catalog.list().iter().map(|r| r.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(
            ids, sorted,
            "list() should return roles in alphabetical id order"
        );
    }

    #[test]
    fn catalog_get_known_role() {
        let catalog = FleetRoleCatalog::builtin();
        let role = catalog
            .get("rust-engineer")
            .expect("rust-engineer not found");
        assert_eq!(role.name, "Rust Engineer");
    }

    #[test]
    fn catalog_get_unknown_returns_none() {
        let catalog = FleetRoleCatalog::builtin();
        assert!(catalog.get("does-not-exist").is_none());
    }

    #[test]
    fn catalog_resolve_alias_display_name() {
        let catalog = FleetRoleCatalog::builtin();
        let role = catalog
            .resolve_alias("Rust Engineer")
            .expect("alias lookup failed");
        assert_eq!(role.id, "rust-engineer");
    }

    #[test]
    fn catalog_resolve_alias_known_short_forms() {
        let catalog = FleetRoleCatalog::builtin();
        assert_eq!(
            catalog.resolve_alias("code-review").map(|r| r.id.as_str()),
            Some("code-reviewer")
        );
        assert_eq!(
            catalog.resolve_alias("TUI Designer").map(|r| r.id.as_str()),
            Some("tui-designer")
        );
        assert_eq!(
            catalog.resolve_alias("Rubber Duck").map(|r| r.id.as_str()),
            Some("rubber-duck")
        );
    }

    #[test]
    fn catalog_merge_overrides_existing_role() {
        let mut base = FleetRoleCatalog::builtin();
        let mut override_catalog = FleetRoleCatalog::empty();
        let custom = FleetAgentRole::new(
            "rust-engineer",
            "Rust Engineer (Custom)",
            "custom variant",
            "Custom preamble.",
        )
        .unwrap();
        override_catalog.insert(custom.clone());
        base.merge(override_catalog);
        assert_eq!(
            base.get("rust-engineer").unwrap().name,
            "Rust Engineer (Custom)"
        );
    }

    #[test]
    fn all_builtin_roles_pass_validation() {
        for role in FleetRoleCatalog::builtin().list() {
            role.validate()
                .unwrap_or_else(|e| panic!("built-in role `{}` failed validation: {e}", role.id));
        }
    }

    #[test]
    fn builtin_role_serde_round_trip() {
        let catalog = FleetRoleCatalog::builtin();
        let role = catalog.get("code-reviewer").unwrap();
        let json = serde_json::to_string(role).expect("serialize");
        let decoded: FleetAgentRole = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(role, &decoded);
    }
}
