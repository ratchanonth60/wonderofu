use std::path::PathBuf;

use async_trait::async_trait;
use wonder_of_u_agent::ProviderResolver;
use wonder_of_u_core::{
    Command, CommandContext, CommandInvocation, CommandKind, CommandOutput, CommandSpec, Result,
};

/// Represents doctor command
pub struct DoctorCommand {
    total_commands: usize,
    public_commands: usize,
    storage_dir: Option<PathBuf>,
}

impl DoctorCommand {
    /// Creates a new value
    pub fn new(
        total_commands: usize,
        public_commands: usize,
        storage_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            total_commands,
            public_commands,
            storage_dir,
        }
    }

    /// Handles command spec
    pub fn command_spec() -> CommandSpec {
        CommandSpec::new(
            "doctor",
            "Run lightweight startup diagnostics",
            CommandKind::NonInteractive,
        )
    }
}

#[async_trait]
impl Command for DoctorCommand {
    fn spec(&self) -> CommandSpec {
        Self::command_spec()
    }

    async fn execute(
        &self,
        context: CommandContext,
        _invocation: CommandInvocation,
    ) -> Result<CommandOutput> {
        let report = ProviderResolver::builtin().load_report(self.storage_dir.as_deref())?;
        let selection = report
            .selection_label()
            .unwrap_or_else(|| "unconfigured".into());
        Ok(CommandOutput::Text(format!(
            concat!(
                "wonder-of-u foundation ok\n",
                "cwd={}\n",
                "features={}\n",
                "commands={} total / {} public\n",
                "provider_selection={}\n",
                "provider_readiness={}\n",
                "auth_kind={}\n",
                "auth_status={}"
            ),
            context.cwd.display(),
            context.features.iter().count(),
            self.total_commands,
            self.public_commands,
            selection,
            report.readiness.label(),
            report.auth.kind_label(),
            report.auth.status_label(),
        )))
    }
}
