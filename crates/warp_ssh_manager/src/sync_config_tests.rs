use std::path::PathBuf;

use super::*;
use crate::ssh_config_parser::SshConfigCandidate;
use crate::types::{ImportProvenance, SshAdvancedConfig, SshServerInfo};

fn imported_node(alias: &str) -> SshServerInfo {
    let mut info = SshServerInfo::new_default("node-1".into());
    info.host = alias.to_string();
    info.port = 22;
    info.username = "alice".into();
    info.advanced = SshAdvancedConfig {
        port_forwards: vec![],
        imported_from: Some(ImportProvenance {
            path: "/home/alice/.ssh/config".into(),
            alias: alias.to_string(),
        }),
    };
    info
}

fn candidate(alias: &str) -> SshConfigCandidate {
    SshConfigCandidate {
        alias: alias.into(),
        hostname: None,
        user: None,
        port: None,
        identity_file: None,
    }
}

#[test]
fn node_without_provenance_returns_none() {
    let info = SshServerInfo::new_default("manual".into());
    assert_eq!(compute_node_sync(&info, &[candidate("anything")]), None);
}

#[test]
fn missing_alias_reports_drift() {
    let info = imported_node("prodbox");
    let status = compute_node_sync(&info, &[candidate("other")]).expect("imported");
    assert_eq!(
        status,
        NodeSyncStatus::Drifted {
            alias: "prodbox".into()
        }
    );
}

#[test]
fn matching_config_with_no_overrides_is_up_to_date() {
    let info = imported_node("prodbox");
    // Candidate declares nothing extra (no port/user/identity) → no changes.
    let status = compute_node_sync(&info, &[candidate("prodbox")]).expect("imported");
    assert_eq!(status, NodeSyncStatus::UpToDate);
}

#[test]
fn changed_port_and_user_produce_field_changes() {
    let info = imported_node("prodbox");
    let cand = SshConfigCandidate {
        user: Some("bob".into()),
        port: Some(2222),
        ..candidate("prodbox")
    };
    let status = compute_node_sync(&info, &[cand]).expect("imported");
    match status {
        NodeSyncStatus::Updated { changes, new_info } => {
            assert_eq!(changes.len(), 2);
            assert!(changes.contains(&FieldChange {
                field: SyncField::Port,
                old: "22".into(),
                new: "2222".into(),
            }));
            assert!(changes.contains(&FieldChange {
                field: SyncField::User,
                old: "alice".into(),
                new: "bob".into(),
            }));
            assert_eq!(new_info.port, 2222);
            assert_eq!(new_info.username, "bob");
            // Provenance is preserved through the update.
            assert!(new_info.advanced.imported_from.is_some());
        }
        other => panic!("expected Updated, got {other:?}"),
    }
}

#[test]
fn omitted_user_does_not_blank_existing_username() {
    let info = imported_node("prodbox");
    // Config block only changes the port; the node keeps its username.
    let cand = SshConfigCandidate {
        port: Some(2200),
        ..candidate("prodbox")
    };
    let status = compute_node_sync(&info, &[cand]).expect("imported");
    match status {
        NodeSyncStatus::Updated { changes, new_info } => {
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].field, SyncField::Port);
            assert_eq!(new_info.username, "alice");
        }
        other => panic!("expected Updated, got {other:?}"),
    }
}

#[test]
fn new_identity_flips_password_auth_to_key() {
    let mut info = imported_node("prodbox");
    info.auth_type = AuthType::Password;
    info.key_path = None;
    let cand = SshConfigCandidate {
        identity_file: Some(PathBuf::from("/home/alice/.ssh/id_ed25519")),
        ..candidate("prodbox")
    };
    let status = compute_node_sync(&info, &[cand]).expect("imported");
    match status {
        NodeSyncStatus::Updated { changes, new_info } => {
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].field, SyncField::IdentityFile);
            assert_eq!(
                new_info.key_path.as_deref(),
                Some("/home/alice/.ssh/id_ed25519")
            );
            assert_eq!(new_info.auth_type, AuthType::Key);
        }
        other => panic!("expected Updated, got {other:?}"),
    }
}

#[test]
fn identity_change_keeps_onekey_auth() {
    let mut info = imported_node("prodbox");
    info.auth_type = AuthType::OneKey;
    info.key_path = Some("/old/key".into());
    let cand = SshConfigCandidate {
        identity_file: Some(PathBuf::from("/new/key")),
        ..candidate("prodbox")
    };
    let status = compute_node_sync(&info, &[cand]).expect("imported");
    match status {
        NodeSyncStatus::Updated { new_info, .. } => {
            assert_eq!(new_info.key_path.as_deref(), Some("/new/key"));
            assert_eq!(new_info.auth_type, AuthType::OneKey);
        }
        other => panic!("expected Updated, got {other:?}"),
    }
}
