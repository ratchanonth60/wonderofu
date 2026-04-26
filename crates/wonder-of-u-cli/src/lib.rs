use std::{ffi::OsString, io::Write, path::PathBuf};

use clap::{Parser, Subcommand};
use wonder_of_u_core::{AppState, FeatureSet, MessageEnvelope, Result};
use wonder_of_u_storage::{SessionMetadata, TranscriptStore};

#[derive(Debug, Parser)]
#[command(
    name = "wonder-of-u",
    version,
    about = "Rust workspace foundation for the wonder-of-u agent CLI"
)]
pub struct Cli {
    /// Override the wonder-of-u data directory used by storage-backed commands.
    #[arg(long, global = true)]
    pub storage_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Print enabled first-release feature gates.
    Features,
    /// Run lightweight startup diagnostics.
    Doctor,
    /// Session persistence helpers.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Create a new session metadata record and seed transcript.
    New {
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
}

pub fn run_from<I, T, W>(args: I, writer: &mut W) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    W: Write,
{
    let cli = Cli::parse_from(args);
    run(cli, writer)
}

pub fn run<W: Write>(cli: Cli, writer: &mut W) -> Result<()> {
    match cli.command.unwrap_or(Commands::Doctor) {
        Commands::Features => print_features(writer),
        Commands::Doctor => {
            writeln!(writer, "wonder-of-u foundation ok")?;
            writeln!(writer, "workspace crates: core, storage, cli, test-support")?;
            Ok(())
        }
        Commands::Session { command } => run_session_command(cli.storage_dir, command, writer),
    }
}

fn print_features<W: Write>(writer: &mut W) -> Result<()> {
    for flag in FeatureSet::first_release().iter() {
        writeln!(writer, "{flag:?}")?;
    }
    Ok(())
}

fn run_session_command<W: Write>(
    storage_dir: Option<PathBuf>,
    command: SessionCommand,
    writer: &mut W,
) -> Result<()> {
    match command {
        SessionCommand::New { title, cwd } => {
            let cwd = cwd.unwrap_or(std::env::current_dir()?);
            let mut state = AppState::new(cwd);
            if let Some(title) = title {
                state.session.title = title;
            }
            let message = MessageEnvelope::system(state.session.id, "Session created");
            state.push_message(message.clone())?;

            if let Some(storage_dir) = storage_dir {
                let store = TranscriptStore::new(storage_dir);
                store.append_message(&message)?;
                store.write_metadata(&SessionMetadata::from_app_state(&state))?;
            }

            writeln!(writer, "session_id={}", state.session.id)?;
            writeln!(writer, "title={}", state.session.title)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_is_default_command() {
        let mut output = Vec::new();
        run_from(["wonder-of-u"], &mut output).expect("run doctor");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("foundation ok"));
    }

    #[test]
    fn features_command_lists_session_persistence() {
        let mut output = Vec::new();
        run_from(["wonder-of-u", "features"], &mut output).expect("run features");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("SessionPersistence"));
    }
}
