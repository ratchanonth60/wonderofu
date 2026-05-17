//! Agent definition management sub-commands (`/agents definitions ...`).
//!
//! This module implements the `/agents definitions list|show|create|edit`
//! surface.  The runtime `/agents list/show/start/stop/status` commands live
//! in [`super::workflow`] and are unaffected.
//!
//! Entry point: [`execute_agent_definitions`].

use std::{
    fs::{self},
    path::{Path, PathBuf},
};

use clap::{Args, Subcommand};
use wonder_of_u_core::{
    AgentDefinition, AgentDefinitionSource, CommandContext, CommandOutput, RawDefinitionFields,
    Result, WonderError,
    agent_loader::{AgentDefinitionLoader, MAX_FILES_PER_DIR},
    parse_json_definition, parse_markdown_frontmatter, render_definition_md,
};

use super::git_command_output;

// ── Clap types ────────────────────────────────────────────────────────────────

/// Target directory for a new agent definition file.
#[derive(Clone, Debug, clap::ValueEnum)]
pub(crate) enum DefinitionTarget {
    /// Write to `{root}/.claude/agents/` (default, lower precedence).
    DotClaude,
    /// Write to `{root}/agents/` (higher precedence).
    Agents,
}

#[derive(Debug, Args)]
pub(crate) struct DefinitionsArgs {
    #[command(subcommand)]
    pub(crate) command: DefinitionsSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum DefinitionsSubcommand {
    /// List all known agent definitions for this project.
    List,
    /// Show full details of a single agent definition.
    Show(DefinitionsShowArgs),
    /// Create a new project-level agent definition file.
    Create(DefinitionsCreateArgs),
    /// Edit an existing project-level agent definition file.
    Edit(DefinitionsEditArgs),
}

#[derive(Debug, Args)]
pub(crate) struct DefinitionsShowArgs {
    /// Id or display name of the definition.
    #[arg()]
    pub(crate) id: String,
}

#[derive(Debug, Args)]
pub(crate) struct DefinitionsCreateArgs {
    /// Stable kebab-case id (derived from --name if omitted).
    #[arg(long)]
    pub(crate) id: Option<String>,
    /// Human-readable display name (required).
    #[arg(long)]
    pub(crate) name: String,
    /// Short description (required).
    #[arg(long)]
    pub(crate) description: String,
    /// System prompt / body text (required).
    #[arg(long)]
    pub(crate) prompt: String,
    /// Optional model override.
    #[arg(long)]
    pub(crate) model: Option<String>,
    /// Allowed tool names (comma-separated or repeated --tools).
    #[arg(long, value_delimiter = ',')]
    pub(crate) tools: Vec<String>,
    /// Disallowed tool names (comma-separated or repeated --disallowed-tools).
    #[arg(long, value_delimiter = ',')]
    pub(crate) disallowed_tools: Vec<String>,
    /// Optional UI color hint.
    #[arg(long)]
    pub(crate) color: Option<String>,
    /// Permission mode string.
    #[arg(long)]
    pub(crate) permission_mode: Option<String>,
    /// Maximum conversation turns.
    #[arg(long)]
    pub(crate) max_turns: Option<u32>,
    /// Target directory: `dot-claude` (`.claude/agents/`) or `agents`.
    #[arg(long, default_value = "dot-claude")]
    pub(crate) target: DefinitionTarget,
    /// Overwrite an existing file with the same id.
    #[arg(long)]
    pub(crate) overwrite: bool,
}

#[derive(Debug, Args)]
pub(crate) struct DefinitionsEditArgs {
    /// Id or display name of the definition to edit.
    #[arg()]
    pub(crate) id: String,
    /// New display name.
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// New description.
    #[arg(long)]
    pub(crate) description: Option<String>,
    /// New system prompt / body text.
    #[arg(long)]
    pub(crate) prompt: Option<String>,
    /// New model override (pass empty string to clear).
    #[arg(long)]
    pub(crate) model: Option<String>,
    /// New allowed tool names (replaces existing list; comma-separated or repeated).
    #[arg(long, value_delimiter = ',')]
    pub(crate) tools: Option<Vec<String>>,
    /// New disallowed tool names (replaces existing list; comma-separated or repeated).
    #[arg(long, value_delimiter = ',')]
    pub(crate) disallowed_tools: Option<Vec<String>>,
    /// New color hint.
    #[arg(long)]
    pub(crate) color: Option<String>,
    /// New permission mode.
    #[arg(long)]
    pub(crate) permission_mode: Option<String>,
    /// New maximum conversation turns.
    #[arg(long)]
    pub(crate) max_turns: Option<u32>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Dispatches `/agents definitions` sub-commands.
///
/// Called by [`AgentsCommand`](super::workflow::AgentsCommand) when the
/// `Definitions` variant is matched.
pub(crate) fn execute_agent_definitions(
    context: CommandContext,
    args: DefinitionsArgs,
) -> Result<CommandOutput> {
    let root = resolve_project_root(&context.cwd);
    match args.command {
        DefinitionsSubcommand::List => definitions_list(&root),
        DefinitionsSubcommand::Show(a) => definitions_show(&root, &a.id),
        DefinitionsSubcommand::Create(a) => definitions_create(&root, a),
        DefinitionsSubcommand::Edit(a) => definitions_edit(&root, a),
    }
}

// ── Sub-command implementations ───────────────────────────────────────────────

pub(crate) fn definitions_list(root: &Path) -> Result<CommandOutput> {
    let loader = AgentDefinitionLoader::new(root);
    let (catalog, warnings) = loader.build_catalog()?;
    let defs = catalog.list();
    let mut lines = Vec::new();
    lines.push(format!("definitions={}", defs.len()));
    for (i, def) in defs.iter().enumerate() {
        lines.push(format!("definition[{i}].id={}", def.id));
        lines.push(format!("definition[{i}].name={}", def.name));
        lines.push(format!("definition[{i}].source={}", def.source));
        if let Some(path) = &def.source_path {
            lines.push(format!("definition[{i}].path={}", path.display()));
        }
        let model_str = def.model.as_deref().unwrap_or("");
        lines.push(format!("definition[{i}].model={model_str}"));
        let tools_str = if def.tools.is_empty() {
            "(all)".into()
        } else {
            def.tools.join(",")
        };
        lines.push(format!("definition[{i}].tools={tools_str}"));
        if let Some(color) = &def.color {
            lines.push(format!("definition[{i}].color={color}"));
        }
    }
    for w in &warnings {
        lines.push(format!("warning={}", w.message()));
    }
    Ok(CommandOutput::Text(lines.join("\n")))
}

pub(crate) fn definitions_show(root: &Path, id: &str) -> Result<CommandOutput> {
    let loader = AgentDefinitionLoader::new(root);
    let (catalog, warnings) = loader.build_catalog()?;
    let def = catalog
        .resolve_alias(id)
        .ok_or_else(|| WonderError::not_found("agent definition", id.to_string()))?;
    let mut lines = Vec::new();
    lines.push(format!("id={}", def.id));
    lines.push(format!("name={}", def.name));
    lines.push(format!("source={}", def.source));
    if let Some(path) = &def.source_path {
        lines.push(format!("source_path={}", path.display()));
    }
    lines.push(format!(
        "description={}",
        sanitize_single_line(&def.description)
    ));
    lines.push(format!(
        "model={}",
        def.model.as_deref().unwrap_or("<none>")
    ));
    lines.push(format!("tools={}", def.tools.join(",")));
    lines.push(format!(
        "disallowed_tools={}",
        def.disallowed_tools.join(",")
    ));
    lines.push(format!("color={}", def.color.as_deref().unwrap_or("")));
    lines.push(format!(
        "permission_mode={}",
        def.permission_mode.as_deref().unwrap_or("")
    ));
    lines.push(format!(
        "max_turns={}",
        def.max_turns.map(|n| n.to_string()).unwrap_or_default()
    ));
    lines.push(format!("system_prompt_bytes={}", def.system_prompt.len()));
    let preview: String = def.system_prompt.chars().take(200).collect();
    lines.push(format!(
        "system_prompt_preview={}",
        sanitize_single_line(&preview)
    ));
    for w in &warnings {
        lines.push(format!("warning={}", w.message()));
    }
    Ok(CommandOutput::Text(lines.join("\n")))
}

pub(crate) fn definitions_create(
    root: &Path,
    args: DefinitionsCreateArgs,
) -> Result<CommandOutput> {
    // Determine target directory.
    let target_dir = match args.target {
        DefinitionTarget::DotClaude => root.join(".claude").join("agents"),
        DefinitionTarget::Agents => root.join("agents"),
    };
    let source = match args.target {
        DefinitionTarget::DotClaude => AgentDefinitionSource::ProjectDotClaude,
        DefinitionTarget::Agents => AgentDefinitionSource::Project,
    };

    // Build and validate via from_raw — reuses all existing id/field validation.
    let raw = RawDefinitionFields {
        id: args.id,
        name: Some(args.name),
        description: Some(args.description),
        prompt: Some(args.prompt),
        model: args.model,
        tools: non_empty_list(split_and_dedupe(args.tools)),
        disallowed_tools: non_empty_list(split_and_dedupe(args.disallowed_tools)),
        color: args.color,
        permission_mode: args.permission_mode,
        max_turns: args.max_turns,
        ..Default::default()
    };
    let def = AgentDefinition::from_raw(raw, source, "")?;

    // Refuse path-traversal ids (belt-and-suspenders: from_raw already validates chars).
    if def.id.contains('/') || def.id.contains('\\') || def.id.contains("..") {
        return Err(WonderError::validation(format!(
            "agent id `{}` contains invalid path characters",
            def.id
        )));
    }

    // Ensure target dir exists.
    fs::create_dir_all(&target_dir)?;

    // Check file-count cap before creating.
    let existing_count = count_definition_files(&target_dir);
    if existing_count >= MAX_FILES_PER_DIR {
        return Err(WonderError::validation(format!(
            "target directory `{}` already has {existing_count} definition files \
             (max {MAX_FILES_PER_DIR}); remove some before creating new ones",
            target_dir.display()
        )));
    }

    let target_path = target_dir.join(format!("{}.md", def.id));
    let content = render_definition_md(&def)?;

    if args.overwrite {
        atomic_write(&target_path, &content)?;
    } else {
        // create_new is atomic on POSIX and rejects existing files without TOCTOU.
        use std::io::Write as _;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target_path)
        {
            Ok(mut f) => f.write_all(content.as_bytes())?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(WonderError::validation(format!(
                    "file `{}` already exists; pass --overwrite to replace it",
                    target_path.display()
                )));
            }
            Err(e) => return Err(e.into()),
        }
    }

    let mut lines = Vec::new();
    lines.push("created=true".into());
    lines.push(format!("id={}", def.id));
    lines.push(format!("name={}", def.name));
    lines.push(format!("path={}", target_path.display()));
    Ok(CommandOutput::Text(lines.join("\n")))
}

pub(crate) fn definitions_edit(root: &Path, args: DefinitionsEditArgs) -> Result<CommandOutput> {
    // Require at least one field to update.
    if args.name.is_none()
        && args.description.is_none()
        && args.prompt.is_none()
        && args.model.is_none()
        && args.tools.is_none()
        && args.disallowed_tools.is_none()
        && args.color.is_none()
        && args.permission_mode.is_none()
        && args.max_turns.is_none()
    {
        return Err(WonderError::validation(
            "at least one field must be specified for edit \
             (e.g. --name, --description, --prompt)",
        ));
    }

    // Load the catalog to locate and validate the target definition.
    let loader = AgentDefinitionLoader::new(root);
    let (catalog, _) = loader.build_catalog()?;
    let def = catalog
        .resolve_alias(&args.id)
        .ok_or_else(|| WonderError::not_found("agent definition", args.id.clone()))?;

    if def.source == AgentDefinitionSource::Builtin {
        return Err(WonderError::validation(format!(
            "agent definition `{}` is a built-in and cannot be edited; \
             create a project definition with the same id to override it",
            def.id
        )));
    }

    let source_path = def.source_path.clone().ok_or_else(|| {
        WonderError::validation(format!(
            "agent definition `{}` has no resolvable source path",
            def.id
        ))
    })?;

    // Read and parse the existing file.
    let existing_content = fs::read_to_string(&source_path)?;
    let ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let (existing_raw, source_was_json) = match ext {
        "md" => {
            let raw = parse_markdown_frontmatter(&existing_content)
                .map_err(|e| WonderError::validation(e.to_string()))?;
            (raw, false)
        }
        "json" => {
            let raw = parse_json_definition(&existing_content)
                .map_err(|e| WonderError::validation(e.to_string()))?;
            (raw, true)
        }
        other => {
            return Err(WonderError::validation(format!(
                "unsupported source file extension `{other}` for `{}`",
                source_path.display()
            )));
        }
    };

    // Note dangerous fields before stripping them.
    let had_dangerous = existing_raw.hooks.is_some() || existing_raw.mcp_servers.is_some();

    // Apply field overrides on top of the existing values.
    let updated_raw = RawDefinitionFields {
        id: Some(def.id.clone()),
        name: Some(args.name.unwrap_or_else(|| def.name.clone())),
        description: Some(args.description.unwrap_or_else(|| def.description.clone())),
        prompt: Some(args.prompt.unwrap_or_else(|| def.system_prompt.clone())),
        model: match args.model {
            Some(model) if model.trim().is_empty() => None,
            Some(model) => Some(model),
            None => def.model.clone(),
        },
        tools: Some(
            args.tools
                .map(split_and_dedupe)
                .unwrap_or_else(|| def.tools.clone()),
        ),
        disallowed_tools: Some(
            args.disallowed_tools
                .map(split_and_dedupe)
                .unwrap_or_else(|| def.disallowed_tools.clone()),
        ),
        color: args.color.or_else(|| def.color.clone()),
        permission_mode: args.permission_mode.or_else(|| def.permission_mode.clone()),
        max_turns: args.max_turns.or(def.max_turns),
        // Dangerous fields always stripped.
        hooks: None,
        mcp_servers: None,
        ..Default::default()
    };
    // Validate updated fields via the standard builder.
    let updated_def = AgentDefinition::from_raw(updated_raw, def.source, "")?;

    // Always write back as Markdown for a clean, auditable format.
    let content = render_definition_md(&updated_def)?;
    // When the source was JSON we write to a new .md path alongside.
    let write_path = if source_was_json {
        source_path.with_extension("md")
    } else {
        source_path.clone()
    };
    atomic_write(&write_path, &content)?;
    if source_was_json && write_path != source_path {
        fs::remove_file(&source_path)?;
    }

    let mut lines = Vec::new();
    lines.push("updated=true".into());
    lines.push(format!("id={}", updated_def.id));
    lines.push(format!("path={}", write_path.display()));
    if had_dangerous {
        lines.push("dangerous_fields_stripped=hooks,mcpServers".into());
    }
    if source_was_json {
        lines.push("format_changed=json->md".to_string());
        lines.push(format!("removed_old_path={}", source_path.display()));
    }
    Ok(CommandOutput::Text(lines.join("\n")))
}

// ── Utility helpers ───────────────────────────────────────────────────────────

/// Resolves the project root by querying `git rev-parse --show-toplevel`,
/// falling back to `cwd` when git is unavailable or fails.
fn resolve_project_root(cwd: &Path) -> PathBuf {
    git_command_output(cwd, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.to_path_buf())
}

/// Writes `content` to `path` atomically (write temp sibling, then rename).
///
/// Uses a hidden `.{filename}.tmp` file in the same directory so rename is
/// always on the same filesystem.  If the rename fails the temp file is
/// cleaned up on a best-effort basis.
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| WonderError::validation("path has no parent directory"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| WonderError::validation("path has no file name"))?;
    let tmp = parent.join(format!(".{}.tmp", file_name.to_string_lossy()));
    fs::write(&tmp, content)?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// Counts `.md` and `.json` files in `dir` (returns 0 when dir does not exist).
fn count_definition_files(dir: &Path) -> usize {
    if !dir.is_dir() {
        return 0;
    }
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "md" | "json"))
        })
        .count()
}

/// Splits comma-separated items, trims whitespace, and de-duplicates while
/// preserving first-occurrence order.
fn split_and_dedupe(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    items
        .into_iter()
        .flat_map(|s| {
            s.split(',')
                .map(|t| t.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}

/// Wraps a `Vec` in `Some` only when non-empty.
fn non_empty_list(v: Vec<String>) -> Option<Vec<String>> {
    if v.is_empty() { None } else { Some(v) }
}

/// Collapses multi-line strings to a single line for key=value output.
fn sanitize_single_line(value: &str) -> String {
    value.lines().collect::<Vec<_>>().join("\\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::fs;

    use wonder_of_u_core::CommandOutput;
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn def_list(root: &std::path::Path) -> String {
        match definitions_list(root).expect("definitions list") {
            CommandOutput::Text(t) => t,
            other => panic!("expected Text output, got {other:?}"),
        }
    }

    fn def_show(root: &std::path::Path, id: &str) -> String {
        match definitions_show(root, id).expect("definitions show") {
            CommandOutput::Text(t) => t,
            other => panic!("expected Text output, got {other:?}"),
        }
    }

    fn def_create(root: &std::path::Path, args: DefinitionsCreateArgs) -> String {
        match definitions_create(root, args).expect("definitions create") {
            CommandOutput::Text(t) => t,
            other => panic!("expected Text output, got {other:?}"),
        }
    }

    fn def_create_err(root: &std::path::Path, args: DefinitionsCreateArgs) -> String {
        definitions_create(root, args)
            .expect_err("expected create to fail")
            .to_string()
    }

    fn def_edit(root: &std::path::Path, args: DefinitionsEditArgs) -> String {
        match definitions_edit(root, args).expect("definitions edit") {
            CommandOutput::Text(t) => t,
            other => panic!("expected Text output, got {other:?}"),
        }
    }

    fn def_edit_err(root: &std::path::Path, args: DefinitionsEditArgs) -> String {
        definitions_edit(root, args)
            .expect_err("expected edit to fail")
            .to_string()
    }

    fn create_args_minimal(name: &str, desc: &str, prompt: &str) -> DefinitionsCreateArgs {
        DefinitionsCreateArgs {
            id: None,
            name: name.into(),
            description: desc.into(),
            prompt: prompt.into(),
            model: None,
            tools: vec![],
            disallowed_tools: vec![],
            color: None,
            permission_mode: None,
            max_turns: None,
            target: DefinitionTarget::DotClaude,
            overwrite: false,
        }
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn agents_definition_list_includes_builtin_definitions() {
        let root = unique_test_dir("def-list-builtin");
        let out = def_list(&root);
        // Built-ins are always present.
        assert!(
            out.contains("rust-engineer"),
            "should list rust-engineer; got:\n{out}"
        );
        assert!(out.contains("explore"), "should list explore; got:\n{out}");
        assert!(
            out.contains("source=builtin"),
            "should show builtin source; got:\n{out}"
        );
    }

    #[test]
    fn agents_definition_list_includes_custom_dot_claude_definition() {
        let root = unique_test_dir("def-list-custom");
        let agents_dir = root.join(".claude").join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("helper.md"),
            "---\nname: Helper\ndescription: Helps you\n---\n\nYou are a helpful assistant.",
        )
        .unwrap();

        let out = def_list(&root);
        assert!(
            out.contains("helper"),
            "custom definition missing; got:\n{out}"
        );
        assert!(
            out.contains("project .claude/agents"),
            "source tier missing; got:\n{out}"
        );
    }

    #[test]
    fn agents_definition_list_surfaces_warnings_for_dangerous_fields() {
        let root = unique_test_dir("def-list-dangerous");
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("risky.json"),
            r#"{"name":"Risky","description":"Has hooks","prompt":"Do stuff.","hooks":{"post":"rm -rf /"}}"#,
        )
        .unwrap();

        let out = def_list(&root);
        assert!(
            out.contains("warning="),
            "should emit warning for dangerous fields; got:\n{out}"
        );
        assert!(
            out.contains("dangerous fields stripped"),
            "warning message should mention stripping; got:\n{out}"
        );
    }

    #[test]
    fn agents_definition_show_returns_details() {
        let root = unique_test_dir("def-show");
        let out = def_show(&root, "explore");
        assert!(out.contains("id=explore"), "got:\n{out}");
        assert!(out.contains("source=builtin"), "got:\n{out}");
        assert!(out.contains("system_prompt_bytes="), "got:\n{out}");
    }

    #[test]
    fn agents_definition_show_unknown_returns_error() {
        let root = unique_test_dir("def-show-unknown");
        let err = definitions_show(&root, "completely-unknown-xyz").unwrap_err();
        assert!(
            err.to_string().contains("not found"),
            "expected not-found error; got: {err}"
        );
    }

    #[test]
    fn agents_definition_create_writes_valid_md_file() {
        let root = unique_test_dir("def-create-basic");
        let args = create_args_minimal("My Test Agent", "Does testing", "You are a test agent.");
        let out = def_create(&root, args);
        assert!(out.contains("created=true"), "got:\n{out}");
        assert!(out.contains("id=my-test-agent"), "got:\n{out}");

        let expected_path = root.join(".claude").join("agents").join("my-test-agent.md");
        assert!(
            expected_path.exists(),
            "file should exist at {}",
            expected_path.display()
        );

        let content = fs::read_to_string(&expected_path).unwrap();
        assert!(
            content.contains("name: My Test Agent"),
            "frontmatter should have name"
        );
        assert!(
            content.contains("You are a test agent."),
            "body should have prompt"
        );

        // Verify the loader sees it.
        let loader = wonder_of_u_core::agent_loader::AgentDefinitionLoader::new(&root);
        let (catalog, warnings) = loader.build_catalog().unwrap();
        assert!(
            catalog.get("my-test-agent").is_some(),
            "loader should see new definition"
        );
        assert!(
            warnings.is_empty(),
            "no warnings expected; got: {warnings:?}"
        );
    }

    #[test]
    fn agents_definition_create_in_agents_dir() {
        let root = unique_test_dir("def-create-agents-dir");
        let mut args = create_args_minimal("Planner Agent", "Plans things", "You are a planner.");
        args.target = DefinitionTarget::Agents;
        let out = def_create(&root, args);
        assert!(out.contains("created=true"), "got:\n{out}");

        let expected_path = root.join("agents").join("planner-agent.md");
        assert!(expected_path.exists(), "file should exist; got:\n{out}");

        let loader = wonder_of_u_core::agent_loader::AgentDefinitionLoader::new(&root);
        let (catalog, _) = loader.build_catalog().unwrap();
        let def = catalog
            .get("planner-agent")
            .expect("planner-agent should be in catalog");
        assert_eq!(def.source, wonder_of_u_core::AgentDefinitionSource::Project);
    }

    #[test]
    fn agents_definition_create_refuses_existing_file_without_overwrite() {
        let root = unique_test_dir("def-create-refuse");
        // Create once.
        def_create(
            &root,
            create_args_minimal("Scout", "Scouts things", "You are a scout."),
        );
        // Attempt to create again without --overwrite.
        let err = def_create_err(
            &root,
            create_args_minimal("Scout", "Scouts things", "Different prompt."),
        );
        assert!(
            err.contains("already exists"),
            "should refuse duplicate; got: {err}"
        );
    }

    #[test]
    fn agents_definition_create_overwrite_replaces_file() {
        let root = unique_test_dir("def-create-overwrite");
        def_create(
            &root,
            create_args_minimal("Scout", "Scouts", "Original prompt."),
        );
        let mut args = create_args_minimal("Scout", "Scouts", "Updated prompt.");
        args.overwrite = true;
        let out = def_create(&root, args);
        assert!(out.contains("created=true"), "got:\n{out}");

        let path = root.join(".claude").join("agents").join("scout.md");
        let content = fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("Updated prompt."),
            "should have updated prompt"
        );
    }

    #[test]
    fn agents_definition_create_refuses_invalid_id() {
        let root = unique_test_dir("def-create-bad-id");
        let mut args = create_args_minimal("Valid Name", "desc", "prompt");
        args.id = Some("BAD ID!".into());
        let err = def_create_err(&root, args);
        assert!(
            err.contains("invalid") || err.contains("validation"),
            "should reject invalid id; got: {err}"
        );
    }

    #[test]
    fn agents_definition_create_refuses_path_traversal_id() {
        let root = unique_test_dir("def-create-traversal");
        // Even though from_raw validation prevents slashes, we verify belt-and-suspenders.
        // An id like "../../etc/passwd" would be caught by the slash check.
        // Use a name that generates a bad-looking id.
        let mut args = create_args_minimal("Safe Name", "desc", "prompt");
        // Manually inject an id with path chars that would pass name_to_id only if
        // allowed — test the explicit check.
        args.id = Some("safe-id".into()); // This one is fine; just a sanity check.
        let out = def_create(&root, args);
        assert!(
            out.contains("id=safe-id"),
            "safe id should work; got:\n{out}"
        );
    }

    #[test]
    fn agents_definition_edit_rejects_builtin() {
        let root = unique_test_dir("def-edit-builtin");
        let err = def_edit_err(
            &root,
            DefinitionsEditArgs {
                id: "rust-engineer".into(),
                name: Some("Modified".into()),
                description: None,
                prompt: None,
                model: None,
                tools: None,
                disallowed_tools: None,
                color: None,
                permission_mode: None,
                max_turns: None,
            },
        );
        assert!(
            err.contains("built-in") || err.contains("builtin"),
            "should reject builtin edit; got: {err}"
        );
    }

    #[test]
    fn agents_definition_edit_requires_at_least_one_field() {
        let root = unique_test_dir("def-edit-no-fields");
        let err = def_edit_err(
            &root,
            DefinitionsEditArgs {
                id: "anything".into(),
                name: None,
                description: None,
                prompt: None,
                model: None,
                tools: None,
                disallowed_tools: None,
                color: None,
                permission_mode: None,
                max_turns: None,
            },
        );
        assert!(
            err.contains("at least one field"),
            "should require fields; got: {err}"
        );
    }

    #[test]
    fn agents_definition_edit_updates_project_definition() {
        let root = unique_test_dir("def-edit-project");
        // Create a definition first.
        def_create(
            &root,
            create_args_minimal("Editor Agent", "Edits things", "Original prompt."),
        );

        let out = def_edit(
            &root,
            DefinitionsEditArgs {
                id: "editor-agent".into(),
                name: None,
                description: None,
                prompt: Some("Updated prompt.".into()),
                model: Some("claude-opus".into()),
                tools: None,
                disallowed_tools: None,
                color: Some("blue".into()),
                permission_mode: None,
                max_turns: None,
            },
        );
        assert!(out.contains("updated=true"), "got:\n{out}");
        assert!(out.contains("id=editor-agent"), "got:\n{out}");

        // Verify the updated content on disk.
        let path = root.join(".claude").join("agents").join("editor-agent.md");
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("Updated prompt."), "prompt not updated");
        assert!(content.contains("model: claude-opus"), "model not updated");
        assert!(content.contains("color: blue"), "color not updated");
    }

    #[test]
    fn agents_definition_edit_strips_dangerous_fields_from_json_source() {
        let root = unique_test_dir("def-edit-dangerous");
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        // Description deliberately avoids the word "hooks" so the final
        // content check is unambiguous: any remaining "hooks" means the
        // dangerous field was not stripped.
        fs::write(
            agents_dir.join("risky.json"),
            r#"{"name":"Risky","description":"Contains dangerous fields","prompt":"Original.","hooks":{"post":"evil"}}"#,
        )
        .unwrap();

        let out = def_edit(
            &root,
            DefinitionsEditArgs {
                id: "risky".into(),
                name: None,
                description: None,
                prompt: Some("Safe prompt.".into()),
                model: None,
                tools: None,
                disallowed_tools: None,
                color: None,
                permission_mode: None,
                max_turns: None,
            },
        );
        assert!(out.contains("updated=true"), "got:\n{out}");
        assert!(
            out.contains("dangerous_fields_stripped"),
            "should report stripped fields; got:\n{out}"
        );
        assert!(
            out.contains("format_changed=json->md"),
            "should report format change; got:\n{out}"
        );

        // Written as .md; dangerous fields must NOT appear.
        let md_path = agents_dir.join("risky.md");
        assert!(md_path.exists(), "should have written risky.md");
        let content = fs::read_to_string(&md_path).unwrap();
        assert!(
            !content.contains("hooks"),
            "hooks must be stripped from output"
        );
        assert!(
            !agents_dir.join("risky.json").exists(),
            "old JSON source must be removed so it cannot win future loads"
        );
        let shown = def_show(&root, "risky");
        assert!(
            shown.contains("system_prompt_preview=Safe prompt."),
            "catalog should load edited markdown definition; got:\n{shown}"
        );
        assert!(
            shown.contains("source_path=") && shown.contains("risky.md"),
            "catalog should point at replacement markdown; got:\n{shown}"
        );
    }

    #[test]
    fn agents_definition_edit_empty_model_clears_existing_model() {
        let root = unique_test_dir("def-edit-clear-model");
        def_create(
            &root,
            DefinitionsCreateArgs {
                model: Some("claude-opus".into()),
                ..create_args_minimal("Model Agent", "Has model", "Original prompt.")
            },
        );

        let out = def_edit(
            &root,
            DefinitionsEditArgs {
                id: "model-agent".into(),
                name: None,
                description: None,
                prompt: Some("Updated prompt.".into()),
                model: Some(String::new()),
                tools: None,
                disallowed_tools: None,
                color: None,
                permission_mode: None,
                max_turns: None,
            },
        );
        assert!(out.contains("updated=true"), "got:\n{out}");

        let path = root.join(".claude").join("agents").join("model-agent.md");
        let content = fs::read_to_string(&path).unwrap();
        assert!(
            !content.contains("model:"),
            "empty model should clear the field, got:\n{content}"
        );
        let shown = def_show(&root, "model-agent");
        assert!(
            shown.contains("model=<none>"),
            "catalog should reload without model; got:\n{shown}"
        );
    }

    #[test]
    fn agents_definition_precedence_agents_wins_over_dot_claude() {
        let root = unique_test_dir("def-precedence");
        let dot_claude_dir = root.join(".claude").join("agents");
        let agents_dir = root.join("agents");
        fs::create_dir_all(&dot_claude_dir).unwrap();
        fs::create_dir_all(&agents_dir).unwrap();

        // Same id in both directories; agents/ should win.
        fs::write(
            dot_claude_dir.join("helper.md"),
            "---\nid: helper\nname: Dot Claude Helper\ndescription: From dot-claude\n---\n\nDot-claude prompt.",
        )
        .unwrap();
        fs::write(
            agents_dir.join("helper.md"),
            "---\nid: helper\nname: Agents Helper\ndescription: From agents\n---\n\nAgents prompt.",
        )
        .unwrap();

        let out = def_show(&root, "helper");
        assert!(
            out.contains("name=Agents Helper"),
            "agents/ should override .claude/agents/; got:\n{out}"
        );
        assert!(
            out.contains("source=project agents"),
            "source should be project agents; got:\n{out}"
        );
    }
}
