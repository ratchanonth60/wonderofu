//! Context fragment assembly for token-limited prompt construction.
//!
//! Defines piecemeal context fragments and an assembler that composes them
//! into the model prompt while respecting token budgets. Inspired by codex-rs
//! `context-fragments` crate.
//!
//! # Usage
//!
//! ```ignore
//! let mut assembler = ContextAssembler::new(100_000);
//! assembler.add_file_fragment("/src/main.rs", content, 80);
//! assembler.add_git_status(status_output, 60);
//! let prompt_context = assembler.assemble();
//! ```

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Where a context fragment originated.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FragmentSource {
    /// Content from a file in the workspace.
    File {
        /// Absolute or relative path.
        path: PathBuf,
        /// Line range if reading a portion.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line_range: Option<(usize, usize)>,
    },
    /// Project-level metadata (Cargo.toml, package.json, etc.).
    ProjectMetadata,
    /// Current git status output.
    GitStatus,
    /// Summary of a previous conversation.
    ConversationSummary {
        /// Session this summary was extracted from.
        session_id: String,
    },
    /// A memory entry from the memory pipeline.
    Memory {
        /// Memory ID from the pipeline store.
        memory_id: String,
    },
    /// Output from a tool execution.
    ToolOutput {
        /// Name of the tool that produced this output.
        tool_name: String,
    },
    /// Directory listing of the workspace.
    DirectoryListing,
    /// Active linter/compiler diagnostics.
    Diagnostics {
        /// Source of the diagnostics (e.g. "rustc", "clippy").
        source: String,
    },
    /// Custom/user-defined fragment.
    Custom {
        /// Human-readable label for this fragment.
        label: String,
    },
}

/// A single piece of context that contributes to the model prompt.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextFragment {
    /// Unique identifier for deduplication.
    pub id: String,
    /// Where this fragment came from.
    pub source: FragmentSource,
    /// The actual text content.
    pub content: String,
    /// Priority (0 = lowest, 255 = highest).
    /// Higher-priority fragments are included first.
    pub priority: u8,
    /// Estimated token count for this fragment.
    pub token_count: usize,
    /// Whether this fragment is mandatory (always included).
    #[serde(default)]
    pub mandatory: bool,
}

/// Strategy for assembling fragments when under token pressure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssemblyStrategy {
    /// Include highest priority fragments first.
    #[default]
    PriorityFirst,
    /// Deduplicate and keep the most recent version.
    LatestFirst,
    /// Spread fragments across different sources for diversity.
    Diverse,
}

/// Assembles context fragments into a token-limited prompt context.
#[derive(Clone, Debug)]
pub struct ContextAssembler {
    /// All collected fragments.
    fragments: Vec<ContextFragment>,
    /// Maximum token budget.
    max_tokens: usize,
    /// Assembly strategy.
    strategy: AssemblyStrategy,
}

impl ContextAssembler {
    /// Create a new assembler with the given token budget.
    #[must_use]
    pub fn new(max_tokens: usize) -> Self {
        Self {
            fragments: Vec::new(),
            max_tokens,
            strategy: AssemblyStrategy::default(),
        }
    }

    /// Set the assembly strategy.
    pub fn with_strategy(&mut self, strategy: AssemblyStrategy) -> &mut Self {
        self.strategy = strategy;
        self
    }

    /// Add a fragment. Returns whether it was added (false if duplicate).
    pub fn add(&mut self, fragment: ContextFragment) -> bool {
        if self.fragments.iter().any(|f| f.id == fragment.id) {
            return false;
        }
        self.fragments.push(fragment);
        true
    }

    /// Add a file content fragment.
    pub fn add_file_fragment(
        &mut self,
        path: impl Into<PathBuf>,
        content: impl Into<String>,
        priority: u8,
    ) {
        let path = path.into();
        let content = content.into();
        let id = format!("file:{}", path.display());
        let token_count = estimate_tokens(&content);
        self.add(ContextFragment {
            id,
            source: FragmentSource::File {
                path,
                line_range: None,
            },
            content,
            priority,
            token_count,
            mandatory: false,
        });
    }

    /// Add a git status fragment.
    pub fn add_git_status(&mut self, status: impl Into<String>, priority: u8) {
        let content = status.into();
        let token_count = estimate_tokens(&content);
        self.add(ContextFragment {
            id: "git:status".into(),
            source: FragmentSource::GitStatus,
            content,
            priority,
            token_count,
            mandatory: false,
        });
    }

    /// Add a project metadata fragment.
    pub fn add_project_metadata(&mut self, metadata: impl Into<String>, priority: u8) {
        let content = metadata.into();
        let token_count = estimate_tokens(&content);
        self.add(ContextFragment {
            id: "project:metadata".into(),
            source: FragmentSource::ProjectMetadata,
            content,
            priority,
            token_count,
            mandatory: false,
        });
    }

    /// Add a memory fragment.
    pub fn add_memory(
        &mut self,
        memory_id: impl Into<String>,
        content: impl Into<String>,
        priority: u8,
    ) {
        let memory_id = memory_id.into();
        let content = content.into();
        let token_count = estimate_tokens(&content);
        self.add(ContextFragment {
            id: format!("memory:{memory_id}"),
            source: FragmentSource::Memory { memory_id },
            content,
            priority,
            token_count,
            mandatory: false,
        });
    }

    /// Add a diagnostics fragment from an LSP source (e.g. `"rust-analyzer"`).
    pub fn add_diagnostics_fragment(
        &mut self,
        source: impl Into<String>,
        content: impl Into<String>,
        priority: u8,
    ) {
        let source = source.into();
        let content = content.into();
        let id = format!("diagnostics:{source}");
        let token_count = estimate_tokens(&content);
        self.add(ContextFragment {
            id,
            source: FragmentSource::Diagnostics { source },
            content,
            priority,
            token_count,
            mandatory: false,
        });
    }

    /// Total token count of all fragments.
    #[must_use]
    pub fn total_tokens(&self) -> usize {
        self.fragments.iter().map(|f| f.token_count).sum()
    }

    /// Assemble fragments into a single context string, respecting token budget.
    #[must_use]
    pub fn assemble(&self) -> String {
        let mut sorted = self.fragments.clone();
        match self.strategy {
            AssemblyStrategy::PriorityFirst => {
                sorted.sort_by_key(|f| std::cmp::Reverse(f.priority));
            }
            AssemblyStrategy::LatestFirst => {
                sorted.reverse();
            }
            AssemblyStrategy::Diverse => {
                // Group by source kind, pick top from each group in round-robin
                let mut grouped: HashMap<String, Vec<&ContextFragment>> = HashMap::new();
                for f in &self.fragments {
                    let key = source_group_key(&f.source);
                    grouped.entry(key).or_default().push(f);
                }
                sorted.clear();
                loop {
                    let mut added = false;
                    for group in grouped.values_mut() {
                        if let Some(f) = group.pop() {
                            sorted.push(f.clone());
                            added = true;
                        }
                    }
                    if !added {
                        break;
                    }
                }
            }
        }

        let mut parts: Vec<String> = Vec::new();
        let mut used = 0;
        for frag in &sorted {
            if frag.mandatory || used + frag.token_count <= self.max_tokens {
                parts.push(frag.content.clone());
                used += frag.token_count;
            }
        }

        parts.join("\n\n")
    }

    /// Estimate tokens for each fragment and return total if assembled.
    #[must_use]
    pub fn estimate_total_if_assembled(&self) -> usize {
        let mut sorted = self.fragments.clone();
        sorted.sort_by_key(|f| std::cmp::Reverse(f.priority));
        let mut used = 0;
        for frag in &sorted {
            if used + frag.token_count > self.max_tokens {
                break;
            }
            used += frag.token_count;
        }
        used
    }
}

fn source_group_key(source: &FragmentSource) -> String {
    match source {
        FragmentSource::File { .. } => "file".into(),
        FragmentSource::ProjectMetadata => "project".into(),
        FragmentSource::GitStatus => "git".into(),
        FragmentSource::ConversationSummary { .. } => "conversation".into(),
        FragmentSource::Memory { .. } => "memory".into(),
        FragmentSource::ToolOutput { .. } => "tool".into(),
        FragmentSource::DirectoryListing => "directory".into(),
        FragmentSource::Diagnostics { .. } => "diagnostics".into(),
        FragmentSource::Custom { .. } => "custom".into(),
    }
}

/// Quick token estimation heuristic (4 chars ≈ 1 token for English text).
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_rounds_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("1234"), 1);
        assert_eq!(estimate_tokens("12345"), 2);
    }

    #[test]
    fn assembler_respects_budget() {
        let mut a = ContextAssembler::new(25);
        a.add(ContextFragment {
            id: "a".into(),
            source: FragmentSource::Custom {
                label: "test".into(),
            },
            content: "low priority, small".into(),
            priority: 50,
            token_count: 2,
            mandatory: false,
        });
        a.add(ContextFragment {
            id: "b".into(),
            source: FragmentSource::Custom {
                label: "test".into(),
            },
            content: "HIGH PRIORITY".into(),
            priority: 100,
            token_count: 3,
            mandatory: false,
        });
        let result = a.assemble();
        // Highest priority included first
        assert!(result.contains("HIGH PRIORITY"));
    }

    #[test]
    fn mandatory_fragments_always_included() {
        let mut a = ContextAssembler::new(5);
        a.add(ContextFragment {
            id: "mandatory".into(),
            source: FragmentSource::Custom {
                label: "req".into(),
            },
            content: "required context".into(),
            priority: 0,
            token_count: 20,
            mandatory: true,
        });
        a.add(ContextFragment {
            id: "optional".into(),
            source: FragmentSource::Custom {
                label: "opt".into(),
            },
            content: "optional".into(),
            priority: 100,
            token_count: 2,
            mandatory: false,
        });
        let result = a.assemble();
        assert!(result.contains("required")); // mandatory
    }

    #[test]
    fn diverse_strategy_interleaves_sources() {
        let mut a = ContextAssembler::new(1000);
        a.with_strategy(AssemblyStrategy::Diverse);
        a.add(ContextFragment {
            id: "f1".into(),
            source: FragmentSource::File {
                path: "a.rs".into(),
                line_range: None,
            },
            content: "file-a".into(),
            priority: 10,
            token_count: 1,
            mandatory: false,
        });
        a.add(ContextFragment {
            id: "f2".into(),
            source: FragmentSource::File {
                path: "b.rs".into(),
                line_range: None,
            },
            content: "file-b".into(),
            priority: 10,
            token_count: 1,
            mandatory: false,
        });
        a.add(ContextFragment {
            id: "g".into(),
            source: FragmentSource::GitStatus,
            content: "git".into(),
            priority: 10,
            token_count: 1,
            mandatory: false,
        });
        let result = a.assemble();
        // Should have content from 3 different fragments
        assert!(result.contains("file"));
        assert!(result.contains("git"));
    }
}
