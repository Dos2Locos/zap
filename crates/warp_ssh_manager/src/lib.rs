//! SSH manager data layer — persistent server/folder tree, OS keychain
//! credential storage, and command assembly. UI and PTY injection logic live in
//! `app/src/ssh_manager/` and the `secret_injector` module; this crate stays
//! pure Rust with no warpui dependency and can be tested with `cargo test`
//! standalone.

pub mod config_model;
pub mod db;
pub mod repository;
pub mod secrets;
pub mod ssh_command;
pub mod ssh_config_parser;
pub mod sync_config;
pub mod sync_provider;
pub mod types;

pub use config_model::{
    CoreHostFields, ForwardEntry, Group, HostView, OutlineEntry, SshConfigDocument,
    load_document_from, save_document_atomic,
};
pub use db::{set_database_path, with_conn};
pub use repository::{SshRepository, SshRepositoryError, SyncMetaRepository};
pub use secrets::{KeychainSecretStore, SecretKind, SshSecretStore, SshSecretStoreError};
pub use ssh_command::{
    ConnectionTestResult, build_ssh_alias_command_line, build_ssh_args, build_ssh_command_line,
    test_connection,
};
pub use ssh_config_parser::{
    LoadOutcome, LoadResult, SshConfigCandidate, default_ssh_config_path, load_candidates,
    load_candidates_from, parse_ssh_config,
};
pub use sync_config::{FieldChange, NodeSyncStatus, SyncField, compute_node_sync};
pub use sync_provider::{
    DbVersionStore, SshSyncData, SshSyncProvider, SyncNode, SyncOneKeyCredential, SyncServer,
};
pub use types::ConnectionStatus;
pub use types::{
    AuthType, ImportProvenance, NodeKind, OneKeyCredentialKind, PortForward, PortForwardKind,
    ResolvedSshAuth, SshAdvancedConfig, SshNode, SshOneKeyCredential, SshServerInfo,
};
