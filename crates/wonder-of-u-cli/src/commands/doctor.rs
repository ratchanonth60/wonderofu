use std::path::PathBuf;

use async_trait::async_trait;
use wonder_of_u_agent::{ProviderRegistry, ProviderResolver};
use wonder_of_u_core::{
    AuthMaterialKind, Command, CommandContext, CommandInvocation, CommandKind, CommandOutput,
    CommandSpec, Result,
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

        let mut lines = vec![
            "wonder-of-u foundation ok".to_string(),
            format!("cwd={}", context.cwd.display()),
            format!("features={}", context.features.iter().count()),
            format!(
                "commands={} total / {} public",
                self.total_commands, self.public_commands
            ),
            format!("provider_selection={selection}"),
            format!("provider_readiness={}", report.readiness.label()),
            format!("auth_kind={}", report.auth.kind_label()),
            format!("auth_status={}", report.auth.status_label()),
        ];

        // Enumerate every builtin provider with auth/env hints so the user can
        // tell at a glance what env vars to set for providers they want to use.
        // Never print secret values—only variable names and readiness state.
        let registry = ProviderRegistry::builtin();
        let ready_ids: std::collections::BTreeSet<&str> = report
            .available_providers
            .iter()
            .map(|p| p.id.as_str())
            .collect();

        lines.push(format!(
            "providers_registered={}",
            registry.providers().count()
        ));
        lines.push(format!("providers_ready={}", ready_ids.len()));

        for p in registry.providers() {
            let ready = if ready_ids.contains(p.id.as_str()) {
                "ready"
            } else {
                "missing"
            };
            let auth = auth_kind_label(p.auth_kind);
            // Build a compact hint showing what the user needs to configure.
            let hint = provider_env_hint(p);
            lines.push(format!(
                "provider[{}]={};auth={};state={};{}",
                p.id, p.display_name, auth, ready, hint
            ));
        }

        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

/// Returns a short `key=value` hint string describing the env vars or auth
/// material needed to activate this provider.  Never includes secret values.
fn auth_kind_label(kind: AuthMaterialKind) -> &'static str {
    match kind {
        AuthMaterialKind::None => "none",
        AuthMaterialKind::ApiKey => "api_key",
        AuthMaterialKind::OAuth => "oauth",
        AuthMaterialKind::AwsSigV4 => "aws_sigv4",
        AuthMaterialKind::AwsBearer => "aws_bearer",
        AuthMaterialKind::AwsProfile => "aws_profile",
        AuthMaterialKind::GcpOAuth2 => "gcp_oauth2",
    }
}

fn provider_env_hint(p: &wonder_of_u_agent::ProviderDescriptor) -> String {
    match p.auth_kind {
        AuthMaterialKind::None => "hint=no_auth_required".into(),
        AuthMaterialKind::ApiKey => {
            let mut parts = Vec::new();
            if let Some(env) = &p.api_key_env {
                parts.push(format!("api_key_env={env}"));
            }
            if let Some(env) = &p.endpoint_env {
                parts.push(format!("endpoint_env={env}"));
            }
            if parts.is_empty() {
                "hint=set_api_key_via_login".into()
            } else {
                parts.join(";")
            }
        }
        AuthMaterialKind::OAuth => "hint=run_login_provider_copilot".into(),
        AuthMaterialKind::AwsSigV4 => {
            "hint=set_AWS_ACCESS_KEY_ID+AWS_SECRET_ACCESS_KEY_or_AWS_PROFILE".into()
        }
        AuthMaterialKind::AwsBearer => "hint=set_AWS_BEARER_TOKEN_BEDROCK".into(),
        AuthMaterialKind::AwsProfile => {
            "hint=set_AWS_PROFILE_and_configure_credentials_file".into()
        }
        AuthMaterialKind::GcpOAuth2 => {
            "hint=set_VERTEXAI_PROJECT+VERTEXAI_LOCATION+GOOGLE_APPLICATION_CREDENTIALS".into()
        }
    }
}
