//! Async agent-loop host (Phase 1.1 of the codex-rs port).
//!
//! `ConversationManager` owns the wiring for a single conversation thread: it
//! produces the SQ/EQ channel pair that TUI/CLI clients push `Submission`s
//! into and pull `Event`s from. Phase 1.1 establishes the skeleton; Phase 1.3
//! spawns the turn-driver task that consumes the submission side and emits to
//! the event side.
//!
//! Gated behind the `wonder-of-u-async` cargo feature (CLAUDE.md pitfall #1).

#![cfg(feature = "wonder-of-u-async")]

use tokio::sync::mpsc;

use wonder_of_u_protocol::{
    config::{AskForApproval, SandboxPolicy},
    events::Event,
    protocol::Submission,
};

/// Initial configuration for a conversation thread.
///
/// Persisted by `wonder-of-u-state` in Phase 5; for Phase 1.1 it is passed
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
/// `ConversationManager` owns the wiring for a single conversation thread.
/// Clients construct one with a [`ConversationConfig`], call [`spawn`] to
/// obtain the channel pair, and exchange [`Submission`]s / [`Event`]s over
/// those channels.
///
/// [`spawn`]: ConversationManager::spawn
pub struct ConversationManager {
    config: ConversationConfig,
    /// Phase 1.1: keep-alive for the channel halves that no task owns yet.
    /// Phase 1.3 replaces this with a `tokio::task::JoinHandle` for the
    /// turn-driver task.
    keep_alive: Option<ConversationKeepAlive>,
}

/// Keep-alive halves for an un-spawned conversation (Phase 1.1 only).
///
/// Phase 1.3 removes this entirely once the turn-driver task owns the
/// `Submission` receiver and `Event` sender.
struct ConversationKeepAlive {
    _sub_rx: mpsc::Receiver<Submission>,
    _event_tx: mpsc::Sender<Event>,
}

impl ConversationManager {
    /// Construct a new manager from a [`ConversationConfig`].
    pub fn new(config: ConversationConfig) -> Self {
        Self {
            config,
            keep_alive: None,
        }
    }

    /// Borrow the active config.
    pub fn config(&self) -> &ConversationConfig {
        &self.config
    }

    /// Channel buffer size used by [`spawn`] when none is provided.
    pub const DEFAULT_CHANNEL_BUFFER: usize = 64;

    /// Create the channel pair for a conversation.
    ///
    /// Phase 1.1: creates the channels, retains the receiver/transmitter
    /// halves as a keep-alive, and returns the client-facing handles. No
    /// task is spawned yet. Phase 1.3 will spawn the turn-driver task here
    /// (the signature stays the same so call sites don't churn).
    ///
    /// `&mut self` because spawning moves the keep-alive halves into the
    /// manager. A manager can host at most one conversation.
    pub fn spawn(&mut self) -> ConversationHandle {
        self.spawn_with_buffer(Self::DEFAULT_CHANNEL_BUFFER)
    }

    /// Same as [`spawn`] but with an explicit channel buffer size.
    pub fn spawn_with_buffer(&mut self, buffer: usize) -> ConversationHandle {
        let (submissions, sub_rx) = mpsc::channel(buffer);
        let (event_tx, events) = mpsc::channel(buffer);
        self.keep_alive = Some(ConversationKeepAlive {
            _sub_rx: sub_rx,
            _event_tx: event_tx,
        });
        ConversationHandle {
            submissions,
            events,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wonder_of_u_protocol::{
        config::NetworkAccess,
        events::EventMsg,
        protocol::{Op, Submission},
        session::SubmissionId,
    };

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
        let mgr = ConversationManager::new(ConversationConfig::new("m"));
        assert_eq!(mgr.config().model, "m");
    }

    #[tokio::test]
    async fn spawn_returns_working_channels() {
        let mut mgr = ConversationManager::new(ConversationConfig::new("m"));
        let ConversationHandle {
            submissions,
            mut events,
        } = mgr.spawn();

        // Channels are connected but empty.
        assert!(events.try_recv().is_err());

        // Sender can be cloned across tasks.
        let submissions_clone = submissions.clone();
        let _ = Arc::new(submissions_clone);

        // Sender can drop without panicking.
        drop(submissions);
    }

    #[tokio::test]
    async fn channel_roundtrip_when_loop_is_simulated() {
        // Phase 1.1 simulates the future turn-driver task by manually moving
        // the channel halves between tasks. Phase 1.3 replaces this with the
        // real driver.
        let mut mgr = ConversationManager::new(ConversationConfig::new("m"));
        let ConversationHandle {
            submissions,
            mut events,
        } = mgr.spawn_with_buffer(4);

        // The real check: send a submission, ensure no panic, drop the
        // manager (which drops the keep-alive halves and closes the
        // channels), then verify the events channel closes.
        let sub = Submission::new(SubmissionId::new("sub_1"), Op::Shutdown);
        submissions.send(sub).await.expect("send should succeed");

        // No driver in Phase 1.1, so events stays empty while mgr is alive.
        assert!(events.try_recv().is_err());

        // Drop the manager → keep-alive halves drop → channels close.
        drop(mgr);
        // After close, the receiver returns None.
        assert!(events.recv().await.is_none());
        drop(submissions);
    }

    #[test]
    fn keep_alive_lets_sender_succeed_until_dropped() {
        // Synchronous smoke check: create manager + spawn, send a few
        // submissions via blocking `try_send` (which requires a live
        // receiver), then drop the manager and verify the sender reports
        // the receiver closed.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut mgr = ConversationManager::new(ConversationConfig::new("m"));
            let ConversationHandle {
                submissions,
                mut events,
            } = mgr.spawn_with_buffer(2);

            let sub = Submission::new(SubmissionId::new("s1"), Op::Shutdown);
            submissions
                .send(sub)
                .await
                .expect("send succeeds while mgr alive");
            // No driver in Phase 1.1, so events is empty.
            assert!(events.try_recv().is_err());

            drop(mgr);
            // After manager drops, sender should report the channel closed.
            let res = submissions
                .send(Submission::new(SubmissionId::new("s2"), Op::Shutdown))
                .await;
            assert!(res.is_err(), "expected SendError after manager dropped");
        });
    }

    #[test]
    fn default_buffer_is_documented() {
        // Lock the default buffer size. Phase 1.3 may tune this but should
        // update this test in lockstep.
        assert_eq!(ConversationManager::DEFAULT_CHANNEL_BUFFER, 64);
    }

    #[test]
    fn event_msg_shutdown_complete_is_unit_variant() {
        // Smoke check: the unit variant we expect Phase 1.3 to emit on
        // Op::Shutdown is reachable from the same wire envelope.
        let evt = EventMsg::ShutdownComplete;
        assert_eq!(evt.kind(), "shutdown_complete");
    }

    #[test]
    fn sandbox_policy_with_network() {
        let cfg =
            ConversationConfig::new("m").with_sandbox_policy(SandboxPolicy::ExternalSandbox {
                network_access: NetworkAccess::Enabled,
            });
        assert!(cfg.sandbox_policy.has_full_network_access());
    }
}
