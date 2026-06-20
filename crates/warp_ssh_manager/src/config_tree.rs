//! Build the panel's display tree from a [`SshConfigDocument`].
//!
//! Phase 2 of the `~/.ssh/config`-as-single-source redesign: the server list is
//! grouped by `#SCE_GROUP` markers into collapsible folders, with each host
//! carrying its decoded icon/color. This module turns the parsed config document
//! into the same `Vec<SshNode>` shape the existing panel already renders, so the
//! UI layer only has to swap its data source — the grouping, ordering and
//! orphan-handling logic lives here where it can be unit-tested without GPUI.
//!
//! Layout rules:
//! - Groups become `Folder` nodes; hosts become `Server` nodes.
//! - A host whose `#SCEGroup` matches a known group is a child of that folder;
//!   a host with no group (or a dangling uuid) sits at the root.
//! - Top-level entries (folders and ungrouped hosts) keep their document order;
//!   a group's hosts are gathered under it in document order.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDateTime};

use crate::config_model::{HostView, OutlineEntry, SshConfigDocument};
use crate::types::{NodeKind, SshNode};

/// The display model derived from a config document.
pub struct SshConfigTree {
    /// Folder + server nodes in display order (folder immediately followed by
    /// its children). `parent_id` and `is_collapsed` are populated so the panel
    /// can reuse its existing visibility logic.
    pub nodes: Vec<SshNode>,
    /// Decoded host details keyed by alias (the server node's `id`), for row
    /// rendering (icon, color, hostname, …).
    pub hosts: HashMap<String, HostView>,
}

/// A fixed timestamp for synthetic nodes — the config file has no per-entry
/// timestamps, and the panel does not display them.
fn epoch() -> NaiveDateTime {
    DateTime::UNIX_EPOCH.naive_utc()
}

fn folder_node(uuid: &str, name: &str, sort_order: i32, collapsed: bool) -> SshNode {
    SshNode {
        id: uuid.to_string(),
        parent_id: None,
        kind: NodeKind::Folder,
        name: name.to_string(),
        sort_order,
        created_at: epoch(),
        updated_at: epoch(),
        is_collapsed: collapsed,
    }
}

fn server_node(alias: &str, parent: Option<&str>, sort_order: i32) -> SshNode {
    SshNode {
        id: alias.to_string(),
        parent_id: parent.map(|p| p.to_string()),
        kind: NodeKind::Server,
        name: alias.to_string(),
        sort_order,
        created_at: epoch(),
        updated_at: epoch(),
        is_collapsed: false,
    }
}

/// Build the grouped display tree. `collapsed` holds the uuids of folders the
/// user has collapsed (held in panel state, since the config has no collapse
/// flag).
pub fn build_config_tree(doc: &SshConfigDocument, collapsed: &HashSet<String>) -> SshConfigTree {
    let outline = doc.outline();

    // Known group uuids (so a dangling `#SCEGroup` reference falls back to root).
    let known_groups: HashSet<&str> = outline
        .iter()
        .filter_map(|e| match e {
            OutlineEntry::Group { uuid, .. } => Some(uuid.as_str()),
            _ => None,
        })
        .collect();

    // First pass: split into an ordered top-level sequence and per-group children.
    enum TopItem<'a> {
        Group { uuid: &'a str, name: &'a str },
        Host { alias: &'a str },
    }
    let mut top: Vec<TopItem> = Vec::new();
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for entry in &outline {
        match entry {
            OutlineEntry::Group { uuid, name } => {
                children.entry(uuid).or_default();
                top.push(TopItem::Group { uuid, name });
            }
            OutlineEntry::Host { alias, group } => match group.as_deref() {
                Some(g) if known_groups.contains(g) => children.entry(g).or_default().push(alias),
                _ => top.push(TopItem::Host { alias }),
            },
        }
    }

    // Second pass: emit nodes in display order.
    let mut nodes = Vec::new();
    let mut order: i32 = 0;
    for item in &top {
        match item {
            TopItem::Group { uuid, name } => {
                let is_collapsed = collapsed.contains(*uuid);
                nodes.push(folder_node(uuid, name, order, is_collapsed));
                order += 1;
                if let Some(members) = children.get(*uuid) {
                    for alias in members {
                        nodes.push(server_node(alias, Some(uuid), order));
                        order += 1;
                    }
                }
            }
            TopItem::Host { alias } => {
                nodes.push(server_node(alias, None, order));
                order += 1;
            }
        }
    }

    let hosts = nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Server)
        .filter_map(|n| doc.host_view(&n.id).map(|v| (n.id.clone(), v)))
        .collect();

    SshConfigTree { nodes, hosts }
}

#[cfg(test)]
#[path = "config_tree_tests.rs"]
mod tests;
