use std::{collections::BTreeMap, path::PathBuf};

use async_trait::async_trait;
use clap::Parser;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec,
    FeatureFlag, Result, SessionId, WonderError,
};
use wonder_of_u_storage::{SessionMetadata, SessionSnapshot, TranscriptStore};

use super::{parse_command_args, parse_session_id};

/// Represents tag command.
pub struct TagCommand {
    storage_dir: Option<PathBuf>,
}

impl TagCommand {
    /// Creates a new tag command.
    pub const fn new(storage_dir: Option<PathBuf>) -> Self {
        Self { storage_dir }
    }

    /// Returns the command spec.
    pub fn command_spec() -> CommandSpec {
        let mut spec = CommandSpec::new(
            "tag",
            "Add, remove, or list searchable session tags",
            CommandKind::Local,
        );
        spec.required_features =
            std::collections::BTreeSet::from([FeatureFlag::SessionPersistence]);
        spec
    }

    fn current_session_tags(&self, context: &CommandContext) -> Result<Vec<String>> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(context.session_tags.clone());
        };
        let store = TranscriptStore::new(storage_dir);
        if let Some(snapshot) = store.read_snapshot_if_exists(context.session_id)? {
            return Ok(snapshot.state.session.tags);
        }
        Ok(store.read_metadata(context.session_id)?.tags)
    }

    fn persist_session_tags(
        &self,
        store: &TranscriptStore,
        session_id: SessionId,
        tags: Vec<String>,
    ) -> Result<()> {
        let restored = store.restore_session(session_id)?;
        let mut state = restored.state;
        state.set_session_tags(tags);
        store.write_metadata(&SessionMetadata::from_state_with_transcript(
            &state,
            restored.transcript.messages.len(),
        ))?;
        store.write_snapshot(&SessionSnapshot::from_app_state(
            &state,
            restored.transcript.messages.len(),
            restored.transcript.warnings.len(),
        ))
    }

    fn resolve_store_and_session(
        &self,
        context: &CommandContext,
        requested_session_id: Option<&str>,
    ) -> Result<(TranscriptStore, SessionId)> {
        let storage_dir = self.storage_dir.as_ref().ok_or_else(|| {
            WonderError::validation(
                "this command requires --storage-dir so persisted session data can be loaded",
            )
        })?;
        let store = TranscriptStore::new(storage_dir);
        let session_id = match requested_session_id {
            Some(session_id) => parse_session_id(session_id)?,
            None if context.interactive => context.session_id,
            None => store
                .list_metadata()?
                .into_iter()
                .next()
                .map(|metadata| metadata.session_id)
                .ok_or_else(|| WonderError::not_found("persisted session", "latest"))?,
        };
        Ok((store, session_id))
    }

    fn add_tag(
        &self,
        context: &CommandContext,
        requested_session_id: Option<&str>,
        tag: &str,
    ) -> Result<CommandOutput> {
        if self.storage_dir.is_none() && requested_session_id.is_none() && context.interactive {
            let mut tags = context.session_tags.clone();
            let added = push_tag_if_missing(&mut tags, tag);
            return Ok(render_tag_update(
                context.session_id,
                tags,
                if added {
                    format!("tagged session with #{tag}")
                } else {
                    format!("session already has #{tag}")
                },
            ));
        }

        let (store, session_id) = self.resolve_store_and_session(context, requested_session_id)?;
        let mut metadata = store.read_metadata(session_id)?;
        let added = push_tag_if_missing(&mut metadata.tags, tag);
        self.persist_session_tags(&store, session_id, metadata.tags.clone())?;
        Ok(render_tag_update(
            session_id,
            metadata.tags,
            if added {
                format!("tagged session with #{tag}")
            } else {
                format!("session already has #{tag}")
            },
        ))
    }

    fn remove_tag(
        &self,
        context: &CommandContext,
        requested_session_id: Option<&str>,
        tag: &str,
    ) -> Result<CommandOutput> {
        if self.storage_dir.is_none() && requested_session_id.is_none() && context.interactive {
            let mut tags = context.session_tags.clone();
            let removed = remove_tag_if_present(&mut tags, tag);
            return Ok(render_tag_update(
                context.session_id,
                tags,
                if removed {
                    format!("removed tag #{tag}")
                } else {
                    format!("tag #{tag} was not present")
                },
            ));
        }

        let (store, session_id) = self.resolve_store_and_session(context, requested_session_id)?;
        let mut metadata = store.read_metadata(session_id)?;
        let removed = remove_tag_if_present(&mut metadata.tags, tag);
        self.persist_session_tags(&store, session_id, metadata.tags.clone())?;
        Ok(render_tag_update(
            session_id,
            metadata.tags,
            if removed {
                format!("removed tag #{tag}")
            } else {
                format!("tag #{tag} was not present")
            },
        ))
    }

    fn list_tags(&self) -> Result<CommandOutput> {
        let storage_dir = self.storage_dir.as_ref().ok_or_else(|| {
            WonderError::validation(
                "this command requires --storage-dir so persisted session data can be loaded",
            )
        })?;
        let store = TranscriptStore::new(storage_dir);
        let mut grouped = BTreeMap::<String, Vec<SessionMetadata>>::new();
        for metadata in store
            .list_metadata()?
            .into_iter()
            .filter(|metadata| !metadata.tags.is_empty())
        {
            for tag in metadata
                .tags
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
            {
                grouped.entry(tag).or_default().push(metadata.clone());
            }
        }

        let mut lines = vec![format!("tags={}", grouped.len())];
        for (index, (tag, sessions)) in grouped.into_iter().enumerate() {
            lines.push(format!("tag[{index}].name={tag}"));
            lines.push(format!("tag[{index}].sessions={}", sessions.len()));
            for (session_index, session) in sessions.into_iter().enumerate() {
                lines.push(format!(
                    "tag[{index}].session[{session_index}].id={}",
                    session.session_id
                ));
                lines.push(format!(
                    "tag[{index}].session[{session_index}].title={}",
                    session.title
                ));
                lines.push(format!(
                    "tag[{index}].session[{session_index}].cwd={}",
                    session.cwd.display()
                ));
            }
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    fn legacy_toggle_current_tag(
        &self,
        context: &CommandContext,
        tag: &str,
    ) -> Result<CommandOutput> {
        let current_tags = self.current_session_tags(context)?;
        let current_tag = current_tags.first().cloned();

        if current_tag.as_deref() == Some(tag) && context.interactive {
            return Ok(CommandOutput::Text(format!(
                "tag_remove_confirmation={tag}\nstatus=confirm tag removal"
            )));
        }

        let next_tags = vec![tag.to_string()];
        if let Some(storage_dir) = self.storage_dir.as_deref() {
            let store = TranscriptStore::new(storage_dir);
            self.persist_session_tags(&store, context.session_id, next_tags.clone())?;
        }

        Ok(CommandOutput::Text(format!(
            "session_tags={}\nstatus=tagged session with #{tag}",
            next_tags.join(","),
        )))
    }
}

#[derive(Debug, Parser)]
struct TagArgs {
    #[arg()]
    name: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    remove: bool,
    #[arg(long)]
    list: bool,
}

#[async_trait]
impl Command for TagCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let raw = invocation.args.trim();
        if context.interactive && (raw.is_empty() || is_help_arg(raw)) {
            return Ok(CommandOutput::Text(tag_help_text()));
        }

        let args = parse_command_args::<TagArgs>("tag", &invocation)?;
        validate_tag_args(&args)?;

        if args.list {
            return self.list_tags();
        }

        let tag = normalize_tag_name(args.name.as_deref().unwrap_or_default())?;
        if args.remove {
            return self.remove_tag(&context, args.session.as_deref(), &tag);
        }
        if context.interactive && args.session.is_none() {
            return self.legacy_toggle_current_tag(&context, &tag);
        }
        self.add_tag(&context, args.session.as_deref(), &tag)
    }
}

fn render_tag_update(session_id: SessionId, tags: Vec<String>, status: String) -> CommandOutput {
    CommandOutput::Text(format!(
        "session_id={session_id}\nsession_tags={}\nstatus={status}",
        tags.join(",")
    ))
}

fn push_tag_if_missing(tags: &mut Vec<String>, tag: &str) -> bool {
    if tags.iter().any(|existing| existing == tag) {
        return false;
    }
    tags.push(tag.to_string());
    true
}

fn remove_tag_if_present(tags: &mut Vec<String>, tag: &str) -> bool {
    let original_len = tags.len();
    tags.retain(|existing| existing != tag);
    original_len != tags.len()
}

fn validate_tag_args(args: &TagArgs) -> Result<()> {
    if args.list {
        if args.name.is_some() || args.session.is_some() || args.remove {
            return Err(WonderError::validation(
                "--list cannot be combined with a tag name, --session, or --remove",
            ));
        }
        return Ok(());
    }
    if args.name.is_none() {
        return Err(WonderError::validation(
            "tag name is required unless --list is used",
        ));
    }
    Ok(())
}

fn is_help_arg(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help" | "info")
}

fn normalize_tag_name(value: &str) -> Result<String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(WonderError::validation("tag name cannot be empty"));
    }
    Ok(normalized.into())
}

fn tag_help_text() -> String {
    concat!(
        "Usage: /tag <tag-name>\n\n",
        "Toggle a searchable tag on the current session.\n",
        "Run the same command again to remove the tag.\n",
        "Tags are displayed after the branch name in /resume and can be searched later.\n\n",
        "Examples:\n",
        "  /tag bugfix\n",
        "  /tag feature-auth\n",
        "  /tag wip"
    )
    .into()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use futures::executor::block_on;
    use wonder_of_u_core::{AppState, FeatureSet, MessageEnvelope, PermissionMode, SessionId};
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn command_context(cwd: &Path, session_id: SessionId) -> CommandContext {
        CommandContext {
            session_id,
            cwd: cwd.to_path_buf(),
            features: FeatureSet::first_release(),
            authenticated: false,
            interactive: true,
            permission_mode: PermissionMode::Default,
            theme: None,
            session_color: None,
            effort_level: None,
            brief_mode: false,
            fast_mode: false,
            session_tags: Vec::new(),
            additional_working_directories: Vec::new(),
        }
    }

    fn noninteractive_context(cwd: &Path) -> CommandContext {
        CommandContext {
            interactive: false,
            session_id: SessionId::new(),
            session_tags: Vec::new(),
            ..command_context(cwd, SessionId::new())
        }
    }

    fn persist_session(dir: &Path, title: &str, tags: Vec<String>) -> (TranscriptStore, AppState) {
        let mut state = AppState::new(dir.to_path_buf());
        state.session.title = title.into();
        state.set_session_tags(tags);
        let store = TranscriptStore::new(dir);
        let message = MessageEnvelope::system(state.session.id, "Session created");
        state.push_message(message.clone()).expect("push message");
        store.append_message(&message).expect("append transcript");
        store
            .write_metadata(&SessionMetadata::from_state_with_transcript(&state, 1))
            .expect("write metadata");
        store
            .write_snapshot(&SessionSnapshot::from_app_state(&state, 1, 0))
            .expect("write snapshot");
        (store, state)
    }

    #[test]
    fn tag_command_adds_tag_and_is_idempotent() {
        let dir = unique_test_dir("tag-command-add");
        let (store, state) = persist_session(&dir, "Tagged Session", Vec::new());
        let command = TagCommand::new(Some(dir.clone()));

        let first = block_on(command.execute(
            noninteractive_context(&dir),
            CommandInvocation {
                name: "tag".into(),
                args: "bugfix".into(),
                raw: "tag bugfix".into(),
            },
        ))
        .expect("first add succeeds");
        let second = block_on(command.execute(
            noninteractive_context(&dir),
            CommandInvocation {
                name: "tag".into(),
                args: "bugfix".into(),
                raw: "tag bugfix".into(),
            },
        ))
        .expect("second add succeeds");

        assert_eq!(
            first,
            CommandOutput::Text(format!(
                "session_id={}\nsession_tags=bugfix\nstatus=tagged session with #bugfix",
                state.session.id
            ))
        );
        assert_eq!(
            second,
            CommandOutput::Text(format!(
                "session_id={}\nsession_tags=bugfix\nstatus=session already has #bugfix",
                state.session.id
            ))
        );
        assert_eq!(
            store
                .read_metadata(state.session.id)
                .expect("read metadata")
                .tags,
            vec!["bugfix"]
        );
        assert_eq!(
            store
                .read_snapshot(state.session.id)
                .expect("read snapshot")
                .state
                .session
                .tags,
            vec!["bugfix"]
        );
    }

    #[test]
    fn tag_command_remove_tag_is_noop_when_missing() {
        let dir = unique_test_dir("tag-command-remove");
        let (store, state) = persist_session(&dir, "Tagged Session", vec!["bugfix".into()]);
        let command = TagCommand::new(Some(dir.clone()));

        let removed = block_on(command.execute(
            noninteractive_context(&dir),
            CommandInvocation {
                name: "tag".into(),
                args: "--remove bugfix".into(),
                raw: "tag --remove bugfix".into(),
            },
        ))
        .expect("remove succeeds");
        let missing = block_on(command.execute(
            noninteractive_context(&dir),
            CommandInvocation {
                name: "tag".into(),
                args: "--remove missing".into(),
                raw: "tag --remove missing".into(),
            },
        ))
        .expect("missing remove succeeds");

        assert_eq!(
            removed,
            CommandOutput::Text(format!(
                "session_id={}\nsession_tags=\nstatus=removed tag #bugfix",
                state.session.id
            ))
        );
        assert_eq!(
            missing,
            CommandOutput::Text(format!(
                "session_id={}\nsession_tags=\nstatus=tag #missing was not present",
                state.session.id
            ))
        );
        assert!(
            store
                .read_metadata(state.session.id)
                .expect("read metadata")
                .tags
                .is_empty()
        );
    }

    #[test]
    fn tag_command_lists_sessions_grouped_by_tag() {
        let dir = unique_test_dir("tag-command-list");
        let (_, first) = persist_session(&dir, "Bugfix Session", vec!["bugfix".into()]);
        let (_, second) =
            persist_session(&dir, "Release Session", vec!["bugfix".into(), "wip".into()]);
        let command = TagCommand::new(Some(dir.clone()));

        let output = block_on(command.execute(
            noninteractive_context(&dir),
            CommandInvocation {
                name: "tag".into(),
                args: "--list".into(),
                raw: "tag --list".into(),
            },
        ))
        .expect("list succeeds");

        let text = match output {
            CommandOutput::Text(text) => text,
            other => panic!("unexpected output: {other:?}"),
        };
        assert!(text.contains("tags=2"));
        assert!(text.contains("tag[0].name=bugfix"));
        assert!(text.contains(&format!("tag[0].session[0].id={}", second.session.id)));
        assert!(text.contains(&format!("tag[0].session[1].id={}", first.session.id)));
        assert!(text.contains("tag[1].name=wip"));
        assert!(text.contains(&format!("tag[1].session[0].id={}", second.session.id)));
    }

    #[test]
    fn tag_command_prompts_before_removing_current_tag_interactively() {
        let dir = unique_test_dir("tag-command-confirm");
        let (store, state) = persist_session(&dir, "Tagged Session", vec!["bugfix".into()]);

        let output = block_on(TagCommand::new(Some(dir.clone())).execute(
            command_context(&dir, state.session.id),
            CommandInvocation {
                name: "tag".into(),
                args: "bugfix".into(),
                raw: "/tag bugfix".into(),
            },
        ))
        .expect("tag prompt succeeds");

        assert_eq!(
            output,
            CommandOutput::Text(
                "tag_remove_confirmation=bugfix\nstatus=confirm tag removal".into()
            )
        );
        assert_eq!(
            store
                .read_metadata(state.session.id)
                .expect("read metadata")
                .tags,
            vec!["bugfix"]
        );
    }
}
