//! Tests for the editable `~/.ssh/config` model. The fixture is an anonymized
//! reproduction of the user's real SCE-formatted config, covering every observed
//! construct: a root host with unrecognized directives, an `Include`, group
//! markers surrounded by blank lines, descriptions, lowercase repeated
//! `localforward`s, a `0.0.0.0:` bind, `ProxyJump`, a commented-out directive,
//! and a trailing `Match` block. Indentation is tabs, as in the real file.

use super::*;

/// Anonymized real-world config. `\t` = the tab indentation SCE writes.
const FIXTURE: &str = "Include ~/.ssh/work/*.conf

Host github.com
\tUser git
\tHostName github.com
\tIdentityFile ~/.ssh/id_rsa
\tIdentitiesOnly yes
\tIdentityAgent none

#SCE_GROUP:53B0A32B-AD88-4368-9EB3-9568BEDCDF52:::Dvuelta

# synchronizoho PRO - LAN
Host synchronizoho
\tUser svc.tasks.pro
\tHostName 192.168.10.220
\tPort 9022
\t#SCEGroup 53B0A32B-AD88-4368-9EB3-9568BEDCDF52
\t#SCETags Purple

#SCE_GROUP:CD353C24-8FC7-4E7E-8E87-596749F78757:::Personal

Host dos2locos.example
\tUser ubuntu
\tHostName monitor.example.test
\tPort 9922
\tlocalforward 9999 localhost:9000
\tlocalforward 8880 192.168.1.142:80
\t#SCEIcon home
\t#SCEGroup CD353C24-8FC7-4E7E-8E87-596749F78757

#SCE_GROUP:FD5FCA8E-DA89-4CD4-B0BC-F82BC0EC2D2E:::AWS

# Servidor Web
Host awsbastion
\tUser ec2-user
\tHostName 18.0.0.1
\tIdentityFile ~/.ssh/bastion.pem
\t#SCEGroup FD5FCA8E-DA89-4CD4-B0BC-F82BC0EC2D2E

# Servidor interno
# ProxyJump awsbastion
Host awsinternal
\tUser ec2-user
\tHostName 10.0.0.20
\tIdentityFile ~/.ssh/internal.pem
\tLocalForward 0.0.0.0:9201 localhost:9200
\tProxyJump awsbastion
\t#SCEGroup FD5FCA8E-DA89-4CD4-B0BC-F82BC0EC2D2E

Match host *.internal
\tForwardAgent yes
";

fn doc() -> SshConfigDocument {
    SshConfigDocument::parse(FIXTURE)
}

// ---- Round-trip -------------------------------------------------------------

#[test]
fn round_trip_is_byte_for_byte_identical() {
    assert_eq!(doc().to_config_string(), FIXTURE);
}

#[test]
fn round_trip_is_idempotent() {
    let once = doc().to_config_string();
    let twice = SshConfigDocument::parse(&once).to_config_string();
    assert_eq!(once, twice);
}

#[test]
fn empty_input_round_trips_to_empty() {
    assert_eq!(SshConfigDocument::parse("").to_config_string(), "");
}

#[test]
fn file_without_trailing_newline_is_preserved() {
    let input = "Host a\n\tUser x";
    assert_eq!(SshConfigDocument::parse(input).to_config_string(), input);
}

// ---- Decoding ---------------------------------------------------------------

#[test]
fn groups_are_decoded_in_order() {
    let groups = doc().groups();
    let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, vec!["Dvuelta", "Personal", "AWS"]);
    assert_eq!(groups[0].uuid, "53B0A32B-AD88-4368-9EB3-9568BEDCDF52");
}

#[test]
fn host_aliases_lists_every_host() {
    assert_eq!(
        doc().host_aliases(),
        vec![
            "github.com",
            "synchronizoho",
            "dos2locos.example",
            "awsbastion",
            "awsinternal"
        ]
    );
}

#[test]
fn host_view_decodes_scalar_fields_and_sce() {
    let v = doc().host_view("synchronizoho").expect("host present");
    assert_eq!(v.user.as_deref(), Some("svc.tasks.pro"));
    assert_eq!(v.hostname.as_deref(), Some("192.168.10.220"));
    assert_eq!(v.port, Some(9022));
    assert_eq!(
        v.group.as_deref(),
        Some("53B0A32B-AD88-4368-9EB3-9568BEDCDF52")
    );
    assert_eq!(v.color.as_deref(), Some("Purple"));
}

#[test]
fn host_view_preserves_repeated_lowercase_forwards_verbatim() {
    let v = doc().host_view("dos2locos.example").expect("host present");
    let specs: Vec<&str> = v.forwards.iter().map(|f| f.spec.as_str()).collect();
    assert_eq!(specs, vec!["9999 localhost:9000", "8880 192.168.1.142:80"]);
    assert!(v.forwards.iter().all(|f| f.kind == PortForwardKind::Local));
    assert_eq!(v.icon.as_deref(), Some("home"));
}

#[test]
fn host_view_decodes_proxy_jump_and_bind_forward() {
    let v = doc().host_view("awsinternal").expect("host present");
    assert_eq!(v.proxy_jump.as_deref(), Some("awsbastion"));
    assert_eq!(v.forwards[0].spec, "0.0.0.0:9201 localhost:9200");
    assert_eq!(v.identity_file.as_deref(), Some("~/.ssh/internal.pem"));
}

#[test]
fn host_view_description_is_the_comment_above_host() {
    let v = doc().host_view("awsbastion").expect("host present");
    assert_eq!(v.description.as_deref(), Some("Servidor Web"));
}

// ---- upsert: surgical, non-destructive --------------------------------------

#[test]
fn upsert_existing_host_keeps_sce_and_description() {
    let mut d = doc();
    d.upsert_host(
        "synchronizoho",
        &CoreHostFields {
            port: Some(9023),
            user: Some("svc.tasks.pro".into()),
            hostname: Some("192.168.10.220".into()),
            ..Default::default()
        },
    );
    let out = d.to_config_string();
    assert!(out.contains("\tPort 9023"), "port updated");
    assert!(!out.contains("\tPort 9022"), "old port gone");
    assert!(out.contains("\t#SCEGroup 53B0A32B-AD88-4368-9EB3-9568BEDCDF52"));
    assert!(out.contains("\t#SCETags Purple"));
    assert!(
        out.contains("# synchronizoho PRO - LAN"),
        "description kept"
    );
}

#[test]
fn upsert_does_not_touch_unrecognized_directives() {
    let mut d = doc();
    d.upsert_host(
        "github.com",
        &CoreHostFields {
            hostname: Some("github.com".into()),
            user: Some("git".into()),
            identity_file: Some("~/.ssh/id_rsa".into()),
            ..Default::default()
        },
    );
    let out = d.to_config_string();
    assert!(out.contains("\tIdentitiesOnly yes"));
    assert!(out.contains("\tIdentityAgent none"));
    // The Include and Match blocks are untouched too.
    assert!(out.contains("Include ~/.ssh/work/*.conf"));
    assert!(out.contains("Match host *.internal"));
}

#[test]
fn upsert_clears_field_when_none() {
    let mut d = doc();
    d.upsert_host(
        "awsbastion",
        &CoreHostFields {
            hostname: Some("18.0.0.1".into()),
            user: Some("ec2-user".into()),
            identity_file: None, // clear it
            ..Default::default()
        },
    );
    let v = d.host_view("awsbastion").unwrap();
    assert_eq!(v.identity_file, None);
    assert!(!d.to_config_string().contains("bastion.pem"));
}

#[test]
fn upsert_creates_new_host_with_detected_indent() {
    let mut d = doc();
    d.upsert_host(
        "newbox",
        &CoreHostFields {
            hostname: Some("new.example.test".into()),
            user: Some("deploy".into()),
            port: Some(2222),
            ..Default::default()
        },
    );
    let out = d.to_config_string();
    assert!(out.contains("Host newbox\n\tHostName new.example.test\n\tUser deploy\n\tPort 2222"));
    assert_eq!(d.host_view("newbox").unwrap().port, Some(2222));
}

#[test]
fn upsert_inserts_ssh_directives_before_sce_metadata() {
    // dos2locos has SCE lines at the end; a newly set ProxyJump must land before
    // them so metadata stays trailing.
    let mut d = doc();
    d.upsert_host(
        "dos2locos.example",
        &CoreHostFields {
            hostname: Some("monitor.example.test".into()),
            user: Some("ubuntu".into()),
            port: Some(9922),
            proxy_jump: Some("awsbastion".into()),
            forwards: vec![ForwardEntry {
                kind: PortForwardKind::Local,
                spec: "9999 localhost:9000".into(),
            }],
            ..Default::default()
        },
    );
    let out = d.to_config_string();
    let pj = out.find("ProxyJump awsbastion").unwrap();
    let icon = out.find("#SCEIcon home").unwrap();
    assert!(pj < icon, "ProxyJump must precede the SCE metadata");
}

// ---- remove / rename --------------------------------------------------------

#[test]
fn remove_host_drops_the_block() {
    let mut d = doc();
    assert!(d.remove_host("synchronizoho"));
    assert!(d.host_view("synchronizoho").is_none());
    assert!(!d.to_config_string().contains("Host synchronizoho"));
}

#[test]
fn remove_host_returns_false_when_absent() {
    assert!(!doc().remove_host("nope"));
}

#[test]
fn rename_host_rewrites_the_host_line() {
    let mut d = doc();
    assert!(d.rename_host("awsbastion", "aws-bastion-01"));
    let out = d.to_config_string();
    assert!(out.contains("Host aws-bastion-01"));
    assert!(!out.contains("Host awsbastion\n"));
    assert!(d.host_view("aws-bastion-01").is_some());
}

// ---- group membership -------------------------------------------------------

#[test]
fn set_group_moves_host_between_groups() {
    let mut d = doc();
    assert!(d.set_group(
        "synchronizoho",
        Some("CD353C24-8FC7-4E7E-8E87-596749F78757")
    ));
    let v = d.host_view("synchronizoho").unwrap();
    assert_eq!(
        v.group.as_deref(),
        Some("CD353C24-8FC7-4E7E-8E87-596749F78757")
    );
}

#[test]
fn set_group_none_makes_host_ungrouped() {
    let mut d = doc();
    assert!(d.set_group("synchronizoho", None));
    assert_eq!(d.host_view("synchronizoho").unwrap().group, None);
    assert!(!d.to_config_string().contains("\t#SCEGroup 53B0A32B"));
}

#[test]
fn set_icon_and_color_add_then_update() {
    let mut d = doc();
    assert!(d.set_icon("github.com", Some("server")));
    assert!(d.set_color("github.com", Some("Blue")));
    let v = d.host_view("github.com").unwrap();
    assert_eq!(v.icon.as_deref(), Some("server"));
    assert_eq!(v.color.as_deref(), Some("Blue"));

    assert!(d.set_color("github.com", Some("Red")));
    assert_eq!(
        d.host_view("github.com").unwrap().color.as_deref(),
        Some("Red")
    );
    // Only one tag line, not two.
    assert_eq!(d.to_config_string().matches("#SCETags").count(), 2); // synchronizoho + github
}

#[test]
fn set_description_replaces_or_adds() {
    let mut d = doc();
    assert!(d.set_description("awsbastion", Some("Bastión AWS")));
    assert_eq!(
        d.host_view("awsbastion").unwrap().description.as_deref(),
        Some("Bastión AWS")
    );
    // github.com had no description; adding one creates the comment line.
    assert!(d.set_description("github.com", Some("Repos")));
    assert!(d.to_config_string().contains("# Repos\nHost github.com"));
}

// ---- group CRUD -------------------------------------------------------------

#[test]
fn create_group_appends_uppercase_uuid_marker() {
    let mut d = doc();
    let uuid = d.create_group("Nuevos");
    assert_eq!(uuid, uuid.to_uppercase());
    assert_eq!(uuid.len(), 36);
    assert!(
        d.groups()
            .iter()
            .any(|g| g.name == "Nuevos" && g.uuid == uuid)
    );
    assert!(
        d.to_config_string()
            .contains(&format!("#SCE_GROUP:{uuid}:::Nuevos"))
    );
}

#[test]
fn rename_group_updates_marker() {
    let mut d = doc();
    assert!(d.rename_group("53B0A32B-AD88-4368-9EB3-9568BEDCDF52", "Dvuelta SL"));
    assert!(d.to_config_string().contains(":::Dvuelta SL"));
    assert!(!doc_renamed_still_has_old(&d));
}

fn doc_renamed_still_has_old(d: &SshConfigDocument) -> bool {
    d.to_config_string().contains(":::Dvuelta\n")
}

#[test]
fn delete_group_removes_marker_and_orphans_hosts() {
    let mut d = doc();
    assert!(d.delete_group("53B0A32B-AD88-4368-9EB3-9568BEDCDF52"));
    let out = d.to_config_string();
    assert!(!out.contains("#SCE_GROUP:53B0A32B"));
    assert!(!out.contains("#SCEGroup 53B0A32B"));
    // The host itself survives, just ungrouped.
    assert!(d.host_view("synchronizoho").is_some());
    assert_eq!(d.host_view("synchronizoho").unwrap().group, None);
}

// ---- IO: atomic save + backup + perms --------------------------------------

#[test]
fn save_then_reload_round_trips_through_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    std::fs::write(&path, FIXTURE).unwrap();

    let mut d = load_document_from(&path).unwrap();
    d.set_color("github.com", Some("Green"));
    save_document_atomic(&path, &d).unwrap();

    let reloaded = load_document_from(&path).unwrap();
    assert_eq!(
        reloaded.host_view("github.com").unwrap().color.as_deref(),
        Some("Green")
    );
    // A backup of the prior contents exists.
    let backup = std::fs::read_to_string(dir.path().join("config.bak")).unwrap();
    assert_eq!(backup, FIXTURE);
}

#[cfg(unix)]
#[test]
fn saved_file_is_owner_only_readable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    let d = SshConfigDocument::parse("Host a\n\tUser x\n");
    save_document_atomic(&path, &d).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn load_missing_file_yields_empty_document() {
    let dir = tempfile::tempdir().unwrap();
    let d = load_document_from(&dir.path().join("absent")).unwrap();
    assert!(d.host_aliases().is_empty());
}
