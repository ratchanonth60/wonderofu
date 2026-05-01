use std::path::PathBuf;

use crate::manifest::SkillManifest;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundledSkill {
    pub manifest: SkillManifest,
    pub prompt: String,
}

impl BundledSkill {
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

#[must_use]
pub fn bundled_skills() -> Vec<BundledSkill> {
    vec![BundledSkill::inline(
        "workspace-audit",
        "Summarize the current Rust workspace layout and recommend validation commands.",
        "Inspect the current repository as a Rust workspace. List the main crates, note any plugin or skill metadata that was discovered, and finish by recommending the exact cargo validation commands that should be run before shipping changes.",
    )]
}
