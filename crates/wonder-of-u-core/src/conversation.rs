//! Async agent-loop host (Phase 1.3 of the codex-rs port).
//!
//! `ConversationManager<P>` owns the lifetime of a single conversation
//! thread. Construct one with a [`ConversationConfig`] and an
//! [`Arc<P>`](std::sync::Arc) over a [`ModelProvider`]; call
//! [`spawn`](ConversationManager::spawn) to obtain the SQ/EQ channel pair and
//! start the turn-driver task. Drop the manager to abort the task.
//!
//! Phase 1.3: spawns the real turn driver (see [`crate::turn`]).
//!
//! Gated behind the `wonder-of-u-async` cargo feature (CLAUDE.md pitfall #1).

#![cfg(feature = "wonder-of-u-async")]

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use wonder_of_u_protocol::{
    config::{AskForApproval, SandboxPolicy},
    events::Event,
    protocol::Submission,
};

use crate::provider_async::ModelProvider;
use crate::turn::spawn_turn_driver;

/// Initial configuration for a conversation thread.
///
/// Persisted by `wonder-of-u-state` in Phase 5; for Phase 1.3 it is passed
/// directly to the `ConversationManager` constructor.
#[derive(Debug, Clone)]
pub struct ConversationConfig {
    /// Model identifier (e.g. `"claude-opus-4"`).
    pub model: String,
    /// Approval policy in effect for the thread.
    pub approval_policy: AskForApproval,
    /// Sandbox policy in effect for the thread.
    pub sandbox_policy: SandboxPolicy,
    /// Working directory the conversation runs in.
    pub cwd: String,
}

impl ConversationConfig {
    /// Construct a config with sensible defaults (model only; cwd empty).
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            approval_policy: AskForApproval::default(),
            sandbox_policy: SandboxPolicy::new_workspace_write_policy(),
            cwd: String::new(),
        }
    }

    /// Set the working directory.
    #[must_use]
    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = cwd.into();
        self
    }

    /// Set the approval policy.
    #[must_use]
    pub fn with_approval_policy(mut self, policy: AskForApproval) -> Self {
        self.approval_policy = policy;
        self
    }

    /// Set the sandbox policy.
    #[must_use]
    pub fn with_sandbox_policy(mut self, policy: SandboxPolicy) -> Self {
        self.sandbox_policy = policy;
        self
    }
}

/// Submission Queue sender. Clients push [`Submission`]s into the agent.
pub type SubmissionSender = mpsc::Sender<Submission>;

/// Event Queue receiver. Clients pull [`Event`]s out of the agent.
pub type EventReceiver = mpsc::Receiver<Event>;

/// Channel handles returned by [`ConversationManager::spawn`].
#[derive(Debug)]
pub struct ConversationHandle {
    /// Sender half of the SQ.
    pub submissions: SubmissionSender,
    /// Receiver half of the EQ.
    pub events: EventReceiver,
}

/// Conversation thread host.
///
/// Owns the lifetime of the turn-driver task. Dropping the manager aborts
/// the task (the client should normally send `Op::Shutdown` first for a
/// graceful exit).
pub struct ConversationManager<P: ModelProvider + 'static> {
    config: ConversationConfig,
    provider: Arc<P>,
    task: Option<JoinHandle<()>>,
}

impl<P: ModelProvider + 'static> ConversationManager<P> {
    /// Construct a new manager from a [`ConversationConfig`] and provider.
    pub fn new(config: ConversationConfig, provider: Arc<P>) -> Self {
        Self {
            config,
            provider,
            task: None,
        }
    }

    /// Borrow the active config.
    pub fn config(&self) -> &ConversationConfig {
        &self.config
    }

    /// Channel buffer size used by [`spawn`] when none is provided.
    pub const DEFAULT_CHANNEL_BUFFER: usize = 64;

    /// Spawn the turn driver and return the channel pair.
    ///
    /// `&mut self` because spawning moves the `JoinHandle` into the manager.
    /// A manager can host at most one conversation.
    pub fn spawn(&mut self) -> ConversationHandle {
        self.spawn_with_buffer(Self::DEFAULT_CHANNEL_BUFFER)
    }

    /// Same as [`spawn`] but with an explicit channel buffer size.
    pub fn spawn_with_buffer(&mut self, buffer: usize) -> ConversationHandle {
        let (submissions, sub_rx) = mpsc::channel(buffer);
        let (event_tx, events) = mpsc::channel(buffer);
        let config = self.config.clone();
        let provider = Arc::clone(&self.provider);
        let handle = spawn_turn_driver(config, provider, sub_rx, event_tx);
        self.task = Some(handle);
        ConversationHandle {
            submissions,
            events,
        }
    }
}

impl<P: ModelProvider + 'static> Drop for ConversationManager<P> {
    fn drop(&mut self) {
        if let Some(handle) = self.task.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wonder_of_u_protocol::{
        events::EventMsg,
        protocol::{Op, Submission},
        session::SubmissionId,
    };

    use crate::provider_async::MockProvider;

    fn empty_provider() -> Arc<MockProvider> {
        Arc::new(MockProvider::default())
    }

    #[test]
    fn config_helpers_set_fields() {
        let cfg = ConversationConfig::new("test-model")
            .with_cwd("/tmp")
            .with_approval_policy(AskForApproval::Never)
            .with_sandbox_policy(SandboxPolicy::ReadOnly {
                network_access: true,
            });
        assert_eq!(cfg.model, "test-model");
        assert_eq!(cfg.cwd, "/tmp");
        assert_eq!(cfg.approval_policy, AskForApproval::Never);
        assert!(matches!(
            cfg.sandbox_policy,
            SandboxPolicy::ReadOnly {
                network_access: true
            }
        ));
    }

    #[test]
    fn config_is_cloneable() {
        let cfg = ConversationConfig::new("m");
        let cfg2 = cfg.clone();
        assert_eq!(cfg.model, cfg2.model);
    }

    #[test]
    fn manager_exposes_config() {
        let mgr = ConversationManager::new(ConversationConfig::new("m"), empty_provider());
        assert_eq!(mgr.config().model, "m");
    }

    #[tokio::test]
    async fn spawn_returns_connected_channels() {
        let mut mgr = ConversationManager::new(ConversationConfig::new("m"), empty_provider());
        let ConversationHandle {
            submissions,
            mut events,
        } = mgr.spawn();

        // Channels are connected but the empty mock provider emits nothing.
        // Send a shutdown, expect a single event.
        submissions
            .send(Submission::new(SubmissionId::new("s1"), Op::Shutdown))
            .await
            .unwrap();
        drop(submissions);

        let event = events.recv().await.expect("expected shutdown event");
        assert_eq!(event.id, "s1");
        assert_eq!(event.msg.kind(), "shutdown_complete");
    }

    #[tokio::test]
    async fn default_buffer_is_documented() {
        assert_eq!(
            ConversationManager::<MockProvider>::DEFAULT_CHANNEL_BUFFER,
            64
        );
    }

    #[test]
    fn event_msg_shutdown_complete_is_unit_variant() {
        let evt = EventMsg::ShutdownComplete;
        assert_eq!(evt.kind(), "shutdown_complete");
    }
}
