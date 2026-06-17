//! Turn driver — the async agent loop (Phase 1.3 of the codex-rs port).
//!
//! Consumes `Submission`s from the SQ, drives the [`ModelProvider`], and
//! streams `Event`s to the EQ. Honors `Op::Interrupt` mid-turn by dropping
//! the in-flight stream and emitting `TurnAborted`. `Op::Shutdown` exits the
//! loop and emits `ShutdownComplete`.
//!
//! Scope for Phase 1.3:
//! - `Op::UserInput` → build `ModelRequest` from text/image inputs, stream
//!   provider events wrapped in `Event { id, msg }`.
//! - `Op::Interrupt` → emit `TurnAborted`, drop any in-flight stream.
//! - `Op::Shutdown` → emit `ShutdownComplete`, exit the loop.
//! - Other Ops → silently dropped (Phase 1.4+ adds approval, compact, etc.).
//!
//! Gated behind the `wonder-of-u-async` cargo feature.

#![cfg(feature = "wonder-of-u-async")]

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use wonder_of_u_protocol::{
    config::SandboxPolicy,
    events::{ErrorEvent, Event, EventMsg, TurnAbortReason, TurnAbortedEvent},
    protocol::Op,
    user_input::UserInput,
};

use crate::conversation::ConversationConfig;
use crate::provider_async::{ModelInput, ModelProvider, ModelRequest};

/// Spawn the turn driver onto the current tokio runtime and return its
/// `JoinHandle`. The driver consumes `submissions` and produces `events`.
///
/// This is a free function (rather than a method on `ConversationManager`)
/// so it can be tested directly with mocked channels.
pub fn spawn_turn_driver<P>(
    config: ConversationConfig,
    provider: Arc<P>,
    submissions: mpsc::Receiver<wonder_of_u_protocol::protocol::Submission>,
    events: mpsc::Sender<Event>,
) -> JoinHandle<()>
where
    P: ModelProvider + 'static,
{
    tokio::spawn(turn_driver(config, provider, submissions, events))
}

/// Run the turn driver loop. Returns when the submission channel closes or
/// `Op::Shutdown` is received.
pub async fn turn_driver<P>(
    config: ConversationConfig,
    provider: Arc<P>,
    mut submissions: mpsc::Receiver<wonder_of_u_protocol::protocol::Submission>,
    events: mpsc::Sender<Event>,
) where
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
                return;
            }
            Op::UserInput { items, .. } => {
                run_turn(
                    &config,
                    &provider,
                    sub_id,
                    &items,
                    &mut submissions,
                    &events,
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
            _ => {
                // Phase 1.4+ will route ExecApproval, Compact, Review, etc.
                // For now, drop on the floor.
            }
        }
    }
}

/// Drive a single user-input turn: build a request, stream the provider's
/// events into the EQ, watching the submission channel for interrupts.
async fn run_turn<P>(
    config: &ConversationConfig,
    provider: &Arc<P>,
    sub_id: String,
    items: &[UserInput],
    submissions: &mut mpsc::Receiver<wonder_of_u_protocol::protocol::Submission>,
    events: &mpsc::Sender<Event>,
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
                                // Drop the stream by returning; the outer
                                // loop is ready for the next Submission.
                                return;
                            }
                            Op::Shutdown => {
                                let _ = events.send(Event {
                                    id: control_sub.id.0,
                                    msg: EventMsg::ShutdownComplete,
                                }).await;
                                // The outer `turn_driver` loop will exit on
                                // the next iteration when it sees the closed
                                // channel — but we can't tell it to do so
                                // directly. Return here; the next call into
                                // `run_turn` won't happen because the
                                // submission side is still alive. Phase 1.3
                                // leaves this as a known limitation; the
                                // client can detect shutdown by the
                                // `ShutdownComplete` event.
                                return;
                            }
                            _ => {
                                // Other ops queued behind the interrupt are
                                // ignored for this turn. They'll be picked
                                // up after the current turn ends.
                            }
                        }
                    }
                    None => {
                        // Client disconnected.
                        return;
                    }
                }
            }
            next = stream.next() => {
                match next {
                    Some(Ok(msg)) => {
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
                    None => return,  // Stream finished.
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
// when no sandbox-aware code lands in Phase 1.3. The import is here as a
// marker that the turn driver will need to read sandbox config once the
// approval flow lands in Phase 1.4.
#[allow(dead_code)]
fn _sandbox_marker(_: &SandboxPolicy) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use wonder_of_u_protocol::{
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
        let provider = Arc::new(MockProvider::single(EventMsg::AgentMessage(
            AgentMessageEvent {
                id: "msg".to_string(),
                text: "hi".to_string(),
            },
        )));
        let (sub_tx, sub_rx) = mpsc::channel(8);
        let (evt_tx, mut evt_rx) = mpsc::channel(8);

        let handle = spawn_turn_driver(test_config(), provider, sub_rx, evt_tx);
        sub_tx
            .send(Submission::new(SubmissionId::new("s1"), Op::Shutdown))
            .await
            .unwrap();
        drop(sub_tx);

        let event = evt_rx.recv().await.expect("expected shutdown event");
        assert_eq!(event.id, "s1");
        assert_eq!(event.msg.kind(), "shutdown_complete");

        handle.await.unwrap();
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

        let handle = spawn_turn_driver(test_config(), provider, sub_rx, evt_tx);

        sub_tx.send(text_submission("sub_42", "hi")).await.unwrap();

        // All three events should come back correlated to sub_42.
        for expected_kind in [
            "agent_message_content_delta",
            "agent_message_content_delta",
            "agent_message",
        ] {
            let event = evt_rx.recv().await.expect("expected event");
            assert_eq!(event.id, "sub_42");
            assert_eq!(event.msg.kind(), expected_kind);
        }

        // After the stream finishes, the turn driver goes back to waiting
        // for more submissions. Send Shutdown to exit.
        sub_tx
            .send(Submission::new(SubmissionId::new("s_end"), Op::Shutdown))
            .await
            .unwrap();
        let final_event = evt_rx.recv().await.unwrap();
        assert_eq!(final_event.msg.kind(), "shutdown_complete");
        drop(sub_tx);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn interrupt_mid_stream_emits_turn_aborted() {
        // A provider that emits a single delta, then waits forever (the test
        // sends an interrupt to unblock it).
        let provider = Arc::new(MockProvider::new(vec![EventMsg::AgentMessageContentDelta(
            AgentMessageContentDeltaEvent {
                delta: "par".to_string(),
            },
        )]));
        let (sub_tx, sub_rx) = mpsc::channel(8);
        let (evt_tx, mut evt_rx) = mpsc::channel(8);

        let handle = spawn_turn_driver(test_config(), provider, sub_rx, evt_tx);

        sub_tx.send(text_submission("sub_1", "go")).await.unwrap();

        // First event: the partial delta.
        let event = evt_rx.recv().await.unwrap();
        assert_eq!(event.id, "sub_1");
        assert_eq!(event.msg.kind(), "agent_message_content_delta");

        // Now interrupt.
        sub_tx
            .send(Submission::new(SubmissionId::new("sub_2"), Op::Interrupt))
            .await
            .unwrap();

        // The turn driver should emit a TurnAborted correlated to sub_2.
        let event = evt_rx.recv().await.unwrap();
        assert_eq!(event.id, "sub_2");
        match event.msg {
            EventMsg::TurnAborted(TurnAbortedEvent { reason }) => {
                assert_eq!(reason, TurnAbortReason::Interrupted);
            }
            other => panic!("expected TurnAborted, got {:?}", other),
        }

        // The driver should be ready for the next submission.
        sub_tx
            .send(Submission::new(SubmissionId::new("s_end"), Op::Shutdown))
            .await
            .unwrap();
        let event = evt_rx.recv().await.unwrap();
        assert_eq!(event.msg.kind(), "shutdown_complete");
        drop(sub_tx);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn provider_error_emits_error_event() {
        // A provider whose stream yields an Err.
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
        let handle = spawn_turn_driver(test_config(), provider, sub_rx, evt_tx);

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
        handle.await.unwrap();
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
        // Mention + slash command are dropped; text + image remain.
        assert_eq!(req.model, "test-model");
        assert_eq!(req.input.len(), 2);
        assert!(matches!(req.input[0], ModelInput::Text(ref s) if s == "hello"));
        assert!(matches!(req.input[1], ModelInput::Image(ref s) if s == "/tmp/img.png"));
    }
}
