use async_trait::async_trait;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
};

/// Represents tui command
pub struct TuiCommand;

impl TuiCommand {
    /// Constant fn
    pub const fn new() -> Self {
        Self
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "tui",
            "Launch the live interactive terminal shell",
            CommandKind::Tui,
        )
    }
}

#[async_trait]
impl Command for TuiCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        _context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::OpenUi(
            "interactive shell launch is handled by the top-level `wonder-of-u tui` entrypoint"
                .into(),
        ))
    }
}
