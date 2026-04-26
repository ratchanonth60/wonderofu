use async_trait::async_trait;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
};

pub struct TuiCommand;

impl TuiCommand {
    pub const fn new() -> Self {
        Self
    }

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
