//! One-way `~/.ssh/config` → node synchronisation logic (M5 of the SSH editor
//! revamp; see `specs/ssh-editor-revamp/PLAN.md` §6.2).
//!
//! This module is a **pure function** layer with no IO: given a node's current
//! [`SshServerInfo`] (carrying [`ImportProvenance`] in its `advanced` blob) and
//! the candidates parsed from the config file, it computes what would change if
//! the node were re-synced — without touching the database. The panel uses the
//! result to render a confirmation diff before applying anything.
//!
//! Design decisions:
//! - **Only nodes with provenance are considered.** Manually created nodes
//!   (`imported_from == None`) are skipped entirely ([`compute_node_sync`]
//!   returns `None`).
//! - **Config → node only.** We never write back to `~/.ssh/config`.
//! - **Non-destructive.** A field is only proposed for update when the config
//!   block actually provides a value for it; a directive omitted from the config
//!   leaves the node's existing value untouched (e.g. no `User` line in the
//!   block does not blank out the node's username).
//! - **Drift is reported, not applied.** If the provenance alias is no longer
//!   present in the config, the node is flagged [`NodeSyncStatus::Drifted`] and
//!   left as-is — never silently deleted.

use crate::ssh_config_parser::SshConfigCandidate;
use crate::types::{AuthType, SshServerInfo};

/// Which node field a [`FieldChange`] refers to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncField {
    Port,
    User,
    IdentityFile,
}

impl SyncField {
    /// Stable lowercase label for display / i18n lookup.
    pub fn label(&self) -> &'static str {
        match self {
            SyncField::Port => "port",
            SyncField::User => "user",
            SyncField::IdentityFile => "identity",
        }
    }
}

/// A single proposed change to a node field. `old`/`new` are display strings;
/// an empty/absent value renders as an empty string at the call site.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldChange {
    pub field: SyncField,
    pub old: String,
    pub new: String,
}

/// Outcome of comparing one imported node against the current config.
///
/// `Updated` carries a full `SshServerInfo`, which dwarfs the other variants;
/// the value is short-lived (built, shown in the diff, then either persisted or
/// dropped), so boxing it would add churn without a real memory benefit.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NodeSyncStatus {
    /// The config still matches the node — nothing to do.
    UpToDate,
    /// The provenance alias is no longer present in the config. The node is
    /// left untouched; the UI surfaces this as a warning.
    Drifted { alias: String },
    /// The config provides values that differ from the node. `changes` is the
    /// per-field diff for display; `new_info` is the fully-updated node ready to
    /// persist on confirmation.
    Updated {
        changes: Vec<FieldChange>,
        new_info: SshServerInfo,
    },
}

/// Compute the sync outcome for a single node.
///
/// Returns `None` when the node was not imported from `~/.ssh/config` (no
/// provenance), so callers can skip it without special-casing.
pub fn compute_node_sync(
    info: &SshServerInfo,
    candidates: &[SshConfigCandidate],
) -> Option<NodeSyncStatus> {
    let provenance = info.advanced.imported_from.as_ref()?;
    let alias = provenance.alias.clone();

    let Some(candidate) = candidates.iter().find(|c| c.alias == alias) else {
        return Some(NodeSyncStatus::Drifted { alias });
    };

    let mut changes = Vec::new();
    let mut new_info = info.clone();

    // Port — only when the config block declares one (decision: a missing
    // `Port` keeps the node's current port rather than resetting to 22).
    if let Some(port) = candidate.port
        && port != info.port
    {
        changes.push(FieldChange {
            field: SyncField::Port,
            old: info.port.to_string(),
            new: port.to_string(),
        });
        new_info.port = port;
    }

    // User — only when the config block declares a non-empty `User`.
    if let Some(user) = candidate.user.as_ref().filter(|u| !u.is_empty())
        && *user != info.username
    {
        changes.push(FieldChange {
            field: SyncField::User,
            old: info.username.clone(),
            new: user.clone(),
        });
        new_info.username = user.clone();
    }

    // IdentityFile — only when the config block declares one. When it changes
    // and the node was password-auth, flip it to key auth (mirrors the import
    // mapping in `on_import_candidate`); a OneKey node is left as-is.
    if let Some(identity) = candidate.identity_file.as_ref() {
        let new_key = identity.to_string_lossy().into_owned();
        if Some(&new_key) != info.key_path.as_ref() {
            changes.push(FieldChange {
                field: SyncField::IdentityFile,
                old: info.key_path.clone().unwrap_or_default(),
                new: new_key.clone(),
            });
            new_info.key_path = Some(new_key);
            if info.auth_type == AuthType::Password {
                new_info.auth_type = AuthType::Key;
            }
        }
    }

    if changes.is_empty() {
        Some(NodeSyncStatus::UpToDate)
    } else {
        Some(NodeSyncStatus::Updated { changes, new_info })
    }
}

#[cfg(test)]
#[path = "sync_config_tests.rs"]
mod tests;
