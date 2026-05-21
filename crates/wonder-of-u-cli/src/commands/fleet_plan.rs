//! Fleet plan parser and validator.
//!
//! A *fleet plan* is a JSON array of member specs.  This module parses the
//! array, validates all constraints (unique ids, valid id pattern, dependency
//! references, no cycles, known roles), and returns specs in **topological
//! order** so that dependency-free members appear first.
//!
//! # JSON schema (per element)
//!
//! ```json
//! {
//!   "id":          "<required: plan member id>",
//!   "prompt":      "<required: agent prompt>",
//!   "name":        "<optional: human label>",
//!   "description": "<optional>",
//!   "role":        "<optional: fleet role id>",
//!   "model":       "<optional>",
//!   "provider":    "<optional>",
//!   "cwd":         "<optional: path>",
//!   "depends_on":  ["<id>", ...],
//!   "isolation":   { "mode": "worktree", "branch": "<optional branch name>" }
//! }
//! ```

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

use serde::Deserialize;
use wonder_of_u_core::{FleetRoleCatalog, Result, WonderError, WorktreeIsolation};
use wonder_of_u_tools::validate_worktree_branch_name;

/// One member entry parsed from a fleet plan JSON file.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct PlanSpec {
    /// Plan-scoped identifier; must match `[a-z0-9][a-z0-9_-]{0,62}`.
    pub id: String,
    /// Agent prompt.
    pub prompt: String,
    /// Optional human-readable label.
    #[serde(default)]
    pub name: Option<String>,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional fleet role id.
    #[serde(default)]
    pub role: Option<String>,
    /// Optional model override.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional provider override.
    #[serde(default)]
    pub provider: Option<String>,
    /// Optional working directory.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// Ids of members that must complete before this one launches.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Optional worktree isolation for this member.
    #[serde(default)]
    pub isolation: Option<WorktreeIsolation>,
}

/// Parse a fleet plan from JSON text and return specs in topological order.
///
/// Validates:
/// - Non-empty plan.
/// - Unique `id`s.
/// - Every `id` matches `[a-z0-9][a-z0-9_-]{0,62}`.
/// - All `depends_on` references resolve to another member in the plan.
/// - No cycles.
/// - Every `role`, if present, resolves in [`FleetRoleCatalog::builtin()`].
///
/// Returns specs ordered so that each spec's dependencies appear before it
/// (Kahn's algorithm for deterministic topological sort).
pub(super) fn parse_plan(json: &str) -> Result<Vec<PlanSpec>> {
    let specs: Vec<PlanSpec> = serde_json::from_str(json)
        .map_err(|e| WonderError::validation(format!("fleet plan JSON parse error: {e}")))?;

    if specs.is_empty() {
        return Err(WonderError::validation(
            "fleet plan must contain at least one member",
        ));
    }

    // ── Validate ids are unique ───────────────────────────────────────────────
    let mut seen_ids: BTreeSet<&str> = BTreeSet::new();
    for spec in &specs {
        if !seen_ids.insert(spec.id.as_str()) {
            return Err(WonderError::validation(format!(
                "fleet plan contains duplicate id `{}`",
                spec.id
            )));
        }
    }

    // ── Validate id patterns ──────────────────────────────────────────────────
    for spec in &specs {
        validate_plan_id(&spec.id)?;
    }

    // ── Validate depends_on references ────────────────────────────────────────
    for spec in &specs {
        for dep in &spec.depends_on {
            if !seen_ids.contains(dep.as_str()) {
                return Err(WonderError::validation(format!(
                    "fleet plan member `{}` depends_on unknown id `{dep}`",
                    spec.id
                )));
            }
        }
    }

    // ── Validate roles resolve ────────────────────────────────────────────────
    let catalog = FleetRoleCatalog::builtin();
    for spec in &specs {
        if let Some(ref role_id) = spec.role {
            let resolved = catalog
                .get(role_id)
                .or_else(|| catalog.resolve_alias(role_id));
            if resolved.is_none() {
                return Err(WonderError::validation(format!(
                    "fleet plan member `{}` has unknown role `{role_id}`; known roles: {}",
                    spec.id,
                    catalog.known_ids_display()
                )));
            }
        }
    }

    // ── Validate isolation branch names ───────────────────────────────────────
    for spec in &specs {
        if let Some(ref iso) = spec.isolation {
            if let Some(ref branch) = iso.branch {
                validate_worktree_branch_name(branch).map_err(|e| {
                    WonderError::validation(format!(
                        "fleet plan member `{}` isolation.branch is invalid: {e}",
                        spec.id
                    ))
                })?;
            }
        }
    }

    // ── Topological sort (Kahn's algorithm) ───────────────────────────────────
    // Build index map from id → position and adjacency (dependents of each node).
    let index_of: BTreeMap<&str, usize> = specs
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();

    // in_degree[i] = how many unresolved dependencies node i has.
    let mut in_degree = vec![0usize; specs.len()];
    // adjacency[i] = set of node indices that depend on i.
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); specs.len()];

    for (i, spec) in specs.iter().enumerate() {
        in_degree[i] = spec.depends_on.len();
        for dep_id in &spec.depends_on {
            // Safe: we already validated all dep references exist.
            let dep_idx = index_of[dep_id.as_str()];
            dependents[dep_idx].push(i);
        }
    }

    // Seed queue with roots (nodes with no dependencies).  Use a sorted
    // order by id to make the output deterministic regardless of input order.
    let mut roots: Vec<usize> = (0..specs.len()).filter(|&i| in_degree[i] == 0).collect();
    // Sort by id for stability.
    roots.sort_by_key(|&i| specs[i].id.as_str());
    let mut queue: VecDeque<usize> = roots.into();

    let mut order: Vec<usize> = Vec::with_capacity(specs.len());
    while let Some(node) = queue.pop_front() {
        order.push(node);
        // Collect next-level nodes, sort for determinism, then push.
        let mut next: Vec<usize> = dependents[node]
            .iter()
            .copied()
            .filter(|&j| {
                in_degree[j] -= 1;
                in_degree[j] == 0
            })
            .collect();
        next.sort_by_key(|&j| specs[j].id.as_str());
        queue.extend(next);
    }

    if order.len() != specs.len() {
        // Not all nodes visited → cycle detected.
        let cycle_ids: Vec<&str> = (0..specs.len())
            .filter(|i| !order.contains(i))
            .map(|i| specs[i].id.as_str())
            .collect();
        return Err(WonderError::validation(format!(
            "fleet plan contains a dependency cycle involving: {}",
            cycle_ids.join(", ")
        )));
    }

    Ok(order.into_iter().map(|i| specs[i].clone()).collect())
}

/// Validates that a plan member id matches `[a-z0-9][a-z0-9_-]{0,62}`.
fn validate_plan_id(id: &str) -> Result<()> {
    let bytes = id.as_bytes();
    if bytes.is_empty() {
        return Err(WonderError::validation("fleet plan id must not be empty"));
    }
    if id.len() > 63 {
        return Err(WonderError::validation(format!(
            "fleet plan id `{id}` exceeds 63 characters"
        )));
    }
    let first = bytes[0] as char;
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(WonderError::validation(format!(
            "fleet plan id `{id}` must start with [a-z0-9]"
        )));
    }
    for ch in id.chars().skip(1) {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '_' && ch != '-' {
            return Err(WonderError::validation(format!(
                "fleet plan id `{id}` contains invalid character `{ch}`; \
                 allowed: [a-z0-9_-]"
            )));
        }
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Valid plans ───────────────────────────────────────────────────────────

    #[test]
    fn single_member_plan_parses() {
        let json = r#"[{"id":"step-1","prompt":"do the thing"}]"#;
        let specs = parse_plan(json).expect("parse");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].id, "step-1");
    }

    #[test]
    fn linear_chain_returns_topological_order() {
        // step-1 → step-2 → step-3
        let json = serde_json::json!([
            {"id": "step-3", "prompt": "c", "depends_on": ["step-2"]},
            {"id": "step-1", "prompt": "a"},
            {"id": "step-2", "prompt": "b", "depends_on": ["step-1"]},
        ])
        .to_string();
        let specs = parse_plan(&json).expect("parse");
        let ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        // step-1 must precede step-2, step-2 must precede step-3.
        let pos: std::collections::HashMap<&str, usize> =
            ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        assert!(pos["step-1"] < pos["step-2"]);
        assert!(pos["step-2"] < pos["step-3"]);
    }

    #[test]
    fn diamond_dag_returns_topological_order() {
        // root → left, root → right; left + right → leaf
        let json = serde_json::json!([
            {"id": "leaf",  "prompt": "d", "depends_on": ["left","right"]},
            {"id": "root",  "prompt": "a"},
            {"id": "left",  "prompt": "b", "depends_on": ["root"]},
            {"id": "right", "prompt": "c", "depends_on": ["root"]},
        ])
        .to_string();
        let specs = parse_plan(&json).expect("parse");
        let pos: std::collections::HashMap<&str, usize> = specs
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.as_str(), i))
            .collect();
        assert!(pos["root"] < pos["left"]);
        assert!(pos["root"] < pos["right"]);
        assert!(pos["left"] < pos["leaf"]);
        assert!(pos["right"] < pos["leaf"]);
    }

    // ── Validation errors ─────────────────────────────────────────────────────

    #[test]
    fn empty_plan_is_rejected() {
        let err = parse_plan("[]").unwrap_err();
        assert!(err.to_string().contains("at least one"), "got: {err}");
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let json = r#"[
            {"id":"step-1","prompt":"a"},
            {"id":"step-1","prompt":"b"}
        ]"#;
        let err = parse_plan(json).unwrap_err();
        assert!(err.to_string().contains("duplicate id"), "got: {err}");
    }

    #[test]
    fn unknown_dependency_is_rejected() {
        let json = r#"[{"id":"step-1","prompt":"a","depends_on":["step-0"]}]"#;
        let err = parse_plan(json).unwrap_err();
        assert!(
            err.to_string().contains("unknown id `step-0`"),
            "got: {err}"
        );
    }

    #[test]
    fn two_node_cycle_is_rejected() {
        let json = serde_json::json!([
            {"id": "a", "prompt": "x", "depends_on": ["b"]},
            {"id": "b", "prompt": "y", "depends_on": ["a"]},
        ])
        .to_string();
        let err = parse_plan(&json).unwrap_err();
        assert!(err.to_string().contains("cycle"), "got: {err}");
    }

    #[test]
    fn unknown_role_is_rejected() {
        let json = r#"[{"id":"step-1","prompt":"a","role":"nonexistent-role"}]"#;
        let err = parse_plan(json).unwrap_err();
        assert!(err.to_string().contains("unknown role"), "got: {err}");
    }

    #[test]
    fn invalid_id_uppercase_rejected() {
        let json = r#"[{"id":"Step-1","prompt":"a"}]"#;
        let err = parse_plan(json).unwrap_err();
        assert!(err.to_string().contains("must start with"), "got: {err}");
    }

    #[test]
    fn invalid_id_special_char_rejected() {
        let json = r#"[{"id":"step.1","prompt":"a"}]"#;
        let err = parse_plan(json).unwrap_err();
        assert!(err.to_string().contains("invalid character"), "got: {err}");
    }

    #[test]
    fn known_role_is_accepted() {
        let json = r#"[{"id":"step-1","prompt":"a","role":"rust-engineer"}]"#;
        let specs = parse_plan(json).expect("parse");
        assert_eq!(specs[0].role.as_deref(), Some("rust-engineer"));
    }

    #[test]
    fn id_exactly_63_chars_is_valid() {
        let id = "a".repeat(63);
        let json = format!(r#"[{{"id":"{id}","prompt":"a"}}]"#);
        parse_plan(&json).expect("63-char id should be valid");
    }

    #[test]
    fn id_64_chars_is_rejected() {
        let id = "a".repeat(64);
        let json = format!(r#"[{{"id":"{id}","prompt":"a"}}]"#);
        let err = parse_plan(&json).unwrap_err();
        assert!(err.to_string().contains("exceeds 63"), "got: {err}");
    }

    // ── Isolation field tests ─────────────────────────────────────────────────

    #[test]
    fn plan_member_with_worktree_isolation_parses() {
        let json = r#"[{
            "id": "step-1",
            "prompt": "do the thing",
            "isolation": {"mode": "worktree"}
        }]"#;
        let specs = parse_plan(json).expect("parse");
        assert!(specs[0].isolation.is_some());
        let iso = specs[0].isolation.as_ref().unwrap();
        assert!(iso.branch.is_none());
    }

    #[test]
    fn plan_member_with_explicit_isolation_branch_parses() {
        let json = r#"[{
            "id": "step-1",
            "prompt": "do the thing",
            "isolation": {"mode": "worktree", "branch": "feat/my-feature"}
        }]"#;
        let specs = parse_plan(json).expect("parse");
        let iso = specs[0].isolation.as_ref().expect("isolation");
        assert_eq!(iso.branch.as_deref(), Some("feat/my-feature"));
    }

    #[test]
    fn plan_member_with_invalid_isolation_branch_is_rejected() {
        let json = r#"[{
            "id": "step-1",
            "prompt": "do the thing",
            "isolation": {"mode": "worktree", "branch": "feat..bad"}
        }]"#;
        let err = parse_plan(json).unwrap_err();
        assert!(err.to_string().contains("isolation.branch"), "got: {err}");
    }

    #[test]
    fn plan_member_without_isolation_defaults_to_none() {
        let json = r#"[{"id":"step-1","prompt":"a"}]"#;
        let specs = parse_plan(json).expect("parse");
        assert!(specs[0].isolation.is_none());
    }
}
