//! Tests for the grouped display-tree builder.

use super::*;

const FIXTURE: &str = "Host github.com
\tUser git
\tHostName github.com

#SCE_GROUP:53B0A32B-AD88-4368-9EB3-9568BEDCDF52:::Dvuelta

Host synchronizoho
\tUser svc
\tHostName 192.168.10.220
\t#SCEGroup 53B0A32B-AD88-4368-9EB3-9568BEDCDF52
\t#SCETags Purple

Host synchronizoho-wan
\tUser svc
\tHostName 188.86.24.163
\t#SCEGroup 53B0A32B-AD88-4368-9EB3-9568BEDCDF52

#SCE_GROUP:CD353C24-8FC7-4E7E-8E87-596749F78757:::Personal

Host raspi
\tUser ubuntu
\tHostName 192.168.0.14
\t#SCEIcon ubuntu
\t#SCEGroup CD353C24-8FC7-4E7E-8E87-596749F78757
";

fn tree(collapsed: &[&str]) -> SshConfigTree {
    let doc = SshConfigDocument::parse(FIXTURE);
    let set: HashSet<String> = collapsed.iter().map(|s| s.to_string()).collect();
    build_config_tree(&doc, &set)
}

fn ids(t: &SshConfigTree) -> Vec<&str> {
    t.nodes.iter().map(|n| n.id.as_str()).collect()
}

#[test]
fn ungrouped_host_sits_at_root_in_document_order() {
    let t = tree(&[]);
    // github.com is ungrouped and first in the file → first node, no parent.
    assert_eq!(t.nodes[0].id, "github.com");
    assert_eq!(t.nodes[0].kind, NodeKind::Server);
    assert_eq!(t.nodes[0].parent_id, None);
}

#[test]
fn groups_become_folders_with_their_hosts_as_children() {
    let t = tree(&[]);
    assert_eq!(
        ids(&t),
        vec![
            "github.com",
            "53B0A32B-AD88-4368-9EB3-9568BEDCDF52",
            "synchronizoho",
            "synchronizoho-wan",
            "CD353C24-8FC7-4E7E-8E87-596749F78757",
            "raspi",
        ]
    );
    let dvuelta = &t.nodes[1];
    assert_eq!(dvuelta.kind, NodeKind::Folder);
    assert_eq!(dvuelta.name, "Dvuelta");
    // Members are parented to the folder.
    assert_eq!(
        t.nodes[2].parent_id.as_deref(),
        Some("53B0A32B-AD88-4368-9EB3-9568BEDCDF52")
    );
    assert_eq!(
        t.nodes[3].parent_id.as_deref(),
        Some("53B0A32B-AD88-4368-9EB3-9568BEDCDF52")
    );
}

#[test]
fn host_meta_carries_icon_and_color() {
    let t = tree(&[]);
    assert_eq!(t.hosts["synchronizoho"].color.as_deref(), Some("Purple"));
    assert_eq!(t.hosts["raspi"].icon.as_deref(), Some("ubuntu"));
    assert_eq!(
        t.hosts["github.com"].hostname.as_deref(),
        Some("github.com")
    );
}

#[test]
fn collapsed_set_marks_folder_collapsed() {
    let t = tree(&["53B0A32B-AD88-4368-9EB3-9568BEDCDF52"]);
    let dvuelta = t
        .nodes
        .iter()
        .find(|n| n.id == "53B0A32B-AD88-4368-9EB3-9568BEDCDF52")
        .unwrap();
    assert!(dvuelta.is_collapsed);
    let personal = t
        .nodes
        .iter()
        .find(|n| n.id == "CD353C24-8FC7-4E7E-8E87-596749F78757")
        .unwrap();
    assert!(!personal.is_collapsed);
}

#[test]
fn dangling_group_reference_falls_back_to_root() {
    let doc = SshConfigDocument::parse(
        "Host orphan\n\tUser x\n\t#SCEGroup 00000000-0000-0000-0000-000000000000\n",
    );
    let t = build_config_tree(&doc, &HashSet::new());
    assert_eq!(t.nodes.len(), 1);
    assert_eq!(t.nodes[0].id, "orphan");
    assert_eq!(t.nodes[0].parent_id, None);
}

#[test]
fn empty_document_yields_no_nodes() {
    let doc = SshConfigDocument::parse("");
    let t = build_config_tree(&doc, &HashSet::new());
    assert!(t.nodes.is_empty());
    assert!(t.hosts.is_empty());
}

#[test]
fn empty_group_has_no_children() {
    let doc = SshConfigDocument::parse("#SCE_GROUP:AAAA:::Empty\n");
    let t = build_config_tree(&doc, &HashSet::new());
    assert_eq!(t.nodes.len(), 1);
    assert_eq!(t.nodes[0].kind, NodeKind::Folder);
    assert_eq!(t.nodes[0].name, "Empty");
}
