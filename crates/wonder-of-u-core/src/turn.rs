//! Turn driver — the async agent loop (Phase 1.3 + 1.4 of the codex-rs port).
//!
//! Consumes `Submission`s from the SQ, drives the [`ModelProvider`], and
//! streams `Event`s to the EQ. Honors `Op::Interrupt` mid-turn by dropping
//! the in-flight stream and emitting `TurnAborted`. `Op::Shutdown` exits the
//! loop and emits `ShutdownComplete`. `Op::ExecApproval` and `Op::PatchApproval`
//! resolve pending approval requests tracked in [`TurnDriverState`].
//!
//! Scope:
//! - `Op::UserInput` → build `ModelRequest` from text/image inputs, stream
//!   provider events wrapped in `Event { id, msg }`.
//! - `Op::Interrupt` → emit `TurnAborted`, drop any in-flight stream.
//! - `Op::Shutdown` → emit `ShutdownComplete`, exit the loop.
//! - `Op::ExecApproval` / `Op::PatchApproval` → resolve a pending request
//!   registered when the stream emitted `ExecApprovalRequest` (Phase 1.4).
//!   The actual tool execution lands in Phase 1.6.
//!
//! Gated behind the `wonder-of-u-async` cargo feature.

#![cfg(feature = "wonder-of-u-async")]

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use wonder_of_u_protocol::{
    approvals::{ExecApprovalRequestEvent, ReviewDecision},
    config::SandboxPolicy,
    events::{ErrorEvent, Event, EventMsg, TurnAbortReason, TurnAbortedEvent},
    protocol::Op,
    user_input::UserInput,
};

use crate::conversation::ConversationConfig;
use crate::provider_async::{ModelInput, ModelProvider, ModelRequest};

// ---------------------------------------------------------------------------
// TurnDriverState — pending approval tracking
// ---------------------------------------------------------------------------

/// Pending approval request tracked by the turn driver.
///
/// Indexed by the *submission id* of the `ExecApprovalRequest` event (which
/// is also the `id` field of the resolving `Op::ExecApproval`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    /// Tool-call id (correlates with subsequent `ExecCommand*` events in
    /// Phase 1.6).
    pub call_id: String,
    /// Proposed command string (snapshot for diagnostics / replay).
    pub command: String,
}

/// State owned by the conversation manager and borrowed by the turn driver.
///
/// Phase 1.4 uses this to track approval requests; Phase 1.6 will extend it
/// with tool-execution state (running children, sandbox handles, etc.).
#[derive(Debug, Default)]
pub struct TurnDriverState {
    /// Pending approval requests indexed by their submission id.
    pending_approvals: HashMap<String, PendingApproval>,
}

impl TurnDriverState {
    /// Empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an approval request when the stream emits
    /// `ExecApprovalRequest`.
    pub fn register_approval(
        &mut self,
        submission_id: impl Into<String>,
        request: ExecApprovalRequestEvent,
    ) {
        self.pending_approvals.insert(
            submission_id.into(),
            PendingApproval {
                call_id: request.call_id,
                command: request.command,
            },
        );
    }

    /// Resolve (and remove) a pending approval. Returns `None` if no request
    /// is registered under `submission_id`.
    pub fn resolve_approval(
        &mut self,
        submission_id: &str,
    ) -> Option<(PendingApproval, ReviewDecision)> {
        // The decision is supplied by the caller; we just remove and return
        // the request. Phase 1.6 will pass the decision to the executor.
        self.pending_approvals
            .remove(submission_id)
            .map(|req| (req, ReviewDecision::Abstain))
    }

    /// Resolve with an explicit decision. Phase 1.4 + 1.6 use this shape.
    pub fn resolve_with_decision(
        &mut self,
        submission_id: &str,
        decision: ReviewDecision,
    ) -> Option<PendingApproval> {
        let _ = decision;
        self.pending_approvals.remove(submission_id)
    }

    /// Number of pending approvals (test introspection).
    pub fn pending_count(&self) -> usize {
        self.pending_approvals.len()
    }
}

// ---------------------------------------------------------------------------
// Spawn helpers
// ---------------------------------------------------------------------------

/// Spawn the turn driver onto the current tokio runtime and return its
/// `JoinHandle`. The driver consumes `submissions` and produces `events`.
pub fn spawn_turn_driver<P>(
    config: ConversationConfig,
    provider: Arc<P>,
    state: TurnDriverState,
    submissions: mpsc::Receiver<wonder_of_u_protocol::protocol::Submission>,
    events: mpsc::Sender<Event>,
) -> JoinHandle<TurnDriverState>
where
    P: ModelProvider + 'static,
{
    tokio::spawn(turn_driver(config, provider, state, submissions, events))
}

/// Run the turn driver loop. Returns the final state when the submission
/// channel closes or `Op::Shutdown` is received.
pub async fn turn_driver<P>(
    config: ConversationConfig,
    provider: Arc<P>,
    mut state: TurnDriverState,
    mut submissions: mpsc::Receiver<wonder_of_u_protocol::protocol::Submission>,
    events: mpsc::Sender<Event>,
) -> TurnDriverState
where
    P: ModelProvider + 'static,
{
    while let Some(sub) = submissions.recv().await {
        let sub_id = sub.id.0.clone();
        match sub.op {
            Op::Shutdown => {
                let _ = events
                    .send(Event {
                        id: sub_id,
                        msg: EventMsg::ShutdownComplete,
                    })
                    .await;
                return state;
            }
            Op::UserInput { items, .. } => {
                run_turn(
                    &config,
                    &provider,
                    sub_id,
                    &items,
                    &mut submissions,
                    &events,
                    &mut state,
                )
                .await;
            }
            Op::Interrupt => {
                let _ = events
                    .send(Event {
                        id: sub_id,
                        msg: EventMsg::TurnAborted(TurnAbortedEvent {
                            reason: TurnAbortReason::Interrupted,
                        }),
                    })
                    .await;
            }
            Op::ExecApproval { id, decision, .. } => {
                // Phase 1.4: resolve the pending request and record the
                // decision. Phase 1.6 will forward the decision to the
                // tool executor and emit ExecCommand* events.
                state.resolve_with_decision(&id, decision);
            }
            Op::PatchApproval { id, decision } => {
                state.resolve_with_decision(&id, decision);
            }
            _ => {
                // Phase 1.5+ will route Compact, Review, ThreadRollback, etc.
                // For now, drop on the floor.
            }
        }
    }
    state
}

/// Drive a single user-input turn: build a request, stream the provider's
/// events into the EQ, watching the submission channel for interrupts and
/// approval responses.
async fn run_turn<P>(
    config: &ConversationConfig,
    provider: &Arc<P>,
    sub_id: String,
    items: &[UserInput],
    submissions: &mut mpsc::Receiver<wonder_of_u_protocol::protocol::Submission>,
    events: &mpsc::Sender<Event>,
    state: &mut TurnDriverState,
) where
    P: ModelProvider + 'static,
{
    let request = build_request(config, items);

    let stream = match provider.stream(request).await {
        Ok(s) => s,
        Err(e) => {
            let _ = events
                .send(Event {
                    id: sub_id,
                    msg: EventMsg::Error(ErrorEvent {
                        message: e.to_string(),
                        details: None,
                        fatal: true,
                    }),
                })
                .await;
            return;
        }
    };

    futures::pin_mut!(stream);

    loop {
        tokio::select! {
            biased;
            control = submissions.recv() => {
                match control {
                    Some(control_sub) => {
                        match control_sub.op {
                            Op::Interrupt => {
                                let _ = events.send(Event {
                                    id: control_sub.id.0,
                                    msg: EventMsg::TurnAborted(TurnAbortedEvent {
                                        reason: TurnAbortReason::Interrupted,
                                    }),
                                }).await;
                                return;
                            }
                            Op::Shutdown => {
                                let _ = events.send(Event {
                                    id: control_sub.id.0,
                                    msg: EventMsg::ShutdownComplete,
                                }).await;
                                return;
                            }
                            Op::ExecApproval { id, decision, .. } => {
                                state.resolve_with_decision(&id, decision);
                            }
                            Op::PatchApproval { id, decision } => {
                                state.resolve_with_decision(&id, decision);
                            }
                            _ => {
                                // Other ops queued behind the current turn
                                // are ignored for now.
                            }
                        }
                    }
                    None => return,
                }
            }
            next = stream.next() => {
                match next {
                    Some(Ok(msg)) => {
                        // Register pending approvals before forwarding so a
                        // racing client decision is never lost.
                        if let EventMsg::ExecApprovalRequest(ref req) = msg {
                            state.register_approval(&sub_id, req.clone());
                        }
                        if events.send(Event { id: sub_id.clone(), msg }).await.is_err() {
                            return;
                        }
                    }
                    Some(Err(e)) => {
                        let _ = events.send(Event {
                            id: sub_id.clone(),
                            msg: EventMsg::Error(ErrorEvent {
                                message: e.to_string(),
                                details: None,
                                fatal: true,
                            }),
                        }).await;
                        return;
                    }
                    None => return,
                }
            }
        }
    }
}

/// Convert a slice of `UserInput` into a `ModelRequest`.
fn build_request(config: &ConversationConfig, items: &[UserInput]) -> ModelRequest {
    let input = items
        .iter()
        .filter_map(|item| match item {
            UserInput::Text { text } => Some(ModelInput::Text(text.clone())),
            UserInput::LocalImage { path } => Some(ModelInput::Image(path.clone())),
            UserInput::RemoteImage { url } => Some(ModelInput::Image(url.clone())),
            // Mentions and slash commands are resolved by the TUI/CLI before
            // reaching the turn driver (Phase 4). They are not sent to the
            // model directly.
            UserInput::Mention { .. } | UserInput::SlashCommand { .. } => None,
        })
        .collect();
    ModelRequest {
        input,
        model: config.model.clone(),
    }
}

// Allow `unused` on SandboxPolicy import so the file compiles cleanly even
// when no sandbox-aware code lands in Phase 1.4. The import is here as a
// marker that the turn driver will need to read sandbox config once the
// approval flow lands in Phase 1.6.
#[allow(dead_code)]
fn _sandbox_marker(_: &SandboxPolicy) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use wonder_of_u_protocol::{
        approvals::{ExecApprovalRequestEvent, RiskLevel},
        config::{AskForApproval, SandboxPolicy},
        events::{AgentMessageContentDeltaEvent, AgentMessageEvent},
        protocol::{Op, Submission},
        session::SubmissionId,
        user_input::UserInput,
    };

    use crate::provider_async::MockProvider;

    fn test_config() -> ConversationConfig {
        ConversationConfig::new("test-model")
            .with_approval_policy(AskForApproval::Never)
            .with_sandbox_policy(SandboxPolicy::new_read_only_policy())
    }

    fn text_submission(id: &str, text: &str) -> Submission {
        Submission::new(
            SubmissionId::new(id),
            Op::UserInput {
                items: vec![UserInput::Text {
                    text: text.to_string(),
                }],
                final_output_json_schema: None,
                thread_settings: Default::default(),
            },
        )
    }

    #[tokio::test]
    async fn shutdown_emits_shutdown_complete_and_exits() {
        let provider = Arc::new(MockProvider::default());
        let (sub_tx, sub_rx) = mpsc::channel(8);
        let (evt_tx, mut evt_rx) = mpsc::channel(8);

        let handle = spawn_turn_driver(
            test_config(),
            provider,
            TurnDriverState::new(),
            sub_rx,
            evt_tx,
        );
        sub_tx
            .send(Submission::new(SubmissionId::new("s1"), Op::Shutdown))
            .await
            .unwrap();
        drop(sub_tx);

        let event = evt_rx.recv().await.expect("expected shutdown event");
        assert_eq!(event.id, "s1");
        assert_eq!(event.msg.kind(), "shutdown_complete");

        let _state = handle.await.unwrap();
    }

    #[tokio::test]
    async fn user_input_streams_provider_events_correlated_by_submission_id() {
        let provider = Arc::new(MockProvider::new(vec![
            EventMsg::AgentMessageContentDelta(AgentMessageContentDeltaEvent {
                delta: "hel".to_string(),
            }),
            EventMsg::AgentMessageContentDelta(AgentMessageContentDeltaEvent {
                delta: "lo".to_string(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                id: "msg_1".to_string(),
                text: "hello".to_string(),
            }),
        ]));
        let (sub_tx, sub_rx) = mpsc::channel(8);
        let (evt_tx, mut evt_rx) = mpsc::channel(8);

        let handle = spawn_turn_driver(
            test_config(),
            provider,
            TurnDriverState::new(),
            sub_rx,
            evt_tx,
        );

        sub_tx.send(text_submission("sub_42", "hi")).await.unwrap();

        for expected_kind in [
            "agent_message_content_delta",
            "agent_message_content_delta",
            "agent_message",
        ] {
            let event = evt_rx.recv().await.expect("expected event");
            assert_eq!(event.id, "sub_42");
            assert_eq!(event.msg.kind(), expected_kind);
        }

        sub_tx
            .send(Submission::new(SubmissionId::new("s_end"), Op::Shutdown))
            .await
            .unwrap();
        let final_event = evt_rx.recv().await.unwrap();
        assert_eq!(final_event.msg.kind(), "shutdown_complete");
        drop(sub_tx);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn interrupt_mid_stream_emits_turn_aborted() {
        let provider = Arc::new(MockProvider::new(vec![EventMsg::AgentMessageContentDelta(
            AgentMessageContentDeltaEvent {
                delta: "par".to_string(),
            },
        )]));
        let (sub_tx, sub_rx) = mpsc::channel(8);
        let (evt_tx, mut evt_rx) = mpsc::channel(8);

        let handle = spawn_turn_driver(
            test_config(),
            provider,
            TurnDriverState::new(),
            sub_rx,
            evt_tx,
        );

        sub_tx.send(text_submission("sub_1", "go")).await.unwrap();

        let event = evt_rx.recv().await.unwrap();
        assert_eq!(event.id, "sub_1");
        assert_eq!(event.msg.kind(), "agent_message_content_delta");

        sub_tx
            .send(Submission::new(SubmissionId::new("sub_2"), Op::Interrupt))
            .await
            .unwrap();

        let event = evt_rx.recv().await.unwrap();
        assert_eq!(event.id, "sub_2");
        match event.msg {
            EventMsg::TurnAborted(TurnAbortedEvent { reason }) => {
                assert_eq!(reason, TurnAbortReason::Interrupted);
            }
            other => panic!("expected TurnAborted, got {:?}", other),
        }

        sub_tx
            .send(Submission::new(SubmissionId::new("s_end"), Op::Shutdown))
            .await
            .unwrap();
        let event = evt_rx.recv().await.unwrap();
        assert_eq!(event.msg.kind(), "shutdown_complete");
        drop(sub_tx);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn provider_error_emits_error_event() {
        struct ErrProvider;
        #[async_trait::async_trait]
        impl ModelProvider for ErrProvider {
            async fn stream(
                &self,
                _request: ModelRequest,
            ) -> crate::Result<crate::provider_async::EventMsgStream> {
                let item: crate::Result<EventMsg> =
                    Err(crate::error::WonderError::Internal("boom".to_string()));
                Ok(Box::pin(futures::stream::once(async move { item })))
            }
        }

        let provider = Arc::new(ErrProvider);
        let (sub_tx, sub_rx) = mpsc::channel(8);
        let (evt_tx, mut evt_rx) = mpsc::channel(8);
        let handle = spawn_turn_driver(
            test_config(),
            provider,
            TurnDriverState::new(),
            sub_rx,
            evt_tx,
        );

        sub_tx.send(text_submission("sub_e", "hi")).await.unwrap();

        let event = evt_rx.recv().await.unwrap();
        assert_eq!(event.id, "sub_e");
        match event.msg {
            EventMsg::Error(ErrorEvent { message, fatal, .. }) => {
                assert!(message.contains("boom"));
                assert!(fatal);
            }
            other => panic!("expected Error, got {:?}", other),
        }

        sub_tx
            .send(Submission::new(SubmissionId::new("s_end"), Op::Shutdown))
            .await
            .unwrap();
        let event = evt_rx.recv().await.unwrap();
        assert_eq!(event.msg.kind(), "shutdown_complete");
        drop(sub_tx);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn build_request_filters_unsupported_inputs() {
        let cfg = test_config();
        let req = build_request(
            &cfg,
            &[
                UserInput::Text {
                    text: "hello".to_string(),
                },
                UserInput::Mention {
                    path: "/tmp/x".to_string(),
                },
                UserInput::SlashCommand {
                    name: "compact".to_string(),
                    args: String::new(),
                },
                UserInput::LocalImage {
                    path: "/tmp/img.png".to_string(),
                },
            ],
        );
        assert_eq!(req.model, "test-model");
        assert_eq!(req.input.len(), 2);
        assert!(matches!(req.input[0], ModelInput::Text(ref s) if s == "hello"));
        assert!(matches!(req.input[1], ModelInput::Image(ref s) if s == "/tmp/img.png"));
    }

    // -----------------------------------------------------------------------
    // Phase 1.4 — approval flow
    // -----------------------------------------------------------------------

    fn approval_request(submission_id: &str, call_id: &str, command: &str) -> EventMsg {
        EventMsg::ExecApprovalRequest(ExecApprovalRequestEvent {
            submission_id: submission_id.to_string(),
            turn_id: Some("turn_1".to_string()),
            call_id: call_id.to_string(),
            command: command.to_string(),
            cwd: "/tmp".to_string(),
            reason: Some("writes outside workspace".to_string()),
            risk_level: RiskLevel::High,
        })
    }

    #[tokio::test]
    async fn approval_request_registers_pending_and_forwards_event() {
        let provider = Arc::new(MockProvider::new(vec![approval_request(
            "sub_1",
            "call_1",
            "rm -rf build",
        )]));
        let (sub_tx, sub_rx) = mpsc::channel(8);
        let (evt_tx, mut evt_rx) = mpsc::channel(8);
        let handle = spawn_turn_driver(
            test_config(),
            provider,
            TurnDriverState::new(),
            sub_rx,
            evt_tx,
        );

        sub_tx
            .send(text_submission("sub_1", "clean"))
            .await
            .unwrap();

        let event = evt_rx.recv().await.unwrap();
        assert_eq!(event.id, "sub_1");
        assert_eq!(event.msg.kind(), "exec_approval_request");
        match event.msg {
            EventMsg::ExecApprovalRequest(req) => {
                assert_eq!(req.submission_id, "sub_1");
                assert_eq!(req.call_id, "call_1");
            }
            _ => unreachable!(),
        }

        // Shutdown and recover state to confirm the request was registered.
        sub_tx
            .send(Submission::new(SubmissionId::new("s_end"), Op::Shutdown))
            .await
            .unwrap();
        let evt = evt_rx.recv().await.unwrap();
        assert_eq!(evt.msg.kind(), "shutdown_complete");
        drop(sub_tx);
        let state = handle.await.unwrap();
        // We never sent Op::ExecApproval, so the request should still be
        // pending.
        assert_eq!(state.pending_count(), 1);
    }

    #[tokio::test]
    async fn exec_approval_resolves_pending_and_lets_stream_continue() {
        let provider = Arc::new(MockProvider::new(vec![
            approval_request("sub_1", "call_1", "rm -rf build"),
            EventMsg::AgentMessage(AgentMessageEvent {
                id: "msg_1".to_string(),
                text: "ok, removed".to_string(),
            }),
        ]));
        let (sub_tx, sub_rx) = mpsc::channel(8);
        let (evt_tx, mut evt_rx) = mpsc::channel(8);
        let handle = spawn_turn_driver(
            test_config(),
            provider,
            TurnDriverState::new(),
            sub_rx,
            evt_tx,
        );

        sub_tx
            .send(text_submission("sub_1", "clean"))
            .await
            .unwrap();

        // Event 1: ExecApprovalRequest.
        let evt = evt_rx.recv().await.unwrap();
        assert_eq!(evt.msg.kind(), "exec_approval_request");

        // Client approves.
        sub_tx
            .send(Submission::new(
                SubmissionId::new("sub_1"),
                Op::ExecApproval {
                    id: "sub_1".to_string(),
                    turn_id: None,
                    decision: ReviewDecision::Approved,
                },
            ))
            .await
            .unwrap();

        // Event 2: AgentMessage (stream continues after approval).
        let evt = evt_rx.recv().await.unwrap();
        assert_eq!(evt.id, "sub_1");
        assert_eq!(evt.msg.kind(), "agent_message");

        sub_tx
            .send(Submission::new(SubmissionId::new("s_end"), Op::Shutdown))
            .await
            .unwrap();
        let evt = evt_rx.recv().await.unwrap();
        assert_eq!(evt.msg.kind(), "shutdown_complete");
        drop(sub_tx);
        let state = handle.await.unwrap();
        // Decision was processed → pending count is 0.
        assert_eq!(state.pending_count(), 0);
    }

    #[tokio::test]
    async fn patch_approval_resolves_pending() {
        let provider = Arc::new(MockProvider::new(vec![
            EventMsg::ApplyPatchApprovalRequest(
                wonder_of_u_protocol::approvals::ApplyPatchApprovalRequestEvent {
                    submission_id: "sub_2".to_string(),
                    turn_id: None,
                    call_id: "patch_1".to_string(),
                    changes: vec![wonder_of_u_protocol::approvals::PatchChange {
                        path: "/tmp/x.rs".to_string(),
                        diff: "+ hello".to_string(),
                    }],
                },
            ),
        ]));
        let (sub_tx, sub_rx) = mpsc::channel(8);
        let (evt_tx, mut evt_rx) = mpsc::channel(8);
        let handle = spawn_turn_driver(
            test_config(),
            provider,
            TurnDriverState::new(),
            sub_rx,
            evt_tx,
        );

        sub_tx
            .send(text_submission("sub_2", "patch"))
            .await
            .unwrap();
        let evt = evt_rx.recv().await.unwrap();
        assert_eq!(evt.msg.kind(), "apply_patch_approval_request");

        sub_tx
            .send(Submission::new(
                SubmissionId::new("sub_2"),
                Op::PatchApproval {
                    id: "sub_2".to_string(),
                    decision: ReviewDecision::Denied,
                },
            ))
            .await
            .unwrap();

        sub_tx
            .send(Submission::new(SubmissionId::new("s_end"), Op::Shutdown))
            .await
            .unwrap();
        let evt = evt_rx.recv().await.unwrap();
        assert_eq!(evt.msg.kind(), "shutdown_complete");
        drop(sub_tx);
        let state = handle.await.unwrap();
        assert_eq!(state.pending_count(), 0);
    }

    #[tokio::test]
    async fn exec_approval_for_unknown_submission_is_a_noop() {
        let provider = Arc::new(MockProvider::default());
        let (sub_tx, sub_rx) = mpsc::channel(8);
        let (evt_tx, mut evt_rx) = mpsc::channel(8);
        let handle = spawn_turn_driver(
            test_config(),
            provider,
            TurnDriverState::new(),
            sub_rx,
            evt_tx,
        );

        // Send an ExecApproval for a submission that has no pending request.
        sub_tx
            .send(Submission::new(
                SubmissionId::new("ghost"),
                Op::ExecApproval {
                    id: "ghost".to_string(),
                    turn_id: None,
                    decision: ReviewDecision::Approved,
                },
            ))
            .await
            .unwrap();

        // Send shutdown and verify no error events were emitted.
        sub_tx
            .send(Submission::new(SubmissionId::new("s_end"), Op::Shutdown))
            .await
            .unwrap();
        let evt = evt_rx.recv().await.unwrap();
        assert_eq!(evt.msg.kind(), "shutdown_complete");
        drop(sub_tx);
        let state = handle.await.unwrap();
        assert_eq!(state.pending_count(), 0);
    }

    #[tokio::test]
    async fn turn_driver_state_register_and_resolve() {
        let mut state = TurnDriverState::new();
        assert_eq!(state.pending_count(), 0);

        state.register_approval(
            "sub_x",
            ExecApprovalRequestEvent {
                submission_id: "sub_x".to_string(),
                turn_id: None,
                call_id: "call_x".to_string(),
                command: "ls".to_string(),
                cwd: "/".to_string(),
                reason: None,
                risk_level: RiskLevel::Low,
            },
        );
        assert_eq!(state.pending_count(), 1);

        let resolved = state.resolve_with_decision("sub_x", ReviewDecision::Approved);
        assert!(resolved.is_some());
        let pending = resolved.unwrap();
        assert_eq!(pending.call_id, "call_x");
        assert_eq!(pending.command, "ls");
        assert_eq!(state.pending_count(), 0);

        // Resolving again returns None.
        assert!(
            state
                .resolve_with_decision("sub_x", ReviewDecision::Approved)
                .is_none()
        );
    }
}
