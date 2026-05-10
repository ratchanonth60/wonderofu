use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use wonder_of_u_core::{MessagePayload, Result, SessionId, WonderError};
use wonder_of_u_storage::TranscriptStore;

use super::parse_session_id;

pub(crate) fn copy_last_response(
    storage_dir: Option<&Path>,
    session_id: Option<&str>,
) -> Result<String> {
    let response = load_last_response(storage_dir, session_id)?;
    write_to_clipboard(&response)?;
    Ok(response)
}

fn load_last_response(storage_dir: Option<&Path>, session_id: Option<&str>) -> Result<String> {
    let store = require_store(storage_dir)?;
    let session_id = resolve_session_id(&store, session_id)?;
    let transcript = store.load_session(session_id)?;

    transcript
        .messages
        .iter()
        .rev()
        .find_map(|message| match &message.payload {
            MessagePayload::AssistantText { content } if !content.trim().is_empty() => {
                Some(content.clone())
            }
            _ => None,
        })
        .ok_or_else(|| WonderError::not_found("assistant response", session_id.to_string()))
}

fn require_store(storage_dir: Option<&Path>) -> Result<TranscriptStore> {
    storage_dir.map(TranscriptStore::new).ok_or_else(|| {
        WonderError::validation(
            "copy command requires --storage-dir or HOME/XDG_CONFIG_HOME so persisted session data can be loaded",
        )
    })
}

fn resolve_session_id(store: &TranscriptStore, session_id: Option<&str>) -> Result<SessionId> {
    match session_id {
        Some(session_id) => parse_session_id(session_id),
        None => store
            .list_metadata()?
            .into_iter()
            .next()
            .map(|metadata| metadata.session_id)
            .ok_or_else(|| WonderError::not_found("persisted session", "latest")),
    }
}

#[cfg(target_os = "macos")]
fn write_to_clipboard(text: &str) -> Result<()> {
    run_clipboard_command("pbcopy", &[], text)
}

#[cfg(target_os = "linux")]
fn write_to_clipboard(text: &str) -> Result<()> {
    for (command, args) in [
        ("xclip", &["-selection", "clipboard"][..]),
        ("xsel", &["--clipboard", "--input"][..]),
    ] {
        if run_clipboard_command(command, args, text).is_ok() {
            return Ok(());
        }
    }

    Err(WonderError::validation(
        "failed to copy to clipboard; install xclip or xsel",
    ))
}

#[cfg(target_os = "windows")]
fn write_to_clipboard(text: &str) -> Result<()> {
    run_clipboard_command("clip", &[], text)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn write_to_clipboard(_text: &str) -> Result<()> {
    Err(WonderError::validation(
        "clipboard copy is unsupported on this platform",
    ))
}

fn run_clipboard_command(command: &str, args: &[&str], text: &str) -> Result<()> {
    if run_clipboard_command_impl(command, args, text) {
        Ok(())
    } else {
        Err(WonderError::validation(format!(
            "failed to copy to clipboard with `{command}`"
        )))
    }
}

fn run_clipboard_command_impl(command: &str, args: &[&str], text: &str) -> bool {
    let mut child = match Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };

    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(text.as_bytes()).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
    }

    child.wait().is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use wonder_of_u_core::{AppState, MessageEnvelope, MessagePayload};
    use wonder_of_u_storage::SessionMetadata;
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    fn persist_session(dir: &Path, messages: Vec<MessagePayload>) -> SessionId {
        let store = TranscriptStore::new(dir);
        let mut state = AppState::new(std::path::PathBuf::from("/workspace"));

        for payload in messages {
            let message = MessageEnvelope::new(state.session.id, payload);
            store.append_message(&message).expect("append transcript");
            state.push_message(message).expect("push message");
        }

        store
            .write_metadata(&SessionMetadata::from_state_with_transcript(
                &state,
                state.messages.len(),
            ))
            .expect("write metadata");

        state.session.id
    }

    #[test]
    fn copy_last_response_requires_storage_dir() {
        let error = copy_last_response(None, None).expect_err("copy should fail without storage");
        assert!(
            error
                .to_string()
                .contains("copy command requires --storage-dir")
        );
    }

    #[test]
    fn copy_last_response_errors_when_no_sessions_exist() {
        let dir = unique_test_dir("copy-command-no-sessions");
        let error = copy_last_response(Some(dir.as_path()), None)
            .expect_err("copy should fail without sessions");
        assert!(error.to_string().contains("persisted session"));
    }

    #[test]
    fn load_last_response_returns_latest_assistant_text() {
        let dir = unique_test_dir("copy-command-last-response");
        let session_id = persist_session(
            &dir,
            vec![
                MessagePayload::UserText {
                    content: "hello".into(),
                },
                MessagePayload::AssistantText {
                    content: "first answer".into(),
                },
                MessagePayload::AssistantToolUse {
                    tool: "view".into(),
                    use_id: wonder_of_u_core::ToolUseId::new(),
                    input: serde_json::json!({ "path": "/workspace/demo" }),
                },
                MessagePayload::AssistantText {
                    content: "latest answer".into(),
                },
            ],
        );

        let response = load_last_response(Some(dir.as_path()), Some(&session_id.to_string()))
            .expect("load last response");
        assert_eq!(response, "latest answer");
    }
}
