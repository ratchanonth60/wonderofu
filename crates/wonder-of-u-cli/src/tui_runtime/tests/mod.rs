use super::*;

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    thread,
};

use serde_json::{Value, json};
use wonder_of_u_agent::{
    AgentSettings, AuthMaterial, CredentialStore, SettingsStore, StoredCredentials,
};
use wonder_of_u_core::{
    AuthState, InputMode, MessageEnvelope, MessagePayload, PendingLocalToolCall,
    PendingProviderToolCall, PendingToolApprovalState, PendingToolConversationRound, TodoTaskEntry,
    TodoTaskList, TodoTaskStatus, TokenUsage,
};
use wonder_of_u_storage::TodoTaskStore;
use wonder_of_u_test_support::{EnvVarGuard, unique_test_dir};
use wonder_of_u_tui::KeyModifiers;

use crate::commands;

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(f)
}

include!("chunk_0.rs");
include!("chunk_1.rs");
include!("chunk_2.rs");
include!("chunk_3.rs");
include!("chunk_4.rs");
