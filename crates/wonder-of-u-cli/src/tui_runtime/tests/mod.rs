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

include!("chunk_0.rs");
include!("chunk_1.rs");
include!("chunk_2.rs");
include!("chunk_3.rs");
include!("chunk_4.rs");
