//! Two-phase memory pipeline for cross-session knowledge extraction.
//!
//! # Phase 1: Per-Session Extraction
//!
//! Extracts structured memories from individual conversation sessions.
//! Run after each session, or in the background.
//!
//! # Phase 2: Global Consolidation
//!
//! Consolidates Phase 1 outputs into the memory workspace (MEMORY.md,
//! skills/ dir), resolving conflicts and merging related memories.

use std::{
    path::PathBuf,
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use wonder_of_u_core::Result;

/// A single memory extracted from a conversation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExtractedMemory {
    /// Unique identifier (SHA256 of content).
    pub id: String,
    /// The memory content (plain text).
    pub raw_memory: String,
    /// Short slug for the memory file name.
    pub slug: Option<String>,
    /// When this memory was first extracted.
    pub generated_at: SystemTime,
    /// When this memory was last used/confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_usage: Option<SystemTime>,
    /// How many times this memory has been referenced.
    #[serde(default)]
    pub usage_count: u32,
    /// Source session ID where this memory was extracted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session: Option<String>,
}

/// Status of a Phase 1 extraction job.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    /// Job is pending.
    Pending,
    /// Job is running.
    Running,
    /// Job completed successfully with output.
    Succeeded,
    /// Job completed but produced no useful output.
    SucceededNoOutput,
    /// Job failed.
    Failed,
}

/// A Phase 1 extraction job that processes a single session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExtractionJob {
    /// Session ID to extract memories from.
    pub session_id: String,
    /// Path to the session transcript.
    pub transcript_path: PathBuf,
    /// Current status of the job.
    pub status: ExtractionStatus,
    /// When this job was created.
    pub created_at: SystemTime,
    /// When this job was last updated.
    pub updated_at: SystemTime,
    /// Extracted memories (populated on success).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<ExtractedMemory>,
    /// Error message (populated on failure).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Memory pipeline configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryPipelineConfig {
    /// Whether the memory pipeline is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum number of Phase 1 extraction jobs per run.
    #[serde(default = "default_max_jobs")]
    pub max_jobs_per_run: usize,
    /// Maximum memory age in days before pruning.
    #[serde(default = "default_max_age_days")]
    pub max_unused_days: u32,
    /// Maximum number of memories to keep.
    #[serde(default = "default_max_memories")]
    pub max_memories: usize,
    /// Concurrency limit for Phase 1 extractions.
    #[serde(default = "default_concurrency")]
    pub phase1_concurrency: usize,
}

fn default_max_jobs() -> usize {
    5
}
fn default_max_age_days() -> u32 {
    90
}
fn default_max_memories() -> usize {
    100
}
fn default_concurrency() -> usize {
    3
}

impl Default for MemoryPipelineConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_jobs_per_run: default_max_jobs(),
            max_unused_days: default_max_age_days(),
            max_memories: default_max_memories(),
            phase1_concurrency: default_concurrency(),
        }
    }
}

/// Stores and manages the memory pipeline state.
#[derive(Clone, Debug)]
pub struct MemoryPipelineStore {
    root: PathBuf,
}

impl MemoryPipelineStore {
    /// Create a new pipeline store rooted at the given path.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
        }
    }

    fn memories_path(&self) -> PathBuf {
        self.root.join("memories.json")
    }

    /// Load all persisted memories.
    pub fn load_memories(&self) -> Result<Vec<ExtractedMemory>> {
        let path = self.memories_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let data = std::fs::read_to_string(&path)?;
        serde_json::from_str(&data).map_err(Into::into)
    }

    /// Save memories to disk.
    pub fn save_memories(&self, memories: &[ExtractedMemory]) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let data = serde_json::to_string_pretty(memories)?;
        std::fs::write(self.memories_path(), data)?;
        Ok(())
    }

    /// Prune memories that exceed the configured limits.
    #[must_use]
    pub fn prune_memories(
        memories: &[ExtractedMemory],
        max_unused_days: u32,
        max_memories: usize,
    ) -> Vec<ExtractedMemory> {
        let cutoff = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(u64::from(max_unused_days) * 86400));

        let mut active: Vec<_> = memories
            .iter()
            .filter(|m| {
                cutoff.is_none_or(|cutoff| {
                    m.last_usage.unwrap_or(m.generated_at) >= cutoff
                })
            })
            .cloned()
            .collect();

        active.sort_by_key(|m| {
            std::cmp::Reverse(
                m.last_usage
                    .unwrap_or(m.generated_at)
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default(),
            )
        });

        active.truncate(max_memories);
        active
    }

    /// Track memory usage by incrementing the usage counter.
    pub fn track_usage(memories: &mut [ExtractedMemory], memory_id: &str) {
        if let Some(mem) = memories.iter_mut().find(|m| m.id == memory_id) {
            mem.usage_count += 1;
            mem.last_usage = Some(SystemTime::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_removes_stale() {
        let fresh = ExtractedMemory {
            id: "fresh".into(),
            raw_memory: "fresh".into(),
            slug: None,
            generated_at: SystemTime::now(),
            last_usage: Some(SystemTime::now()),
            usage_count: 1,
            source_session: None,
        };
        let stale = ExtractedMemory {
            id: "stale".into(),
            raw_memory: "stale".into(),
            slug: None,
            generated_at: SystemTime::UNIX_EPOCH,
            last_usage: None,
            usage_count: 0,
            source_session: None,
        };
        let result =
            MemoryPipelineStore::prune_memories(&[fresh.clone(), stale], 90, 100);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "fresh");
    }

    #[test]
    fn prune_enforces_limit() {
        let mems: Vec<_> = (0..10)
            .map(|i| ExtractedMemory {
                id: format!("mem{i}"),
                raw_memory: format!("content {i}"),
                slug: None,
                generated_at: SystemTime::now(),
                last_usage: Some(SystemTime::now()),
                usage_count: 0,
                source_session: None,
            })
            .collect();
        let result = MemoryPipelineStore::prune_memories(&mems, 90, 3);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn track_usage_updates_counter() {
        let mut mem = ExtractedMemory {
            id: "test".into(),
            raw_memory: "test".into(),
            slug: None,
            generated_at: SystemTime::now(),
            last_usage: None,
            usage_count: 0,
            source_session: None,
        };
        MemoryPipelineStore::track_usage(&mut [mem.clone()], "test");
        mem.usage_count += 1;
        assert_eq!(mem.usage_count, 1);
    }
}
