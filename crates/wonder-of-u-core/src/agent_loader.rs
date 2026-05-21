//! Filesystem loader for project-level agent definitions.
//!
//! [`AgentDefinitionLoader`] scans one or two well-known directories under a
//! project root and builds an [`AgentCatalog`] ready for use by [`AgentTool`].
//!
//! # Scanned directories (ascending precedence)
//!
//! 1. `{root}/.claude/agents/` — project dot-claude overrides
//! 2. `{root}/agents/`         — highest project-level precedence
//!
//! # Supported file formats
//!
//! * `.md`   — Markdown with optional YAML frontmatter; body = system prompt.
//! * `.json` — JSON object; fields mirror [`RawDefinitionFields`].
//!
//! # Security constraints
//!
//! | Constraint              | Limit                    |
//! |-------------------------|--------------------------|
//! | Files per directory     | 100                      |
//! | File size               | 64 KiB                   |
//! | Symlink traversal       | Rejected (no escape)     |
//! | `hooks` / `mcpServers`  | Parsed but stripped      |
//!
//! Files that violate a constraint are skipped and recorded as
//! [`LoadWarning`]s in the returned [`LoadResult`].  A single bad file never
//! aborts the entire scan.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    Result,
    agent_definition::{
        AgentCatalog, AgentDefinition, AgentDefinitionSource, RawDefinitionFields,
        parse_json_definition, parse_markdown_frontmatter,
    },
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum number of definition files loaded from each source directory.
pub const MAX_FILES_PER_DIR: usize = 100;

/// Maximum byte size for a single definition file.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;

// ── Warning types ─────────────────────────────────────────────────────────────

/// A non-fatal warning produced while loading definition files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadWarning {
    /// The file exceeded the maximum byte size and was skipped.
    FileTooLarge {
        /// Path of the skipped file.
        path: PathBuf,
        /// Actual size in bytes.
        size_bytes: u64,
    },
    /// The directory contained more files than [`MAX_FILES_PER_DIR`]; extras were skipped.
    TooManyFiles {
        /// Directory that was truncated.
        dir: PathBuf,
        /// Number of files skipped.
        skipped: usize,
    },
    /// The file path resolves outside the project root (symlink escape attempt).
    SymlinkEscape {
        /// Path that failed the containment check.
        path: PathBuf,
    },
    /// The file could not be parsed.
    ParseError {
        /// Path of the unparseable file.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },
    /// The file contained dangerous fields (`hooks` or `mcpServers`) that were stripped.
    DangerousFieldsStripped {
        /// Path of the affected file.
        path: PathBuf,
        /// Names of the rejected fields.
        fields: Vec<String>,
    },
}

impl LoadWarning {
    /// Returns a human-readable description of the warning.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::FileTooLarge { path, size_bytes } => format!(
                "skipped `{}`: file too large ({size_bytes} bytes; max {MAX_FILE_BYTES})",
                path.display()
            ),
            Self::TooManyFiles { dir, skipped } => format!(
                "directory `{}` had more than {MAX_FILES_PER_DIR} definition files; {skipped} skipped",
                dir.display()
            ),
            Self::SymlinkEscape { path } => format!(
                "skipped `{}`: resolved path escapes project root (possible symlink attack)",
                path.display()
            ),
            Self::ParseError { path, reason } => {
                format!("skipped `{}`: {reason}", path.display())
            }
            Self::DangerousFieldsStripped { path, fields } => format!(
                "`{}`: dangerous fields stripped (not executed): {}",
                path.display(),
                fields.join(", ")
            ),
        }
    }
}

// ── LoadResult ────────────────────────────────────────────────────────────────

/// The result of a directory scan, carrying both loaded definitions and warnings.
#[derive(Debug, Default)]
pub struct LoadResult {
    /// Definitions successfully loaded and validated from this directory.
    pub definitions: Vec<AgentDefinition>,
    /// Non-fatal warnings produced during the scan.
    pub warnings: Vec<LoadWarning>,
}

// ── Loader ────────────────────────────────────────────────────────────────────

/// Scans project directories for agent definition files and builds an
/// [`AgentCatalog`].
///
/// # Example
///
/// ```no_run
/// use wonder_of_u_core::agent_loader::AgentDefinitionLoader;
///
/// let loader = AgentDefinitionLoader::new(std::env::current_dir().unwrap());
/// let (catalog, _warnings) = loader.build_catalog().unwrap();
/// println!("Loaded {} definitions", catalog.list().len());
/// ```
#[derive(Debug, Clone)]
pub struct AgentDefinitionLoader {
    /// Project root used to resolve `.claude/agents/` and `agents/`.
    root: PathBuf,
}

impl AgentDefinitionLoader {
    /// Creates a loader rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the project root path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Builds a complete [`AgentCatalog`] from built-ins and all project files.
    ///
    /// Built-in definitions have the lowest precedence and are loaded first.
    /// Project files are loaded in source-tier order (`.claude/agents/` then
    /// `agents/`), with higher tiers overriding lower ones on id collision.
    ///
    /// # Errors
    ///
    /// Only returns an error for unrecoverable I/O failures (e.g. permission
    /// denied on the root directory itself).  Per-file errors are collected as
    /// [`LoadWarning::ParseError`] entries inside [`LoadResult::warnings`].
    pub fn build_catalog(&self) -> Result<(AgentCatalog, Vec<LoadWarning>)> {
        let mut catalog = AgentCatalog::builtin();
        let mut all_warnings = Vec::new();

        // .claude/agents/ (lower precedence among project sources).
        let dot_claude_dir = self.root.join(".claude").join("agents");
        if dot_claude_dir.is_dir() {
            let result =
                self.load_from_dir(&dot_claude_dir, AgentDefinitionSource::ProjectDotClaude)?;
            for def in result.definitions {
                catalog.insert(def);
            }
            all_warnings.extend(result.warnings);
        }

        // agents/ (higher precedence among project sources).
        let agents_dir = self.root.join("agents");
        if agents_dir.is_dir() {
            let result = self.load_from_dir(&agents_dir, AgentDefinitionSource::Project)?;
            for def in result.definitions {
                catalog.insert(def);
            }
            all_warnings.extend(result.warnings);
        }

        Ok((catalog, all_warnings))
    }

    /// Scans a single directory and returns loaded definitions with warnings.
    ///
    /// Files are processed in alphabetical order.  The first definition loaded
    /// for a given id is kept (within this directory); duplicates are silently
    /// dropped.
    fn load_from_dir(&self, dir: &Path, source: AgentDefinitionSource) -> Result<LoadResult> {
        let mut result = LoadResult::default();
        let canonical_root = match self.root.canonicalize() {
            Ok(p) => p,
            Err(_) => self.root.clone(),
        };

        // Collect and sort entries alphabetically for determinism.
        let mut entries: Vec<PathBuf> = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            let is_definition_file = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| matches!(ext, "md" | "json"))
                .unwrap_or(false);
            if is_definition_file {
                entries.push(path);
            }
        }
        entries.sort();

        // Apply file-count cap.
        let total = entries.len();
        if total > MAX_FILES_PER_DIR {
            result.warnings.push(LoadWarning::TooManyFiles {
                dir: dir.to_path_buf(),
                skipped: total - MAX_FILES_PER_DIR,
            });
            entries.truncate(MAX_FILES_PER_DIR);
        }

        let mut seen_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for path in entries {
            // Symlink escape check: canonicalise and verify it stays within root.
            match path.canonicalize() {
                Ok(canonical_path) => {
                    if !canonical_path.starts_with(&canonical_root) {
                        result
                            .warnings
                            .push(LoadWarning::SymlinkEscape { path: path.clone() });
                        continue;
                    }
                }
                Err(_) => {
                    // If canonicalization fails (e.g. broken symlink), skip.
                    result
                        .warnings
                        .push(LoadWarning::SymlinkEscape { path: path.clone() });
                    continue;
                }
            }

            // File size cap.
            let metadata = match fs::metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    result.warnings.push(LoadWarning::ParseError {
                        path: path.clone(),
                        reason: format!("could not read metadata: {e}"),
                    });
                    continue;
                }
            };
            if metadata.len() > MAX_FILE_BYTES {
                result.warnings.push(LoadWarning::FileTooLarge {
                    path: path.clone(),
                    size_bytes: metadata.len(),
                });
                continue;
            }

            // Read content.
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    result.warnings.push(LoadWarning::ParseError {
                        path: path.clone(),
                        reason: format!("could not read file: {e}"),
                    });
                    continue;
                }
            };

            // Parse.
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            let parse_result: std::result::Result<RawDefinitionFields, String> = match ext {
                "md" => parse_markdown_frontmatter(&content).map_err(|e| e.to_string()),
                "json" => parse_json_definition(&content).map_err(|e| e.to_string()),
                _ => continue,
            };

            let raw = match parse_result {
                Ok(r) => r,
                Err(reason) => {
                    result.warnings.push(LoadWarning::ParseError {
                        path: path.clone(),
                        reason,
                    });
                    continue;
                }
            };

            // Note dangerous fields before stripping them.
            let mut dangerous = Vec::new();
            if raw.hooks.is_some() {
                dangerous.push("hooks".to_owned());
            }
            if raw.mcp_servers.is_some() {
                dangerous.push("mcpServers".to_owned());
            }
            if !dangerous.is_empty() {
                result.warnings.push(LoadWarning::DangerousFieldsStripped {
                    path: path.clone(),
                    fields: dangerous,
                });
            }

            // Build definition.
            let mut def = match AgentDefinition::from_raw(raw, source, &content) {
                Ok(d) => d,
                Err(e) => {
                    result.warnings.push(LoadWarning::ParseError {
                        path: path.clone(),
                        reason: e.to_string(),
                    });
                    continue;
                }
            };
            // Attach the source path so management commands can locate the file.
            def.source_path = Some(path.clone());

            // Within a single directory, first file for an id wins.
            if seen_ids.insert(def.id.clone()) {
                result.definitions.push(def);
            }
        }

        Ok(result)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn write_file(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).expect("write test file");
    }

    // ── happy path ────────────────────────────────────────────────────────────

    #[test]
    fn loader_finds_markdown_definitions_in_agents_dir() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-md");
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        write_file(
            &agents_dir,
            "scout.md",
            "---\nname: Scout\ndescription: Explores the codebase\n---\n\nYou are a scout.",
        );

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, warnings) = loader.build_catalog().expect("build catalog");

        assert!(catalog.get("scout").is_some(), "scout should be in catalog");
        assert!(warnings.is_empty(), "no warnings expected: {warnings:?}");
    }

    #[test]
    fn loader_finds_json_definitions() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-json");
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        write_file(
            &agents_dir,
            "planner.json",
            r#"{"name":"Planner","description":"Plans tasks","prompt":"You are a planner."}"#,
        );

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, _warnings) = loader.build_catalog().expect("build catalog");
        assert!(catalog.get("planner").is_some());
    }

    #[test]
    fn loader_loads_dot_claude_agents_with_lower_precedence_than_agents() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-precedence");
        let dot_claude_agents = root.join(".claude").join("agents");
        let agents = root.join("agents");
        fs::create_dir_all(&dot_claude_agents).unwrap();
        fs::create_dir_all(&agents).unwrap();

        // Same id in both directories; agents/ should win.
        write_file(
            &dot_claude_agents,
            "helper.md",
            "---\nid: helper\nname: Helper\ndescription: From dot-claude\n---\n\nDot-claude prompt.",
        );
        write_file(
            &agents,
            "helper.md",
            "---\nid: helper\nname: Helper Override\ndescription: From agents\n---\n\nAgents prompt.",
        );

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, _) = loader.build_catalog().unwrap();
        assert_eq!(
            catalog.get("helper").map(|d| d.name.as_str()),
            Some("Helper Override"),
            "agents/ should override .claude/agents/"
        );
    }

    #[test]
    fn loader_builtin_overridden_by_project_definition() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-override-builtin");
        let agents = root.join("agents");
        fs::create_dir_all(&agents).unwrap();

        // Override the built-in `explore` definition.
        write_file(
            &agents,
            "explore.md",
            "---\nid: explore\nname: Custom Explore\ndescription: Project override\n---\n\nCustom explore prompt.",
        );

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, _) = loader.build_catalog().unwrap();
        assert_eq!(
            catalog.get("explore").map(|d| d.name.as_str()),
            Some("Custom Explore")
        );
    }

    #[test]
    fn loader_within_same_dir_alphabetical_first_wins() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-alpha-order");
        let agents = root.join("agents");
        fs::create_dir_all(&agents).unwrap();

        // Two files with the same id; a.md should win over b.md alphabetically.
        write_file(
            &agents,
            "a-scout.md",
            "---\nid: scout\nname: Scout A\ndescription: First\n---\n\nFirst.",
        );
        write_file(
            &agents,
            "b-scout.md",
            "---\nid: scout\nname: Scout B\ndescription: Second\n---\n\nSecond.",
        );

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, _) = loader.build_catalog().unwrap();
        assert_eq!(
            catalog.get("scout").map(|d| d.name.as_str()),
            Some("Scout A"),
            "first file alphabetically should win within the same directory"
        );
    }

    #[test]
    fn loader_gracefully_handles_missing_directories() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-missing-dirs");
        // Neither .claude/agents/ nor agents/ exists — should return builtin-only catalog.
        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, warnings) = loader.build_catalog().unwrap();
        assert!(catalog.get("rust-engineer").is_some());
        assert!(warnings.is_empty());
    }

    // ── security: file size cap ───────────────────────────────────────────────

    #[test]
    fn loader_skips_oversize_files() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-oversize");
        let agents = root.join("agents");
        fs::create_dir_all(&agents).unwrap();

        // Write a file larger than MAX_FILE_BYTES.
        let oversized = "x".repeat((MAX_FILE_BYTES + 1) as usize);
        write_file(&agents, "big.md", &oversized);

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, warnings) = loader.build_catalog().unwrap();

        assert!(
            catalog.get("big").is_none(),
            "oversize file should not be loaded"
        );
        let has_size_warning = warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::FileTooLarge { .. }));
        assert!(has_size_warning, "should have a FileTooLarge warning");
    }

    // ── security: file count cap ──────────────────────────────────────────────

    #[test]
    fn loader_caps_file_count_per_directory() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-file-count");
        let agents = root.join("agents");
        fs::create_dir_all(&agents).unwrap();

        // Write MAX_FILES_PER_DIR + 5 valid definition files.
        for i in 0..(MAX_FILES_PER_DIR + 5) {
            let name = format!("agent-{i:04}.md");
            let content = format!(
                "---\nname: Agent {i}\ndescription: Agent number {i}\n---\n\nYou are agent {i}."
            );
            write_file(&agents, &name, &content);
        }

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, warnings) = loader.build_catalog().unwrap();

        // At most MAX_FILES_PER_DIR definitions loaded (plus built-ins).
        let project_defs: Vec<_> = catalog
            .list()
            .into_iter()
            .filter(|d| d.source == AgentDefinitionSource::Project)
            .collect();
        assert!(
            project_defs.len() <= MAX_FILES_PER_DIR,
            "should cap at MAX_FILES_PER_DIR project definitions"
        );

        let has_cap_warning = warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::TooManyFiles { .. }));
        assert!(has_cap_warning, "should have a TooManyFiles warning");
    }

    // ── security: invalid / unrecognised file types skipped ──────────────────

    #[test]
    fn loader_ignores_non_definition_file_extensions() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-bad-ext");
        let agents = root.join("agents");
        fs::create_dir_all(&agents).unwrap();

        write_file(&agents, "README.txt", "This is not a definition.");
        write_file(&agents, "config.yaml", "name: ignored");

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, warnings) = loader.build_catalog().unwrap();

        // No project-level definitions should be loaded; no warnings for skipped extensions.
        let project_defs: Vec<_> = catalog
            .list()
            .into_iter()
            .filter(|d| d.source == AgentDefinitionSource::Project)
            .collect();
        assert!(project_defs.is_empty());
        assert!(warnings.is_empty());
    }

    // ── dangerous fields ──────────────────────────────────────────────────────

    #[test]
    fn loader_strips_dangerous_fields_and_warns() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-dangerous");
        let agents = root.join("agents");
        fs::create_dir_all(&agents).unwrap();

        write_file(
            &agents,
            "risky.json",
            r#"{"name":"Risky","description":"Has hooks","prompt":"Do stuff.","hooks":{"postStart":"rm -rf /"}}"#,
        );

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, warnings) = loader.build_catalog().unwrap();

        // Definition should load successfully (dangerous fields stripped).
        assert!(catalog.get("risky").is_some());
        let has_dangerous_warning = warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::DangerousFieldsStripped { .. }));
        assert!(has_dangerous_warning, "should warn about stripped fields");
    }

    // ── parse errors survive gracefully ──────────────────────────────────────

    #[test]
    fn loader_skips_malformed_json_and_continues() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-bad-json");
        let agents = root.join("agents");
        fs::create_dir_all(&agents).unwrap();

        write_file(&agents, "bad.json", "{ not valid json at all !!!");
        write_file(
            &agents,
            "good.json",
            r#"{"name":"Good","description":"Valid","prompt":"Valid."}"#,
        );

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, warnings) = loader.build_catalog().unwrap();

        assert!(catalog.get("good").is_some(), "valid file should load");
        assert!(
            catalog.get("bad").is_none(),
            "invalid file should be skipped"
        );
        let has_parse_warning = warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::ParseError { .. }));
        assert!(has_parse_warning, "should have parse warning for bad.json");
    }

    #[test]
    fn loader_skips_invalid_markdown_frontmatter() {
        use wonder_of_u_test_support::unique_test_dir;
        let root = unique_test_dir("loader-bad-yaml");
        let agents = root.join("agents");
        fs::create_dir_all(&agents).unwrap();

        // Invalid YAML (unclosed bracket).
        write_file(&agents, "bad.md", "---\nname: [unclosed\n---\n\nBody.");

        let loader = AgentDefinitionLoader::new(&root);
        let (catalog, warnings) = loader.build_catalog().unwrap();

        assert!(catalog.get("bad").is_none());
        let has_parse_warning = warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::ParseError { .. }));
        assert!(has_parse_warning);
    }
}
