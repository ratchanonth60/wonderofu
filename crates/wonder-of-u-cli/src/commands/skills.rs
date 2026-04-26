use std::{collections::BTreeSet, path::PathBuf};

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand};
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, Result, WonderError,
};
use wonder_of_u_skills::{SkillCatalog, SkillRegistration};

use super::{
    parse_command_args,
    plugin::load_catalogs,
    prompt::{
        PromptExecutionInput, append_execution_metadata_lines, execute_prompt_run, truncate_chars,
    },
};

pub struct SkillsCommand {
    storage_dir: Option<PathBuf>,
}

impl SkillsCommand {
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "skills",
            "Inspect and run bundled, local, and plugin-provided skills",
            CommandKind::Local,
        );
        spec.required_features.insert(FeatureFlag::Skills);
        spec
    }
}

#[derive(Debug, Parser)]
struct SkillsArgs {
    #[command(subcommand)]
    command: Option<SkillsSubcommand>,
}

#[derive(Debug, Subcommand)]
enum SkillsSubcommand {
    List,
    Show { skill: String },
    Run(SkillRunArgs),
}

#[derive(Debug, Args)]
struct SkillRunArgs {
    #[arg()]
    skill: String,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    system: Option<String>,
    #[arg(long)]
    session_id: Option<String>,
    #[arg(long)]
    max_output_tokens: Option<u32>,
    #[arg(long)]
    temperature: Option<f32>,
    #[arg(long, default_value_t = false)]
    tools: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
    request: Vec<String>,
}

#[async_trait]
impl Command for SkillsCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let args = parse_command_args::<SkillsArgs>("skills", &invocation)?;
        let (_, _, catalog) = load_catalogs(&context.cwd, self.storage_dir.as_deref())?;
        let output = match args.command.unwrap_or(SkillsSubcommand::List) {
            SkillsSubcommand::List => render_skill_list(&catalog),
            SkillsSubcommand::Show { skill } => render_skill_show(&catalog, &skill)?,
            SkillsSubcommand::Run(args) => self.run_skill(&context, &catalog, args)?,
        };
        Ok(CommandOutput::Text(output))
    }
}

impl SkillsCommand {
    fn run_skill(
        &self,
        context: &CommandContext,
        catalog: &SkillCatalog,
        args: SkillRunArgs,
    ) -> Result<String> {
        let skill = catalog
            .find(&args.skill)
            .ok_or_else(|| WonderError::not_found("skill", args.skill.clone()))?;
        let user_request = args.request.join(" ");
        if user_request.trim().is_empty() {
            return Err(WonderError::validation("skill request cannot be empty"));
        }
        let combined_prompt = compose_skill_prompt(skill, &user_request, args.tools);
        let result = execute_prompt_run(
            context,
            self.storage_dir.as_deref(),
            PromptExecutionInput {
                provider: args.provider,
                model: args.model,
                system_prompt: args
                    .system
                    .and_then(|system| (!system.trim().is_empty()).then_some(system)),
                session_id: args.session_id,
                max_output_tokens: args.max_output_tokens,
                temperature: args.temperature,
                tool_use: args.tools,
                allowed_tools: args
                    .tools
                    .then(|| normalized_allowed_tools(&skill.manifest.allowed_tools)),
                session_title: skill_session_title(skill, &user_request),
                prompt: combined_prompt.clone(),
                entrypoint: "skills.run",
            },
        )?;
        Ok(render_skill_run_output(
            skill,
            &user_request,
            &combined_prompt,
            &result,
        ))
    }
}

fn render_skill_list(catalog: &SkillCatalog) -> String {
    let mut lines = vec![
        format!("skills={}", catalog.len()),
        format!("bundled={}", catalog.bundled_count()),
        format!("plugin_skills={}", catalog.plugin_count()),
        format!("slash_commands={}", catalog.command_count()),
        format!("errors={}", catalog.errors().len()),
    ];
    if catalog.skills().is_empty() {
        lines.push("note=no skills discovered".into());
    }
    for (index, skill) in catalog.skills().iter().enumerate() {
        lines.push(format!("skill[{index}].name={}", skill.manifest.name));
        lines.push(format!("skill[{index}].source={}", skill.source.label()));
        lines.push(format!("skill[{index}].trust={}", skill.trust.label()));
        lines.push(format!("skill[{index}].tool={}", skill.tool_spec.name));
        lines.push(format!(
            "skill[{index}].allowed_tools={}",
            allowed_tools_label(skill)
        ));
        if let Some(command_spec) = &skill.command_spec {
            lines.push(format!("skill[{index}].command={}", command_spec.name));
        }
    }
    for (index, error) in catalog.errors().iter().enumerate() {
        lines.push(format!("error[{index}].source={}", error.source.label()));
        lines.push(format!("error[{index}].path={}", error.path.display()));
        lines.push(format!("error[{index}].message={}", error.message));
    }
    lines.push(
        "note=skills can be run with `skills run`; add `--tools` to enable allowed built-in tool execution"
            .into(),
    );
    lines.join("\n")
}

fn render_skill_show(catalog: &SkillCatalog, name: &str) -> Result<String> {
    let skill = catalog
        .find(name)
        .ok_or_else(|| WonderError::not_found("skill", name))?;
    Ok(render_skill_details(skill))
}

fn render_skill_details(skill: &SkillRegistration) -> String {
    let prompt_excerpt = skill
        .prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("-")
        .chars()
        .take(120)
        .collect::<String>();

    vec![
        format!("skill={}", skill.manifest.name),
        format!("description={}", skill.manifest.description),
        format!("source={}", skill.source.label()),
        format!("trust={}", skill.trust.label()),
        format!("prompt_bytes={}", skill.prompt.len()),
        format!("allowed_tools={}", allowed_tools_label(skill)),
        format!(
            "slash_command={}",
            skill
                .command_spec
                .as_ref()
                .map(|command| command.name.as_str())
                .unwrap_or("-")
        ),
        format!("prompt_excerpt={prompt_excerpt}"),
        "note=skills run as prompt compositions in this slice; `skills run --tools` enables only the manifest allowed_tools set".into(),
    ]
    .join("\n")
}

fn compose_skill_prompt(
    skill: &SkillRegistration,
    user_request: &str,
    tools_enabled: bool,
) -> String {
    format!(
        concat!(
            "Skill: {name}\n",
            "Description: {description}\n",
            "Source: {source}\n",
            "Allowed tools: {allowed_tools}\n",
            "Tool execution mode: {tool_mode}\n",
            "Slash command metadata: {slash_command}\n\n",
            "Skill instructions:\n",
            "{instructions}\n\n",
            "User request:\n",
            "{request}\n"
        ),
        name = skill.manifest.name,
        description = skill.manifest.description,
        source = skill.source.label(),
        allowed_tools = allowed_tools_label(skill),
        tool_mode = if tools_enabled {
            "manifest-restricted automatic tool execution enabled"
        } else {
            "metadata only; automatic tool execution disabled"
        },
        slash_command = skill
            .command_spec
            .as_ref()
            .map(|command| command.name.as_str())
            .unwrap_or("-"),
        instructions = skill.prompt,
        request = user_request,
    )
}

fn render_skill_run_output(
    skill: &SkillRegistration,
    user_request: &str,
    combined_prompt: &str,
    result: &super::prompt::PromptExecutionResult,
) -> String {
    let mut lines = vec![result.response.output_text.clone(), String::new()];
    lines.push(format!("skill={}", skill.manifest.name));
    lines.push(format!("skill_source={}", skill.source.label()));
    lines.push(format!("skill_trust={}", skill.trust.label()));
    lines.push(format!("allowed_tools={}", allowed_tools_label(skill)));
    lines.push(format!(
        "slash_command={}",
        skill
            .command_spec
            .as_ref()
            .map(|command| command.name.as_str())
            .unwrap_or("-")
    ));
    lines.push(format!(
        "user_request={}",
        sanitize_single_line(user_request)
    ));
    lines.push(format!("composed_prompt_bytes={}", combined_prompt.len()));
    append_execution_metadata_lines(
        &mut lines,
        &result.state,
        &result.response,
        result.persisted,
    );
    if result.tool_use_requested {
        lines.push(format!("tool_use_requested={}", result.tool_use_requested));
        lines.push(format!("tool_calls={}", result.tool_calls));
        lines.push(
            "note=skill execution used the prompt tool loop and only exposed manifest allowed_tools"
                .into(),
        );
    } else {
        lines.push(
            "note=skill execution is prompt-based in this slice; add `--tools` to enable manifest-restricted tool execution"
                .into(),
        );
    }
    lines.join("\n")
}

fn allowed_tools_label(skill: &SkillRegistration) -> String {
    if skill.manifest.allowed_tools.is_empty() {
        "-".into()
    } else {
        skill.manifest.allowed_tools.join(",")
    }
}

fn sanitize_single_line(value: &str) -> String {
    value.lines().collect::<Vec<_>>().join("\\n")
}

fn skill_session_title(skill: &SkillRegistration, user_request: &str) -> String {
    let summary = truncate_chars(
        &user_request
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        40,
    );
    if summary.is_empty() {
        format!("Skill: {}", skill.manifest.name)
    } else {
        format!("Skill: {} — {}", skill.manifest.name, summary)
    }
}

fn normalized_allowed_tools(tools: &[String]) -> BTreeSet<String> {
    tools
        .iter()
        .map(|tool| tool.trim().to_ascii_lowercase())
        .filter(|tool| !tool.is_empty())
        .collect()
}
