use super::TuiController;
use super::*;
use tokio::sync::mpsc as tokio_mpsc;

impl TuiController<'_> {
    pub(in crate::tui_runtime) async fn resolve_pending_tool_approval(
        &mut self,
        approved: bool,
    ) -> Result<()> {
        let Some(pending) = self.state.pending_tool_approval.take() else {
            self.dismiss_dialog();
            return Ok(());
        };

        self.dialog = None;
        self.state.input_mode = InputMode::Prompt;
        self.needs_render = true;

        // If there's an active tool loop in paused state, execute just the pending
        // tool and transition the phase back so the event loop can continue polling.
        let is_paused_tool_loop = matches!(
            self.active_turn,
            ActiveTurn::ToolLoop(ref tl) if matches!(tl.phase, ToolLoopPhase::PausedForApproval)
        );
        if is_paused_tool_loop {
            let pending_call = local_call_from_pending(&pending.pending_call);
            let reason = pending.reason.clone();
            let remaining_count = pending.remaining_calls.len();
            let remaining_calls: Vec<LocalToolCall> = pending
                .remaining_calls
                .iter()
                .map(local_call_from_pending)
                .collect();
            let mut current_round = runtime_round_from_pending(&pending.current_round);

            let registry = self.build_tool_registry()?;
            let context = self.tool_context();
            let result = if approved {
                self.approve_pending_tool_call(&registry, &context, &pending_call, &reason)
                    .await?
            } else {
                self.deny_pending_tool_call(&pending_call, &reason).await?
            };

            current_round.results.push(result);

            if let ActiveTurn::ToolLoop(tl) = &mut self.active_turn {
                if remaining_count == 0 {
                    tl.rounds.push(current_round);
                    tl.phase = ToolLoopPhase::SendRequest {
                        iteration: tl.rounds.len(),
                    };
                } else {
                    tl.phase = ToolLoopPhase::ExecutingTools {
                        iteration: tl.rounds.len(),
                        tool_index: 0,
                        local_calls: remaining_calls,
                        round: current_round,
                    };
                }
            }
            return Ok(());
        }

        // No active turn — restored from snapshot or legacy path.
        // Run the full approval + continuation synchronously.
        let runtime = ProviderRuntime::new();
        let resolved = self.resolve_prompt_execution(&runtime)?;
        self.state.set_provider_context(
            Some(resolved.provider_id().to_string()),
            Some(resolved.model().to_string()),
            resolved.auth_state(),
        );

        let registry = self.build_tool_registry()?;

        let mut rounds = pending
            .rounds
            .iter()
            .map(runtime_round_from_pending)
            .collect::<Vec<_>>();
        let mut current_round = runtime_round_from_pending(&pending.current_round);
        let pending_call = local_call_from_pending(&pending.pending_call);
        let first_result = if approved {
            self.approve_pending_tool_call(
                &registry,
                &self.tool_context(),
                &pending_call,
                &pending.reason,
            )
            .await?
        } else {
            self.deny_pending_tool_call(&pending_call, &pending.reason)
                .await?
        };
        current_round.results.push(first_result);

        let remaining_calls = pending
            .remaining_calls
            .iter()
            .map(local_call_from_pending)
            .collect::<Vec<_>>();
        for (index, call) in remaining_calls.iter().cloned().enumerate() {
            match self
                .execute_tool_call(
                    &registry,
                    &self.tool_context(),
                    &call.provider_call,
                    call.use_id,
                )
                .await?
            {
                ToolExecutionOutcome::Completed(result) => current_round.results.push(result),
                ToolExecutionOutcome::Paused { reason } => {
                    self.state.pending_tool_approval = Some(PendingToolApprovalState {
                        request_prompt: pending.request_prompt.clone(),
                        rounds: rounds.iter().map(pending_round_from_runtime).collect(),
                        current_round: pending_round_from_runtime(&current_round),
                        pending_call: pending_local_call_from_runtime(&call),
                        remaining_calls: remaining_calls
                            .iter()
                            .skip(index + 1)
                            .map(pending_local_call_from_runtime)
                            .collect(),
                        reason,
                    });
                    self.persist_state_snapshot()?;
                    return Ok(());
                }
            }
        }

        rounds.push(current_round);
        self.continue_tool_loop_from_rounds(&pending.request_prompt, &runtime, &resolved, rounds)
            .await
    }
    pub(in crate::tui_runtime) async fn approve_pending_tool_call(
        &mut self,
        registry: &wonder_of_u_core::ToolRegistry,
        context: &ToolContext,
        call: &LocalToolCall,
        reason: &str,
    ) -> Result<ProviderToolResultMessage> {
        let permission_message = append_contextual_message(
            &mut self.state,
            MessagePayload::Permission {
                tool: call.provider_call.tool_name.clone(),
                decision: "allow".into(),
                reason: format!("approved in tui: {reason}"),
            },
        )?;
        let query = ToolQuery::from(context);
        if let Some(tool) = registry.resolve_enabled(&call.provider_call.tool_name, &query) {
            if tool.spec().kind == ToolKind::Interaction {
                let answer = self.interaction_pending_answer.take().unwrap_or_default();
                return self.deliver_interaction_answer(answer, call).await;
            }

            self.turn_state = TurnState::ToolExecuting;
            self.state.input_mode = InputMode::Bash;
            self.status_note = Some(format!("running tool {}", call.provider_call.tool_name));
            self.needs_render = true;
            let (progress_tx, progress_rx) = std::sync::mpsc::sync_channel::<String>(256);
            self.tool_progress_rx = Some(progress_rx);
            let mut context = context.clone();
            context.progress_tx = Some(progress_tx);
            let arguments = call.provider_call.arguments.clone();
            let use_id = call.use_id;
            let tool_arc = tool.clone();
            let result = tokio::task::spawn_blocking(move || {
                futures::executor::block_on(tool_arc.execute(context, use_id, arguments))
            })
            .await
            .map_err(|e| WonderError::internal(format!("tool task panicked: {e}")))?
            .unwrap_or_else(|e| {
                ToolResult::failure(call.use_id, format!("tool execution failed: {e}"))
            });
            self.tool_progress_rx = None;
            self.tool_progress_lines.clear();
            self.finalize_tool_result(vec![permission_message], &call.provider_call, result)
                .await
        } else {
            self.finalize_tool_result(
                vec![permission_message],
                &call.provider_call,
                ToolResult::failure(
                    call.use_id,
                    format!(
                        "tool `{}` is not available in this session",
                        call.provider_call.tool_name
                    ),
                ),
            )
            .await
        }
    }
    pub(in crate::tui_runtime) async fn deny_pending_tool_call(
        &mut self,
        call: &LocalToolCall,
        reason: &str,
    ) -> Result<ProviderToolResultMessage> {
        let permission_message = append_contextual_message(
            &mut self.state,
            MessagePayload::Permission {
                tool: call.provider_call.tool_name.clone(),
                decision: "deny".into(),
                reason: format!("denied in tui: {reason}"),
            },
        )?;
        self.turn_state = TurnState::Completed;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some(format!("denied tool {}", call.provider_call.tool_name));
        self.finalize_tool_result(
            vec![permission_message],
            &call.provider_call,
            ToolResult::failure(
                call.use_id,
                format!("tool execution denied by user: {reason}"),
            ),
        )
        .await
    }
    pub(in crate::tui_runtime) async fn continue_tool_loop_from_rounds(
        &mut self,
        request_prompt: &str,
        _runtime: &ProviderRuntime,
        resolved: &wonder_of_u_agent::ResolvedProviderExecution,
        mut rounds: Vec<ToolConversationRound>,
    ) -> Result<()> {
        let registry = self.build_tool_registry()?;
        for iteration in rounds.len()..MAX_TOOL_LOOP_ITERATIONS {
            let provider_tools = provider_tool_specs(&registry, &self.tool_context(), None)
                .into_iter()
                .map(tool_spec_to_provider_tool)
                .collect::<Vec<_>>();
            self.turn_state = TurnState::ModelRequestActive;
            self.state.input_mode = InputMode::Prompt;
            self.status_note = Some(format!(
                "continuing tool loop {}/{}",
                iteration + 1,
                MAX_TOOL_LOOP_ITERATIONS
            ));
            self.needs_render = true;

            let resolved_for_call = resolved.clone();
            let request = ToolUseRequest {
                prompt: request_prompt.to_string(),
                system_prompt: self.system_prompt_with_memory(),
                max_output_tokens: None,
                temperature: None,
                tools: provider_tools.clone(),
                rounds: rounds.clone(),
                effort_level: self.state.effort_level.clone(),
                images: Vec::new(),
            };
            let response = tokio::task::spawn_blocking(move || {
                ProviderRuntime::new().complete_with_tool_use(&resolved_for_call, &request)
            })
            .await
            .map_err(|e| WonderError::internal(format!("provider task panicked: {e}")))??;

            match response {
                ToolUseResponse::Final(response) => {
                    self.state.pending_tool_approval = None;
                    self.state
                        .set_context_window_size(response.context_window_size);
                    self.state.record_cost_usage(response.usage, None);
                    let assistant_message = append_contextual_message(
                        &mut self.state,
                        MessagePayload::AssistantText {
                            content: response.output_text,
                        },
                    )?;
                    self.persist_messages(&[assistant_message])?;
                    self.turn_state = TurnState::Completed;
                    self.state.input_mode = InputMode::Prompt;
                    self.status_note = Some("tool loop response recorded".into());
                    self.maybe_autocompact(); // may override status_note if threshold crossed
                    self.trigger_extract_memories(resolved);
                    self.trigger_status_line();
                    return Ok(());
                }
                ToolUseResponse::ToolCalls(batch) => {
                    self.state
                        .set_context_window_size(batch.context_window_size);
                    self.state.record_cost_usage(batch.usage, None);
                    let local_calls = batch
                        .calls
                        .iter()
                        .map(|call| LocalToolCall {
                            provider_call: call.clone(),
                            use_id: ToolUseId::new(),
                        })
                        .collect::<Vec<_>>();

                    let mut staged_messages = Vec::new();
                    if let Some(text) = batch
                        .assistant_text
                        .as_deref()
                        .filter(|text| !text.trim().is_empty())
                    {
                        staged_messages.push(append_contextual_message(
                            &mut self.state,
                            MessagePayload::AssistantText {
                                content: text.to_string(),
                            },
                        )?);
                    }
                    for call in &local_calls {
                        staged_messages.push(append_contextual_message(
                            &mut self.state,
                            MessagePayload::AssistantToolUse {
                                tool: call.provider_call.tool_name.clone(),
                                use_id: call.use_id,
                                input: call.provider_call.arguments.clone(),
                            },
                        )?);
                    }
                    self.persist_messages(&staged_messages)?;
                    self.needs_render = true;

                    let mut round = ToolConversationRound {
                        assistant_text: batch.assistant_text.filter(|text| !text.trim().is_empty()),
                        calls: batch.calls,
                        results: Vec::new(),
                    };
                    for (index, call) in local_calls.iter().cloned().enumerate() {
                        match self
                            .execute_tool_call(
                                &registry,
                                &self.tool_context(),
                                &call.provider_call,
                                call.use_id,
                            )
                            .await?
                        {
                            ToolExecutionOutcome::Completed(result) => round.results.push(result),
                            ToolExecutionOutcome::Paused { reason } => {
                                self.state.pending_tool_approval = Some(PendingToolApprovalState {
                                    request_prompt: request_prompt.to_string(),
                                    rounds: rounds.iter().map(pending_round_from_runtime).collect(),
                                    current_round: pending_round_from_runtime(&round),
                                    pending_call: pending_local_call_from_runtime(&call),
                                    remaining_calls: local_calls
                                        .iter()
                                        .skip(index + 1)
                                        .map(pending_local_call_from_runtime)
                                        .collect(),
                                    reason,
                                });
                                self.persist_state_snapshot()?;
                                return Ok(());
                            }
                        }
                    }
                    rounds.push(round);
                }
            }
        }

        self.state.pending_tool_approval = None;
        let limit_message = append_contextual_message(
            &mut self.state,
            MessagePayload::System {
                content: format!(
                    "tool loop stopped after {MAX_TOOL_LOOP_ITERATIONS} iterations without a final assistant response"
                ),
            },
        )?;
        self.persist_messages(&[limit_message])?;
        self.turn_state = TurnState::Completed;
        self.state.input_mode = InputMode::Prompt;
        self.status_note = Some("tool loop hit iteration limit".into());
        Ok(())
    }
    pub(in crate::tui_runtime) async fn submit_prompt(&mut self) -> Result<()> {
        let original = self.prompt.text();
        let input = original.trim().to_string();

        // If an interaction tool (ask_user) is in free-text mode (no options or
        // "Other" was selected), store the typed answer and resolve the pending
        // approval.  When in option-picker mode the dialog handler handles Enter.
        let interaction_free_text = self.is_interaction_free_text();
        if interaction_free_text && self.state.pending_tool_approval.is_some() {
            self.interaction_pending_answer = Some(input);
            self.prompt = TextBuffer::new(true);
            self.reset_history_recall();
            self.needs_render = true;
            return self.resolve_pending_tool_approval(true).await;
        }

        if input.is_empty() {
            self.status_note = Some("prompt is empty".into());
            self.turn_state = TurnState::Completed;
            self.needs_render = true;
            return Ok(());
        }

        self.prompt = TextBuffer::new(true);
        self.reset_history_recall();
        self.status_note = None;
        self.needs_render = true;

        let is_provider_prompt = !input.starts_with('/');
        let result = if input.starts_with('/') {
            self.turn_state = TurnState::CommandQueued;
            self.needs_render = true;
            self.execute_slash_command_with(&input).await
        } else {
            self.turn_state = TurnState::ModelRequestActive;
            self.needs_render = true;
            self.execute_prompt_submission(&input).await
        };

        match result {
            Ok(()) => {
                if !self.has_active_turn()
                    && !matches!(self.turn_state, TurnState::ToolPermissionPending)
                {
                    self.turn_state = TurnState::Completed;
                }
                self.needs_render = true;
            }
            Err(error) if is_provider_prompt => {
                // Remove the empty assistant placeholder that was optimistically
                // inserted before the provider request started.  If streaming had
                // already delivered partial content the message would be non-empty
                // and this check would not fire, so only a truly blank stub is
                // removed.  The UserText entry that immediately precedes it is
                // preserved so the user can see which prompt triggered the error.
                if self
                    .state
                    .messages
                    .last()
                    .is_some_and(|msg| {
                        matches!(&msg.payload, MessagePayload::AssistantText { content } if content.is_empty())
                    })
                {
                    self.state.messages.pop();
                }
                // Persist the error into history so the user can see it even
                // after scrolling; then leave the prompt cleared.
                let sanitized = sanitize_error_for_display(&error.to_string());
                if let Ok(error_msg) = append_contextual_message(
                    &mut self.state,
                    MessagePayload::ProviderError {
                        kind: "provider".into(),
                        message: sanitized,
                    },
                ) {
                    // Best-effort persist; non-fatal if storage is unavailable.
                    let _ = persist_messages_and_state(
                        self.storage_dir.as_deref(),
                        &self.state,
                        &mut self.persistence,
                        &[error_msg],
                    );
                    self.notify_transcript_changed();
                }
                self.turn_state = TurnState::Interrupted;
                self.status_note = Some("provider error — see history".into());
                self.needs_render = true;
                self.mark_autocompact_failed();
            }
            Err(error) => {
                // Slash-command errors restore the prompt so the user can retry.
                self.prompt = TextBuffer::from_text(&original, true);
                self.turn_state = TurnState::Interrupted;
                self.status_note = Some(format!("error: {error}"));
                self.needs_render = true;
                self.mark_autocompact_failed();
            }
        }

        Ok(())
    }
    pub(in crate::tui_runtime) async fn execute_prompt_submission(
        &mut self,
        input: &str,
    ) -> Result<()> {
        // Blocking limit check: if token usage is so high that the API would
        // reject the request anyway, surface a helpful error immediately rather
        // than wasting a round-trip.  Mirrors claude-code autoCompact.ts.
        if self.is_at_blocking_limit() {
            return Err(WonderError::validation(
                "context window is full — run /compact to summarise history before continuing",
            ));
        }

        let request_prompt = compose_conversation_prompt(&self.state.messages, input);
        let runtime = ProviderRuntime::new();
        let resolved = self.resolve_prompt_execution(&runtime)?;
        self.state.set_provider_context(
            Some(resolved.provider_id().to_string()),
            Some(resolved.model().to_string()),
            resolved.auth_state(),
        );

        if runtime.supports_tool_use_for(&resolved) {
            let user_message = append_contextual_message(
                &mut self.state,
                MessagePayload::UserText {
                    content: input.to_string(),
                },
            )?;
            let images = self.pending_images.drain(..).map(|(img, _)| img).collect();
            self.active_turn = ActiveTurn::ToolLoop(ActiveToolLoop {
                phase: ToolLoopPhase::SendRequest { iteration: 0 },
                request_prompt,
                system_prompt: self.system_prompt_with_memory(),
                resolved,
                rounds: Vec::new(),
                user_message: user_message.clone(),
                staged_messages: vec![user_message],
                images,
            });
            self.turn_state = TurnState::ModelRequestActive;
            self.needs_render = true;
            return Ok(());
        }

        let request = CompletionRequest {
            prompt: request_prompt,
            system_prompt: self.system_prompt_with_memory(),
            max_output_tokens: None,
            temperature: None,
            effort_level: self.state.effort_level.clone(),
            images: self.pending_images.drain(..).map(|(img, _)| img).collect(),
        };
        let streaming = runtime.supports_streaming(resolved.provider_id());

        let user_message = append_contextual_message(
            &mut self.state,
            MessagePayload::UserText {
                content: input.to_string(),
            },
        )?;
        let assistant_index = self.state.messages.len();
        let _assistant_placeholder = append_contextual_message(
            &mut self.state,
            MessagePayload::AssistantText {
                content: String::new(),
            },
        )?;
        self.needs_render = true;

        if streaming {
            let provider_id = resolved.provider_id().to_string();
            let (tx, rx) = std::sync::mpsc::channel::<StreamingCompletionEvent>();
            let resolved_clone = resolved.clone();
            let request_clone = request.clone();
            tokio::task::spawn_blocking(move || {
                let delta_tx = tx.clone();
                let runtime = ProviderRuntime::new();
                let result =
                    runtime.complete_streaming(&resolved_clone, &request_clone, move |delta| {
                        let _ = delta_tx.send(StreamingCompletionEvent::Delta(delta.to_string()));
                        Ok(())
                    });
                let _ = tx.send(StreamingCompletionEvent::Done(result));
            });
            self.active_turn = ActiveTurn::Streaming(ActiveStreaming {
                assistant_index,
                provider_id,
                event_rx: rx,
                user_message,
                resolved,
                input: input.to_string(),
            });
            self.turn_state = TurnState::ModelRequestActive;
            return Ok(());
        }

        // Non-streaming, non-tool-use: blocking path (rare legacy provider)
        let resolved_for_ref = resolved.clone();
        let resolved = resolved.clone();
        let request = request.clone();
        let response = tokio::task::spawn_blocking(move || {
            ProviderRuntime::new().complete(&resolved, &request)
        })
        .await
        .map_err(|e| WonderError::internal(format!("provider task panicked: {e}")))??;
        set_assistant_message_content(&mut self.state, assistant_index, &response.output_text)?;

        self.state
            .set_context_window_size(response.context_window_size);
        self.state.record_cost_usage(response.usage, None);
        let assistant_message = self
            .state
            .messages
            .get(assistant_index)
            .cloned()
            .ok_or_else(|| WonderError::internal("missing streamed assistant message"))?;
        persist_messages_and_state(
            self.storage_dir.as_deref(),
            &self.state,
            &mut self.persistence,
            &[user_message, assistant_message],
        )?;
        self.status_note = Some("model response recorded".into());
        self.maybe_autocompact();
        self.trigger_extract_memories(&resolved_for_ref);
        self.trigger_status_line();
        Ok(())
    }
    #[allow(dead_code)]
    pub(in crate::tui_runtime) async fn complete_streaming_with_progress(
        &mut self,
        resolved: &wonder_of_u_agent::ResolvedProviderExecution,
        request: CompletionRequest,
        assistant_index: usize,
    ) -> Result<wonder_of_u_agent::CompletionResponse> {
        let provider_id = resolved.provider_id().to_string();
        let resolved = resolved.clone();
        let (tx, mut rx) = tokio_mpsc::channel::<StreamingCompletionEvent>(256);
        let runtime = ProviderRuntime::new();
        tokio::task::spawn_blocking(move || {
            let delta_tx = tx.clone();
            let result = runtime.complete_streaming(&resolved, &request, move |delta| {
                let _ = delta_tx.blocking_send(StreamingCompletionEvent::Delta(delta.to_string()));
                Ok(())
            });
            let _ = tx.blocking_send(StreamingCompletionEvent::Done(result));
        });

        loop {
            match rx.recv().await {
                Some(StreamingCompletionEvent::Delta(delta)) => {
                    append_streamed_text(&mut self.state, assistant_index, &delta)?;
                    self.turn_state = TurnState::StreamingResponse;
                    self.status_note = Some(format!("streaming {provider_id} response"));
                    self.loading_frame = self.loading_frame.wrapping_add(1);
                    self.drain_progress_lines();
                    self.needs_render = true;
                }
                Some(StreamingCompletionEvent::Done(result)) => return result,
                Some(StreamingCompletionEvent::Progress) => {
                    self.loading_frame = self.loading_frame.wrapping_add(1);
                    self.needs_render = true;
                }
                None => {
                    return Err(WonderError::internal(
                        "streaming provider worker exited before returning a result",
                    ));
                }
            }
        }
    }
    pub(in crate::tui_runtime) async fn poll_streaming_step(
        &mut self,
        stream: &mut ActiveStreaming,
    ) -> Result<bool> {
        loop {
            match stream.event_rx.try_recv() {
                Ok(StreamingCompletionEvent::Delta(delta)) => {
                    append_streamed_text(&mut self.state, stream.assistant_index, &delta)?;
                    self.turn_state = TurnState::StreamingResponse;
                    self.status_note = Some(format!("streaming {} response", stream.provider_id));
                    self.loading_frame = self.loading_frame.wrapping_add(1);
                    self.drain_progress_lines();
                    self.needs_render = true;
                }
                Ok(StreamingCompletionEvent::Done(result)) => match result {
                    Ok(response) => {
                        let assistant_index = stream.assistant_index;
                        let user_message = stream.user_message.clone();
                        let resolved = stream.resolved.clone();
                        self.active_turn = ActiveTurn::None;

                        self.state
                            .set_context_window_size(response.context_window_size);
                        self.state.record_cost_usage(response.usage, None);
                        let assistant_message = self
                            .state
                            .messages
                            .get(assistant_index)
                            .cloned()
                            .ok_or_else(|| {
                            WonderError::internal("missing streamed assistant message")
                        })?;
                        persist_messages_and_state(
                            self.storage_dir.as_deref(),
                            &self.state,
                            &mut self.persistence,
                            &[user_message, assistant_message],
                        )?;
                        self.turn_state = TurnState::Completed;
                        self.state.input_mode = InputMode::Prompt;
                        self.status_note = Some("streamed model response recorded".into());
                        self.needs_render = true;
                        self.maybe_autocompact();
                        self.trigger_extract_memories(&resolved);
                        self.trigger_status_line();
                        return Ok(false);
                    }
                    Err(error) => {
                        let assistant_index = stream.assistant_index;
                        self.active_turn = ActiveTurn::None;
                        if let Some(msg) = self.state.messages.get(assistant_index) {
                            if matches!(&msg.payload, MessagePayload::AssistantText { content } if content.is_empty())
                            {
                                self.state.messages.remove(assistant_index);
                                if assistant_index > 0 {
                                    self.state.messages.remove(assistant_index - 1);
                                }
                            }
                        }
                        let sanitized = sanitize_error_for_display(&error.to_string());
                        if let Ok(error_msg) = append_contextual_message(
                            &mut self.state,
                            MessagePayload::ProviderError {
                                kind: "provider".into(),
                                message: sanitized,
                            },
                        ) {
                            let _ = persist_messages_and_state(
                                self.storage_dir.as_deref(),
                                &self.state,
                                &mut self.persistence,
                                &[error_msg],
                            );
                        }
                        self.turn_state = TurnState::Interrupted;
                        self.status_note = Some("provider error — see history".into());
                        self.needs_render = true;
                        self.mark_autocompact_failed();
                        return Ok(false);
                    }
                },
                Ok(StreamingCompletionEvent::Progress) => {
                    self.loading_frame = self.loading_frame.wrapping_add(1);
                    self.needs_render = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.active_turn = ActiveTurn::Streaming(ActiveStreaming {
                        assistant_index: stream.assistant_index,
                        provider_id: stream.provider_id.clone(),
                        event_rx: std::mem::replace(
                            &mut stream.event_rx,
                            std::sync::mpsc::channel::<StreamingCompletionEvent>().1,
                        ),
                        user_message: stream.user_message.clone(),
                        resolved: stream.resolved.clone(),
                        input: stream.input.clone(),
                    });
                    return Ok(true);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err(WonderError::internal(
                        "streaming provider worker exited before returning a result",
                    ));
                }
            }
        }
    }
    pub(in crate::tui_runtime) async fn poll_tool_loop_step(
        &mut self,
        tl: &mut ActiveToolLoop,
    ) -> Result<bool> {
        loop {
            match &mut tl.phase {
                ToolLoopPhase::SendRequest { iteration } => {
                    let it = *iteration;
                    let provider_tools = provider_tool_specs(
                        &self.build_tool_registry()?,
                        &self.tool_context(),
                        None,
                    )
                    .into_iter()
                    .map(tool_spec_to_provider_tool)
                    .collect::<Vec<_>>();
                    self.turn_state = TurnState::ModelRequestActive;
                    self.status_note = Some(if it == 0 {
                        format!("awaiting {} tool-aware response", tl.resolved.provider_id())
                    } else {
                        format!(
                            "continuing tool loop {}/{}",
                            it + 1,
                            MAX_TOOL_LOOP_ITERATIONS
                        )
                    });
                    self.needs_render = true;

                    let (tx, rx) =
                        std::sync::mpsc::channel::<Result<wonder_of_u_agent::ToolUseResponse>>();
                    let resolved = tl.resolved.clone();
                    let prompt = tl.request_prompt.clone();
                    let system = tl.system_prompt.clone();
                    let tools = provider_tools;
                    let rounds = tl.rounds.clone();
                    let effort = self.state.effort_level.clone();
                    // Pass images only on the first turn; subsequent rounds carry context via `rounds`.
                    let images = if it == 0 {
                        tl.images.clone()
                    } else {
                        Vec::new()
                    };
                    tokio::task::spawn_blocking(move || {
                        let request = ToolUseRequest {
                            prompt,
                            system_prompt: system,
                            max_output_tokens: None,
                            temperature: None,
                            tools,
                            rounds,
                            effort_level: effort,
                            images,
                        };
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            ProviderRuntime::new().complete_with_tool_use(&resolved, &request)
                        }))
                        .unwrap_or_else(|payload| {
                            let msg = payload
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| payload.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "unknown panic".to_string());
                            Err(WonderError::internal(format!(
                                "provider worker panicked: {msg}"
                            )))
                        });
                        let _ = tx.send(result);
                    });

                    tl.phase = ToolLoopPhase::AwaitingResponse {
                        iteration: it,
                        response_rx: rx,
                    };
                    return Ok(true);
                }
                ToolLoopPhase::AwaitingResponse {
                    iteration,
                    response_rx,
                } => {
                    let it = *iteration;
                    match response_rx.try_recv() {
                        Ok(Ok(response)) => {
                            let phase = ToolLoopPhase::AwaitingResponse {
                                iteration: it,
                                response_rx: std::sync::mpsc::channel().1,
                            };
                            let _old = std::mem::replace(&mut tl.phase, phase);
                            match response {
                                wonder_of_u_agent::ToolUseResponse::Final(response) => {
                                    let resolved = tl.resolved.clone();
                                    self.active_turn = ActiveTurn::None;
                                    self.state.pending_tool_approval = None;
                                    self.state
                                        .set_context_window_size(response.context_window_size);
                                    self.state.record_cost_usage(response.usage, None);
                                    let assistant_message = append_contextual_message(
                                        &mut self.state,
                                        MessagePayload::AssistantText {
                                            content: response.output_text,
                                        },
                                    )?;
                                    let mut msgs = tl.staged_messages.clone();
                                    msgs.push(assistant_message);
                                    self.persist_messages(&msgs)?;
                                    self.status_note = Some(if tl.rounds.is_empty() {
                                        "model response recorded".into()
                                    } else {
                                        "tool loop response recorded".into()
                                    });
                                    self.maybe_autocompact();
                                    self.trigger_extract_memories(&resolved);
                                    self.trigger_status_line();
                                    self.turn_state = TurnState::Completed;
                                    self.state.input_mode = InputMode::Prompt;
                                    return Ok(false);
                                }
                                wonder_of_u_agent::ToolUseResponse::ToolCalls(batch) => {
                                    self.state
                                        .set_context_window_size(batch.context_window_size);
                                    self.state.record_cost_usage(batch.usage, None);
                                    let local_calls = batch
                                        .calls
                                        .iter()
                                        .map(|call| LocalToolCall {
                                            provider_call: call.clone(),
                                            use_id: ToolUseId::new(),
                                        })
                                        .collect::<Vec<_>>();
                                    if let Some(text) = batch
                                        .assistant_text
                                        .as_deref()
                                        .filter(|text| !text.trim().is_empty())
                                    {
                                        tl.staged_messages.push(append_contextual_message(
                                            &mut self.state,
                                            MessagePayload::AssistantText {
                                                content: text.to_string(),
                                            },
                                        )?);
                                    }
                                    for call in &local_calls {
                                        tl.staged_messages.push(append_contextual_message(
                                            &mut self.state,
                                            MessagePayload::AssistantToolUse {
                                                tool: call.provider_call.tool_name.clone(),
                                                use_id: call.use_id,
                                                input: call.provider_call.arguments.clone(),
                                            },
                                        )?);
                                    }
                                    self.persist_messages(&tl.staged_messages)?;
                                    tl.staged_messages.clear();
                                    self.needs_render = true;

                                    let round = ToolConversationRound {
                                        assistant_text: batch
                                            .assistant_text
                                            .filter(|text| !text.trim().is_empty()),
                                        calls: batch.calls,
                                        results: Vec::new(),
                                    };
                                    tl.phase = ToolLoopPhase::ExecutingTools {
                                        iteration: it,
                                        tool_index: 0,
                                        local_calls,
                                        round,
                                    };
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            self.active_turn = ActiveTurn::None;
                            self.turn_state = TurnState::Interrupted;
                            self.status_note = Some(format!("provider error: {e}"));
                            self.needs_render = true;
                            self.mark_autocompact_failed();
                            return Ok(false);
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(true),
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            self.active_turn = ActiveTurn::None;
                            self.turn_state = TurnState::Interrupted;
                            self.state.input_mode = InputMode::Prompt;
                            self.status_note = Some(
                                "provider worker exited unexpectedly — check stderr for details"
                                    .into(),
                            );
                            self.needs_render = true;
                            self.mark_autocompact_failed();
                            return Ok(false);
                        }
                    }
                }
                ToolLoopPhase::PausedForApproval => {
                    return Ok(false);
                }
                _ => {}
            }

            // Handle ExecutingTools separately (needs ownership take)
            if matches!(tl.phase, ToolLoopPhase::ExecutingTools { .. }) {
                let phase = std::mem::replace(&mut tl.phase, ToolLoopPhase::PausedForApproval);
                let (iteration, tool_index, local_calls, mut round) = match phase {
                    ToolLoopPhase::ExecutingTools {
                        iteration,
                        tool_index,
                        local_calls,
                        round,
                    } => (iteration, tool_index, local_calls, round),
                    _ => unreachable!(),
                };

                let it = iteration;
                let idx = tool_index;

                if idx >= local_calls.len() {
                    tl.rounds.push(round);
                    let next = it + 1;
                    if next >= MAX_TOOL_LOOP_ITERATIONS {
                        self.active_turn = ActiveTurn::None;
                        self.state.pending_tool_approval = None;
                        let limit_message = append_contextual_message(
                            &mut self.state,
                            MessagePayload::System {
                                content: format!(
                                    "tool loop stopped after {MAX_TOOL_LOOP_ITERATIONS} iterations without a final assistant response"
                                ),
                            },
                        )?;
                        self.persist_messages(&[limit_message])?;
                        self.turn_state = TurnState::Completed;
                        self.state.input_mode = InputMode::Prompt;
                        self.status_note = Some("tool loop hit iteration limit".into());
                        return Ok(false);
                    }
                    tl.phase = ToolLoopPhase::SendRequest { iteration: next };
                    continue;
                }

                let call = local_calls[idx].clone();
                let registry = self.build_tool_registry()?;
                let context = self.tool_context();
                let outcome = self
                    .execute_tool_call(&registry, &context, &call.provider_call, call.use_id)
                    .await?;

                match outcome {
                    ToolExecutionOutcome::Completed(result) => {
                        round.results.push(result);
                        tl.phase = ToolLoopPhase::ExecutingTools {
                            iteration: it,
                            tool_index: idx + 1,
                            local_calls,
                            round,
                        };
                    }
                    ToolExecutionOutcome::Paused { reason } => {
                        let rounds = tl.rounds.iter().map(pending_round_from_runtime).collect();
                        let current_round = pending_round_from_runtime(&round);
                        let pending_call = pending_local_call_from_runtime(&call);
                        let remaining_calls = local_calls
                            .iter()
                            .skip(idx + 1)
                            .map(pending_local_call_from_runtime)
                            .collect();
                        self.state.pending_tool_approval = Some(PendingToolApprovalState {
                            request_prompt: tl.request_prompt.clone(),
                            rounds,
                            current_round,
                            pending_call,
                            remaining_calls,
                            reason,
                        });
                        self.persist_state_snapshot()?;
                        tl.phase = ToolLoopPhase::PausedForApproval;
                        return Ok(false);
                    }
                }
            }
        }
    }
    #[allow(dead_code)]
    pub(in crate::tui_runtime) async fn execute_tool_loop_submission(
        &mut self,
        input: &str,
        request_prompt: &str,
        _runtime: &ProviderRuntime,
        resolved: &wonder_of_u_agent::ResolvedProviderExecution,
    ) -> Result<()> {
        self.state.pending_tool_approval = None;
        let registry = self.build_tool_registry()?;

        let user_message = append_contextual_message(
            &mut self.state,
            MessagePayload::UserText {
                content: input.into(),
            },
        )?;
        let mut staged_messages = vec![user_message];
        self.needs_render = true;

        let mut rounds = Vec::<ToolConversationRound>::new();
        let system_prompt = self.system_prompt_with_memory();
        for iteration in 0..MAX_TOOL_LOOP_ITERATIONS {
            let provider_tools = provider_tool_specs(&registry, &self.tool_context(), None)
                .into_iter()
                .map(tool_spec_to_provider_tool)
                .collect::<Vec<_>>();
            self.turn_state = TurnState::ModelRequestActive;
            self.status_note = Some(if iteration == 0 {
                format!("awaiting {} tool-aware response", resolved.provider_id())
            } else {
                format!(
                    "continuing tool loop {}/{}",
                    iteration + 1,
                    MAX_TOOL_LOOP_ITERATIONS
                )
            });
            self.needs_render = true;

            let resolved_for_call = resolved.clone();
            let request = ToolUseRequest {
                prompt: request_prompt.to_string(),
                system_prompt: system_prompt.clone(),
                max_output_tokens: None,
                temperature: None,
                tools: provider_tools.clone(),
                rounds: rounds.clone(),
                effort_level: self.state.effort_level.clone(),
                images: Vec::new(),
            };
            let response = tokio::task::spawn_blocking(move || {
                ProviderRuntime::new().complete_with_tool_use(&resolved_for_call, &request)
            })
            .await
            .map_err(|e| WonderError::internal(format!("provider task panicked: {e}")))??;

            match response {
                ToolUseResponse::Final(response) => {
                    self.state.pending_tool_approval = None;
                    self.state
                        .set_context_window_size(response.context_window_size);
                    self.state.record_cost_usage(response.usage, None);
                    let assistant_message = append_contextual_message(
                        &mut self.state,
                        MessagePayload::AssistantText {
                            content: response.output_text,
                        },
                    )?;
                    staged_messages.push(assistant_message);
                    self.persist_messages(&staged_messages)?;
                    self.status_note = Some(if rounds.is_empty() {
                        "model response recorded".into()
                    } else {
                        "tool loop response recorded".into()
                    });
                    self.maybe_autocompact(); // may override status_note if threshold crossed
                    self.trigger_extract_memories(resolved);
                    self.trigger_status_line();
                    return Ok(());
                }
                ToolUseResponse::ToolCalls(batch) => {
                    self.state
                        .set_context_window_size(batch.context_window_size);
                    self.state.record_cost_usage(batch.usage, None);
                    let local_calls = batch
                        .calls
                        .iter()
                        .map(|call| LocalToolCall {
                            provider_call: call.clone(),
                            use_id: ToolUseId::new(),
                        })
                        .collect::<Vec<_>>();

                    if let Some(text) = batch
                        .assistant_text
                        .as_deref()
                        .filter(|text| !text.trim().is_empty())
                    {
                        staged_messages.push(append_contextual_message(
                            &mut self.state,
                            MessagePayload::AssistantText {
                                content: text.to_string(),
                            },
                        )?);
                    }
                    for call in &local_calls {
                        staged_messages.push(append_contextual_message(
                            &mut self.state,
                            MessagePayload::AssistantToolUse {
                                tool: call.provider_call.tool_name.clone(),
                                use_id: call.use_id,
                                input: call.provider_call.arguments.clone(),
                            },
                        )?);
                    }
                    self.persist_messages(&staged_messages)?;
                    staged_messages.clear();
                    self.needs_render = true;

                    let mut round = ToolConversationRound {
                        assistant_text: batch.assistant_text.filter(|text| !text.trim().is_empty()),
                        calls: batch.calls,
                        results: Vec::new(),
                    };
                    for (index, call) in local_calls.iter().cloned().enumerate() {
                        let outcome = self
                            .execute_tool_call(
                                &registry,
                                &self.tool_context(),
                                &call.provider_call,
                                call.use_id,
                            )
                            .await?;
                        match outcome {
                            ToolExecutionOutcome::Completed(result) => round.results.push(result),
                            ToolExecutionOutcome::Paused { reason } => {
                                self.state.pending_tool_approval = Some(PendingToolApprovalState {
                                    request_prompt: request_prompt.to_string(),
                                    rounds: rounds.iter().map(pending_round_from_runtime).collect(),
                                    current_round: pending_round_from_runtime(&round),
                                    pending_call: pending_local_call_from_runtime(&call),
                                    remaining_calls: local_calls
                                        .iter()
                                        .skip(index + 1)
                                        .map(pending_local_call_from_runtime)
                                        .collect(),
                                    reason,
                                });
                                self.persist_state_snapshot()?;
                                return Ok(());
                            }
                        }
                    }
                    rounds.push(round);
                }
            }
        }

        self.state.pending_tool_approval = None;
        let limit_message = append_contextual_message(
            &mut self.state,
            MessagePayload::System {
                content: format!(
                    "tool loop stopped after {MAX_TOOL_LOOP_ITERATIONS} iterations without a final assistant response"
                ),
            },
        )?;
        self.persist_messages(&[limit_message])?;
        self.status_note = Some("tool loop hit iteration limit".into());
        Ok(())
    }
    pub(in crate::tui_runtime) fn resolve_prompt_execution(
        &self,
        runtime: &ProviderRuntime,
    ) -> Result<wonder_of_u_agent::ResolvedProviderExecution> {
        if self.storage_dir.is_some() || !self.state.fast_mode {
            return runtime
                .resolve_execution(self.storage_dir.as_deref(), ProviderSelection::default());
        }

        let resolver = ProviderResolver::builtin();
        let fallback_provider = resolver.load_report(None)?.provider;
        let provider = self.state.provider.clone().or(fallback_provider);
        let selection = provider
            .as_deref()
            .and_then(|provider_id| {
                resolver
                    .registry()
                    .get(provider_id)
                    .map(|descriptor| (provider_id.to_string(), descriptor))
            })
            .and_then(|(provider_id, descriptor)| {
                descriptor
                    .preferred_fast_model()
                    .map(|model| ProviderSelection::new(Some(provider_id), Some(model.id.clone())))
            })
            .unwrap_or_default();
        runtime.resolve_execution(None, selection)
    }
    #[cfg(test)]
    pub(in crate::tui_runtime) async fn execute_slash_command(
        &mut self,
        input: &str,
    ) -> Result<()> {
        self.execute_slash_command_with(input).await
    }
    pub(in crate::tui_runtime) async fn execute_slash_command_with(
        &mut self,
        input: &str,
    ) -> Result<()> {
        let trimmed = input.trim();

        // /diff opens the interactive diff dialog.
        if trimmed == "/diff" || trimmed.starts_with("/diff ") {
            let arg = trimmed.strip_prefix("/diff").map(str::trim).unwrap_or("");
            if arg == "--staged" || arg == "--cached" {
                self.status_note = Some("staged diff not yet supported in dialog".into());
                return Ok(());
            }
            self.open_diff_dialog()?;
            return Ok(());
        }

        // Handle /sidebar as a TUI-local toggle that never reaches the command registry.
        let sub = trimmed
            .strip_prefix("/sidebar")
            .map(str::trim)
            .unwrap_or("");
        if trimmed == "/sidebar" || trimmed.starts_with("/sidebar ") {
            let enabled = match sub {
                "on" => Some(true),
                "off" => Some(false),
                "toggle" | "" => None,
                // Unknown subcommand — treat as a plain toggle.
                _ => None,
            };
            match enabled {
                Some(v) => {
                    self.sidebar_visible = v;
                    self.status_note = Some(if v {
                        "sidebar on".into()
                    } else {
                        "sidebar off".into()
                    });
                    self.needs_render = true;
                }
                None => {
                    self.toggle_sidebar();
                }
            }
            return Ok(());
        }
        if trimmed == "/thinking" || trimmed.starts_with("/thinking ") {
            let arg = trimmed
                .strip_prefix("/thinking")
                .map(str::trim)
                .unwrap_or("");
            let output =
                commands::execute_thinking_command(&self.state, (!arg.is_empty()).then_some(arg))?;
            let effort = match self.state.thinking_effort {
                wonder_of_u_core::app::ThinkingEffort::Low => "low",
                wonder_of_u_core::app::ThinkingEffort::Medium => "medium",
                wonder_of_u_core::app::ThinkingEffort::High => "high",
            };
            match arg {
                "on" => {
                    self.state.set_thinking_enabled(true);
                    self.status_note = Some("thinking on".into());
                }
                "off" => {
                    self.state.set_thinking_enabled(false);
                    self.status_note = Some("thinking off".into());
                }
                "low" => {
                    self.state
                        .set_thinking_effort(wonder_of_u_core::app::ThinkingEffort::Low);
                    self.status_note = Some("thinking effort low".into());
                }
                "medium" => {
                    self.state
                        .set_thinking_effort(wonder_of_u_core::app::ThinkingEffort::Medium);
                    self.status_note = Some("thinking effort medium".into());
                }
                "high" => {
                    self.state
                        .set_thinking_effort(wonder_of_u_core::app::ThinkingEffort::High);
                    self.status_note = Some("thinking effort high".into());
                }
                "" => {
                    self.status_note = Some(format!(
                        "thinking {} · effort {effort}",
                        if self.state.thinking_enabled {
                            "on"
                        } else {
                            "off"
                        }
                    ));
                }
                _ => {}
            }
            // TODO: Include `self.state.thinking_enabled` in provider completion requests.
            self.dismiss_dialog();
            self.record_command_message(input, Some(&output))?;
            self.needs_render = true;
            return Ok(());
        }
        if trimmed == "/help" || trimmed.starts_with("/help ") {
            let arg = trimmed.strip_prefix("/help").map(str::trim).unwrap_or("");
            if !arg.is_empty() {
                return Err(WonderError::validation(
                    "/help does not take arguments in the TUI",
                ));
            }
            let output = commands::execute_help_command()?;
            self.dismiss_dialog();
            self.record_command_message(input, Some(&output))?;
            self.status_note = Some("help".into());
            self.needs_render = true;
            return Ok(());
        }
        if trimmed == "/stats" || trimmed.starts_with("/stats ") {
            let arg = trimmed.strip_prefix("/stats").map(str::trim).unwrap_or("");
            if !arg.is_empty() {
                return Err(WonderError::validation("/stats does not take arguments"));
            }
            let output = commands::execute_stats_command(&self.state)?;
            self.dismiss_dialog();
            self.record_command_message(input, Some(&output))?;
            self.status_note = Some("session stats".into());
            self.needs_render = true;
            return Ok(());
        }
        if trimmed == "/settings" || trimmed.starts_with("/settings ") {
            let arg = trimmed
                .strip_prefix("/settings")
                .map(str::trim)
                .unwrap_or("");
            if !arg.is_empty() {
                return Err(WonderError::validation("/settings does not take arguments"));
            }
            let output = commands::execute_settings_command(&self.state)?;
            self.dismiss_dialog();
            self.record_command_message(input, Some(&output))?;
            self.status_note = Some("settings".into());
            self.needs_render = true;
            return Ok(());
        }
        // Handle /login as a TUI-local shortcut when no CLI flags are present.
        // /login              → open the /setup overlay (guided choice)
        // /login copilot      → open Copilot OAuth device-code flow directly
        // /login <provider>   → open provider API-key form for that provider
        // /login --provider … → fall through to LoginCommand (keeps CLI parity)
        if trimmed == "/login" || trimmed.starts_with("/login ") {
            let arg = trimmed
                .strip_prefix("/login")
                .map(str::trim)
                .unwrap_or("")
                .trim();
            // If the user passed real CLI flags, fall through to the command registry.
            if !arg.starts_with('-') {
                match arg {
                    "" => {
                        // No provider specified — open the setup hub so the user can
                        // choose interactively.
                        return Box::pin(self.execute_slash_command_with("/setup")).await;
                    }
                    "copilot" => {
                        self.open_copilot_oauth_flow();
                        return Ok(());
                    }
                    provider => {
                        // Check whether this is a known API-key provider; if so open
                        // the provider form pre-filtered to that provider.
                        let known = ProviderResolver::builtin()
                            .registry()
                            .get(provider)
                            .map(|p| p.auth_kind == AuthMaterialKind::ApiKey)
                            .unwrap_or(false);
                        if known {
                            self.open_provider_form(ProviderFormKind::ApiKey);
                        } else {
                            // Unknown provider — open the setup hub and let the user pick.
                            return Box::pin(self.execute_slash_command_with("/setup")).await;
                        }
                        return Ok(());
                    }
                }
            }
        }
        if trimmed == "/search" || trimmed.starts_with("/search ") {
            let query = trimmed
                .strip_prefix("/search")
                .map(str::trim)
                .unwrap_or_default();
            self.open_global_search(query);
            return Ok(());
        }
        // /select enters terminal-native text selection mode so the user can
        // click-drag to select text in the transcript and copy to clipboard.
        if trimmed == "/select" {
            self.enter_selection_mode();
            return Ok(());
        }
        // /resume (no args) → interactive session picker
        if trimmed == "/resume" {
            self.open_log_selector();
            return Ok(());
        }
        if trimmed == "/export" {
            self.open_export_dialog();
            return Ok(());
        }
        if trimmed == "/output-style" || trimmed == "/output-style list" {
            self.open_output_style_picker();
            return Ok(());
        }
        if trimmed == "/memory-files" || trimmed == "/memory files" {
            self.open_memory_file_selector();
            return Ok(());
        }
        if trimmed == "/hooks-menu" || trimmed == "/hooks menu" {
            self.open_hooks_menu();
            return Ok(());
        }
        let invocation = parse_slash_command(input)
            .ok_or_else(|| WonderError::validation("invalid slash command"))?;
        let context = self.command_context();
        let query = CommandQuery::from(&context);
        let command = match self.registry.resolve_enabled(&invocation.name, &query) {
            Some(command) => command,
            None => commands::resolve_dynamic_command(
                &context.cwd,
                self.storage_dir.as_deref(),
                &invocation.name,
                &query,
            )?
            .ok_or_else(|| WonderError::not_found("command", invocation.name.clone()))?,
        };
        // Capture the immediate flag before consuming `command` via execute.
        let is_immediate = command.spec().immediate;
        let output = command.execute(context, invocation.clone()).await?;
        let (text, exit_requested) = command_output_text(output);
        if self.persistence.persisted {
            self.restore_current_session()?;
        }
        self.apply_inline_command_hints(input);
        self.apply_command_output_hints(text.as_deref());
        let had_queued_commands = !self.state.queued_commands.is_empty();
        let view_action = parse_view_action_hint(text.as_deref());
        if view_action.is_none()
            && self.pending_external_editor.is_none()
            && self.pending_model_picker.is_none()
            && self.pending_permission_picker.is_none()
            && self.pending_memory_picker.is_none()
            && self.pending_tag_removal.is_none()
            && self.pending_theme_picker.is_none()
            && self.pending_setup_overlay.is_none()
        {
            self.record_command_message(input, text.as_deref())?;
        }
        let _ = self.refresh_runtime_state()?;
        // Immediate commands (e.g. /exit, /clear, /color, /effort, /fast,
        // /hooks) must not trigger queued-prompt draining: they act
        // synchronously and their side-effects should not cascade.
        if !is_immediate {
            self.drain_queued_commands().await?;
        }
        self.exit_requested |= exit_requested;
        if exit_requested {
            self.status_note = Some("exit requested".into());
        } else if let Some(view_action) = view_action {
            self.status_note = Some(match view_action {
                ViewActionHint::Clear => "conversation cleared".into(),
                ViewActionHint::Compact => {
                    // Compact succeeded — reset the circuit breaker.
                    self.autocompact_failures = 0;
                    self.autocompact_pending = false;
                    "conversation compacted".into()
                }
            });
        } else if self.dialog.as_ref().is_some_and(|dialog| {
            dialog.title == "Context Usage"
                || dialog.title == "Activity Stats"
                || dialog.title == "Usage"
                || dialog.title == "Theme"
                || dialog.title == "Color"
                || dialog.title == "Fast"
                || dialog.title == "Brief"
                || dialog.title == "Optimize Token"
                || dialog.title == "Effort"
                || dialog.title == "Feedback"
                || dialog.title == "Insights"
                || dialog.title == "Upgrade"
                || dialog.title == "Desktop"
                || dialog.title == "Mobile"
                || dialog.title == "Chrome"
                || dialog.title == "IDE Integration"
                || dialog.title == "Background Tasks"
                || dialog.title == "Release Notes"
                || dialog.title == "Version"
                || dialog.title == "Hooks"
                || dialog.title == "Keybindings"
                || dialog.title == "Privacy Settings"
                || dialog.title == "Terminal Setup"
        }) {
        } else if parse_vim_toggle_hint(text.as_deref().unwrap_or_default())
            || parse_vim_mode_hint(text.as_deref().unwrap_or_default()).is_some()
        {
            self.status_note = Some(match self.vim.mode() {
                VimMode::Insert => "vim insert".into(),
                VimMode::Normal => "vim normal".into(),
                VimMode::Visual => "vim visual".into(),
            });
        } else if parse_insights_hint(text.as_deref().unwrap_or_default()) {
            self.status_note = Some("insights queued".into());
        } else if let Some(enabled) = parse_fast_mode_hint(text.as_deref().unwrap_or_default()) {
            self.status_note = Some(format!("fast {}", if enabled { "on" } else { "off" }));
        } else if let Some(enabled) = parse_brief_mode_hint(text.as_deref().unwrap_or_default()) {
            self.status_note = Some(format!("brief {}", if enabled { "on" } else { "off" }));
        } else if let Some(enabled) =
            parse_optimize_token_mode_hint(text.as_deref().unwrap_or_default())
        {
            self.status_note = Some(format!(
                "optimize token {}",
                if enabled { "on" } else { "off" }
            ));
        } else if let Some(effort) = parse_effort_hint(text.as_deref().unwrap_or_default()) {
            self.status_note = Some(format!("effort {effort}"));
        } else if let Some(color) = parse_session_color_hint(text.as_deref().unwrap_or_default()) {
            self.status_note = Some(format!("color {color}"));
        } else if self.pending_permission_picker.is_some()
            || self.pending_memory_picker.is_some()
            || self.pending_tag_removal.is_some()
            || self.pending_theme_picker.is_some()
            || self.pending_model_picker.is_some()
            || self.pending_setup_overlay.is_some()
        {
        } else if self.pending_external_editor.is_some() {
            self.status_note = Some("opening file in editor".into());
        } else if !had_queued_commands {
            self.status_note = Some("slash command recorded".into());
        }
        if invocation.name == "tasks"
            && invocation.args.is_empty()
            && self.state.background_tasks.is_empty()
        {
            self.dismiss_dialog();
            self.dialog = Some(DialogView::notice(
                "Background Tasks",
                ["No background tasks running"],
            ));
            self.task_notice_ttl = Some(TASK_NOTICE_TTL);
            self.status_note = Some("no background tasks running".into());
            self.push_notification(
                "background-tasks",
                NotificationSeverity::Info,
                "Background Tasks",
                ["No background tasks running"],
                Some(SHELL_NOTIFICATION_TTL),
                true,
            );
        }
        self.needs_render = true;
        Ok(())
    }
    pub(in crate::tui_runtime) fn apply_inline_command_hints(&mut self, input: &str) {
        let Ok(tokens) = shell_words::split(input.trim_start_matches('/')) else {
            return;
        };
        if tokens.first().map(String::as_str) != Some("model")
            || tokens.get(1).map(String::as_str) != Some("set")
        {
            return;
        }

        let mut provider = None;
        let mut model = None;
        let mut index = 2usize;
        while index < tokens.len() {
            match tokens[index].as_str() {
                "--provider" if index + 1 < tokens.len() => {
                    provider = Some(tokens[index + 1].clone());
                    index += 2;
                }
                "--model" if index + 1 < tokens.len() => {
                    model = Some(tokens[index + 1].clone());
                    index += 2;
                }
                selection if !selection.starts_with('-') => {
                    if let Some((selection_provider, selection_model)) = selection.split_once(':') {
                        provider.get_or_insert_with(|| selection_provider.to_string());
                        model.get_or_insert_with(|| selection_model.to_string());
                    }
                    index += 1;
                }
                _ => {
                    index += 1;
                }
            }
        }

        let Some(provider) = provider else {
            return;
        };
        let resolver = ProviderResolver::builtin();
        let Some(descriptor) = resolver.registry().get(&provider) else {
            return;
        };
        let auth = match descriptor.auth_kind {
            wonder_of_u_core::AuthMaterialKind::None => AuthState::not_required(),
            wonder_of_u_core::AuthMaterialKind::ApiKey => AuthState::missing(descriptor.auth_kind),
            wonder_of_u_core::AuthMaterialKind::AwsSigV4 => {
                AuthState::missing(descriptor.auth_kind)
            }
            wonder_of_u_core::AuthMaterialKind::AwsBearer => {
                AuthState::missing(descriptor.auth_kind)
            }
            wonder_of_u_core::AuthMaterialKind::AwsProfile => {
                AuthState::missing(descriptor.auth_kind)
            }
            wonder_of_u_core::AuthMaterialKind::GcpOAuth2 => {
                AuthState::missing(descriptor.auth_kind)
            }
            wonder_of_u_core::AuthMaterialKind::OAuth => AuthState::pending(
                descriptor.auth_kind,
                None,
                "run `wonder-of-u login --provider copilot`",
            ),
        };
        self.state.set_provider_context(
            Some(provider),
            Some(model.unwrap_or_else(|| descriptor.default_model.clone())),
            auth,
        );
    }
    pub(in crate::tui_runtime) fn apply_command_output_hints(&mut self, text: Option<&str>) {
        let Some(text) = text else {
            return;
        };
        if let Some(directory) = parse_additional_working_directory_hint(text) {
            self.state.add_additional_working_directory(directory);
        }
        if parse_vim_toggle_hint(text) {
            self.vim_enabled = true;
            self.vim = VimState::new(match self.vim.mode() {
                VimMode::Insert => VimMode::Normal,
                VimMode::Normal | VimMode::Visual => VimMode::Insert,
            });
        }
        if let Some(mode) = parse_vim_mode_hint(text) {
            self.vim_enabled = true;
            self.vim = VimState::new(mode);
        }
        if let Some(picker) = parse_permission_picker_state(text) {
            self.open_permission_picker(picker);
        } else {
            self.pending_permission_picker = None;
        }
        if let Some(picker) = parse_memory_picker_state(text) {
            self.open_memory_picker(picker);
        } else {
            self.pending_memory_picker = None;
        }
        if let Some(tag) = parse_tag_remove_confirmation(text) {
            self.open_tag_removal_confirmation(TagRemovalState {
                original_input: format!("/tag {tag}"),
                tag,
            });
        } else {
            self.pending_tag_removal = None;
        }
        if let Some(picker) = parse_theme_picker_state(text) {
            self.open_theme_picker(picker);
        } else {
            self.pending_theme_picker = None;
        }
        if let Some(picker) = parse_model_picker_state(text) {
            self.open_model_picker(picker);
        } else {
            self.pending_model_picker = None;
        }
        if let Some(overlay) = parse_setup_overlay_state(text) {
            self.open_setup_overlay(overlay);
        } else {
            // Only clear setup overlay if we actually parsed some command output;
            // avoid clobbering it mid-interaction when unrelated hints fire.
            if text.lines().any(|line| line.starts_with("setup_menu=")) {
                self.pending_setup_overlay = None;
            }
        }
        if let Some(theme) = parse_theme_hint(text) {
            self.state
                .set_theme((theme != "default").then(|| theme.to_string()));
        }
        if let Some(color) = parse_session_color_hint(text) {
            self.state
                .set_session_color((color != "default").then(|| color.to_string()));
        }
        if let Some(enabled) = parse_fast_mode_hint(text) {
            self.state.set_fast_mode(enabled);
        }
        if let Some(enabled) = parse_brief_mode_hint(text) {
            self.state.set_brief_mode(enabled);
        }
        if let Some(enabled) = parse_optimize_token_mode_hint(text) {
            self.state.set_optimize_token_mode(enabled);
        }
        if let Some(effort) = parse_effort_hint(text) {
            self.state
                .set_effort_level((effort != "auto").then(|| effort.to_string()));
        }
        if let Some(tags) = parse_session_tags_hint(text) {
            self.state.set_session_tags(tags);
        }
        self.pending_external_editor = parse_external_editor_request(text, &self.state.session.cwd);
        if let Some((title, body, note)) = parse_known_notice(text) {
            self.dialog = Some(DialogView::notice(title.clone(), body.clone()));
            self.status_note = Some(note.into());
            self.push_notification(
                format!("notice:{}", title.to_ascii_lowercase().replace(' ', "-")),
                NotificationSeverity::Info,
                title,
                body,
                Some(SHELL_NOTIFICATION_TTL),
                false,
            );
            self.needs_render = true;
        } else if self.state.input_mode == InputMode::Prompt
            && !self.has_picker_overlay()
            && self
                .dialog
                .as_ref()
                .is_some_and(|dialog| dialog.kind() == wonder_of_u_tui::DialogKind::Notice)
        {
            self.dialog = None;
            self.needs_render = true;
        }
        for queued_prompt in text
            .lines()
            .filter_map(|line| line.strip_prefix("enqueue_prompt="))
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            self.state
                .queue_command(queued_prompt.to_string(), QueuePlacement::Now);
        }
        if let Some(mode) = text
            .lines()
            .find_map(|line| line.strip_prefix("permission_mode="))
            .and_then(parse_permission_mode_hint)
        {
            // Track the pre-plan mode so we can restore it on `/plan exit`.
            // Entering plan: remember what we're leaving so exit restores
            // correctly (e.g. AcceptEdits/BypassPermissions, not always Default).
            // Leaving plan: restore the saved pre-plan mode and clear the slot,
            // ignoring whatever the command hardcoded (it doesn't know our origin).
            let apply_mode = if mode == PermissionMode::Plan
                && self.state.permission_mode != PermissionMode::Plan
            {
                // Entering plan mode — save origin.
                self.pre_plan_permission_mode = Some(self.state.permission_mode);
                mode
            } else if mode != PermissionMode::Plan
                && self.state.permission_mode == PermissionMode::Plan
            {
                // Leaving plan mode — restore saved origin, or use the
                // command-supplied mode as a passthrough fallback.
                self.pre_plan_permission_mode.take().unwrap_or(mode)
            } else {
                // Any other transition (Default→AcceptEdits, etc.) — apply as-is
                // and clear the stale pre-plan slot if we somehow have one.
                self.pre_plan_permission_mode = None;
                mode
            };
            self.state.permission_mode = apply_mode;
        }
        let Some(selection) = text
            .lines()
            .find_map(|line| line.strip_prefix("provider_selection="))
        else {
            return;
        };

        let (provider, model) = if selection == "none" {
            (None, None)
        } else {
            match selection.split_once(':') {
                Some((provider, model)) => (Some(provider.to_string()), Some(model.to_string())),
                None => return,
            }
        };
        let resolver = ProviderResolver::builtin();
        let auth = match text
            .lines()
            .find_map(|line| line.strip_prefix("auth_status="))
        {
            Some("not_required") => AuthState::not_required(),
            Some("missing") => provider
                .as_deref()
                .and_then(|provider_id| resolver.registry().get(provider_id))
                .map(|provider| AuthState::missing(provider.auth_kind))
                .unwrap_or_default(),
            Some("pending") => provider
                .as_deref()
                .and_then(|provider_id| resolver.registry().get(provider_id))
                .map(|provider| {
                    AuthState::pending(provider.auth_kind, None, "reported by slash command output")
                })
                .unwrap_or_default(),
            _ => self.state.auth.clone(),
        };

        self.state.set_provider_context(provider, model, auth);
    }
    pub(in crate::tui_runtime) fn record_command_message(
        &mut self,
        input: &str,
        output: Option<&str>,
    ) -> Result<()> {
        let message = append_contextual_message(
            &mut self.state,
            MessagePayload::Command {
                input: input.into(),
                output: output.map(ToString::to_string),
            },
        )?;
        persist_messages_and_state(
            self.storage_dir.as_deref(),
            &self.state,
            &mut self.persistence,
            &[message],
        )?;
        self.notify_transcript_changed();
        Ok(())
    }
    pub(in crate::tui_runtime) fn persist_messages(
        &mut self,
        messages: &[MessageEnvelope],
    ) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        persist_messages_and_state(
            self.storage_dir.as_deref(),
            &self.state,
            &mut self.persistence,
            messages,
        )?;
        // Keep scroll state consistent: follow-tail stays pinned; scrolled-up
        // mode preserves the viewport relative to the bottom of the transcript.
        self.notify_transcript_changed();
        Ok(())
    }
    pub(in crate::tui_runtime) fn drain_progress_lines(&mut self) {
        if let Some(rx) = self.tool_progress_rx.as_mut() {
            while let Ok(line) = rx.try_recv() {
                if !line.trim().is_empty() {
                    self.tool_progress_lines.push_back(line);
                    if self.tool_progress_lines.len() > 8 {
                        self.tool_progress_lines.pop_front();
                    }
                }
            }
        }
    }
    pub(in crate::tui_runtime) async fn execute_tool_call(
        &mut self,
        registry: &wonder_of_u_core::ToolRegistry,
        context: &ToolContext,
        call: &ProviderToolCall,
        use_id: ToolUseId,
    ) -> Result<ToolExecutionOutcome> {
        let query = ToolQuery::from(context);
        if let Some(tool) = registry.resolve_enabled(&call.tool_name, &query) {
            if tool.spec().kind == ToolKind::Interaction {
                let (question, options) = extract_interaction_info(&call.arguments);
                self.interaction_question = Some(question.clone());
                self.interaction_options = options.clone();
                self.interaction_other_mode = false;

                if options.is_empty() {
                    self.dialog = Some(DialogView {
                        title: format!("● {}", call.tool_name),
                        body: question
                            .lines()
                            .map(ToString::to_string)
                            .chain(std::iter::once(String::new()))
                            .chain(std::iter::once(
                                "Type your answer in the prompt box below and press Enter ↵".into(),
                            ))
                            .collect(),
                        actions: vec![],
                        selected_action: 0,
                    });
                    self.state.input_mode = InputMode::Prompt;
                    self.status_note = Some("type answer ↵ to send".into());
                } else {
                    let mut actions: Vec<DialogActionView> = options
                        .iter()
                        .map(|opt| DialogActionView::new(opt.clone(), false))
                        .collect();
                    actions.push(DialogActionView::new("Other (type your answer)", false));
                    self.dialog = Some(DialogView {
                        title: format!("● {}", call.tool_name),
                        body: question.lines().map(ToString::to_string).collect(),
                        actions,
                        selected_action: 0,
                    });
                    self.state.input_mode = InputMode::PermissionPending;
                    self.status_note = Some("↑↓ select · Enter confirm".into());
                }
                self.turn_state = TurnState::ToolPermissionPending;
                self.needs_render = true;
                return Ok(ToolExecutionOutcome::Paused {
                    reason: format!("ask_user: {question}"),
                });
            }

            if let Err(error) = tool.validate_input(&call.arguments) {
                return self
                    .finalize_tool_result(
                        Vec::new(),
                        call,
                        ToolResult::failure(
                            use_id,
                            format!("tool input validation failed: {error}"),
                        ),
                    )
                    .await
                    .map(ToolExecutionOutcome::Completed);
            }
            match tool.permission_decision(context, &call.arguments) {
                PermissionDecision::Allow { .. } => self
                    .run_tool_call(tool, context, call, use_id)
                    .await
                    .map(ToolExecutionOutcome::Completed),
                other @ PermissionDecision::Ask { .. } => {
                    let spec = tool.spec();
                    self.turn_state = TurnState::ToolPermissionPending;
                    self.state.input_mode = InputMode::PermissionPending;
                    let reason = other.reason().to_string();
                    self.dialog = Some(permission_dialog_for_tool_call(
                        &call.tool_name,
                        &call.arguments,
                        spec.read_only,
                        spec.destructive,
                        reason.clone(),
                    ));
                    let permission_message = append_contextual_message(
                        &mut self.state,
                        MessagePayload::Permission {
                            tool: call.tool_name.clone(),
                            decision: "ask".into(),
                            reason: reason.clone(),
                        },
                    )?;
                    self.persist_messages(&[permission_message])?;
                    self.status_note = Some(permission_required_status(&call.tool_name));
                    self.needs_render = true;
                    Ok(ToolExecutionOutcome::Paused { reason })
                }
                other @ PermissionDecision::Deny { .. } => {
                    let reason = other.reason().to_string();
                    self.turn_state = TurnState::ToolPermissionPending;
                    self.state.input_mode = InputMode::PermissionPending;
                    self.dialog = Some(DialogView::notice(
                        "Permission denied",
                        [
                            format!("Tool `{}` was denied.", call.tool_name),
                            reason.clone(),
                        ],
                    ));
                    let permission_message = append_contextual_message(
                        &mut self.state,
                        MessagePayload::Permission {
                            tool: call.tool_name.clone(),
                            decision: permission_decision_label(&other).into(),
                            reason: reason.clone(),
                        },
                    )?;
                    self.finalize_tool_result(
                        vec![permission_message],
                        call,
                        ToolResult::failure(use_id, format!("tool execution denied: {reason}")),
                    )
                    .await
                    .map(ToolExecutionOutcome::Completed)
                }
            }
        } else {
            self.finalize_tool_result(
                Vec::new(),
                call,
                ToolResult::failure(
                    use_id,
                    format!("tool `{}` is not available in this session", call.tool_name),
                ),
            )
            .await
            .map(ToolExecutionOutcome::Completed)
        }
    }
    pub(in crate::tui_runtime) async fn run_tool_call(
        &mut self,
        tool: std::sync::Arc<dyn wonder_of_u_core::Tool>,
        context: &ToolContext,
        call: &ProviderToolCall,
        use_id: ToolUseId,
    ) -> Result<ProviderToolResultMessage> {
        debug_assert_ne!(
            tool.spec().kind,
            ToolKind::Interaction,
            "interaction tools must be handled before run_tool_call"
        );

        self.turn_state = TurnState::ToolExecuting;
        self.state.input_mode = InputMode::Bash;
        self.status_note = Some(format!("running tool {}", call.tool_name));
        self.tool_progress_lines.clear();

        let (progress_tx, progress_rx) = std::sync::mpsc::sync_channel::<String>(256);
        self.tool_progress_rx = Some(progress_rx);
        let mut context = context.clone();
        context.progress_tx = Some(progress_tx);

        self.needs_render = true;
        let arguments = call.arguments.clone();
        let tool_arc = tool.clone();
        let result = tokio::task::spawn_blocking(move || {
            futures::executor::block_on(tool_arc.execute(context, use_id, arguments))
        })
        .await
        .map_err(|e| WonderError::internal(format!("tool task panicked: {e}")))?
        .unwrap_or_else(|e| ToolResult::failure(use_id, format!("tool execution failed: {e}")));
        self.tool_progress_rx = None;
        self.tool_progress_lines.clear();
        self.finalize_tool_result(Vec::new(), call, result).await
    }
    pub(in crate::tui_runtime) async fn deliver_interaction_answer(
        &mut self,
        answer: String,
        call: &LocalToolCall,
    ) -> Result<ProviderToolResultMessage> {
        self.interaction_question = None;
        self.interaction_options.clear();
        self.interaction_other_mode = false;
        self.dialog = None;
        let result = ToolResult::success(call.use_id, answer);
        self.finalize_tool_result(Vec::new(), &call.provider_call, result)
            .await
    }
    pub(in crate::tui_runtime) async fn finalize_tool_result(
        &mut self,
        mut messages: Vec<MessageEnvelope>,
        call: &ProviderToolCall,
        result: ToolResult,
    ) -> Result<ProviderToolResultMessage> {
        let cwd = self.state.session.cwd.clone();
        let result = process_tool_effects(
            result,
            self.storage_dir.as_deref(),
            &cwd,
            Some(&mut self.state),
        );
        let _ = commands::apply_worktree_tool_result(&mut self.state, &result)?;
        messages.push(append_contextual_message(
            &mut self.state,
            MessagePayload::ToolResult {
                tool: call.tool_name.clone(),
                use_id: result.use_id,
                success: result.success,
                content: result.content.clone(),
            },
        )?);
        self.persist_messages(&messages)?;
        self.needs_render = true;
        Ok(ProviderToolResultMessage {
            call_id: call.call_id.clone(),
            content: render_provider_tool_result(&result),
        })
    }
}
