use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// Connection status — used only in the UI layer for display; not persisted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionStatus {
    Unknown,
    Online,
    Offline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    Folder,
    Server,
}

impl NodeKind {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            NodeKind::Folder => "folder",
            NodeKind::Server => "server",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "folder" => Some(NodeKind::Folder),
            "server" => Some(NodeKind::Server),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AuthType {
    Password,
    Key,
    OneKey,
}

impl AuthType {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            AuthType::Password => "password",
            AuthType::Key => "key",
            AuthType::OneKey => "onekey",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "password" => Some(AuthType::Password),
            "key" => Some(AuthType::Key),
            "onekey" => Some(AuthType::OneKey),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OneKeyCredentialKind {
    Password,
    Key,
}

impl OneKeyCredentialKind {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            OneKeyCredentialKind::Password => "password",
            OneKeyCredentialKind::Key => "key",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "password" => Some(OneKeyCredentialKind::Password),
            "key" => Some(OneKeyCredentialKind::Key),
            _ => None,
        }
    }
}

/// Tree node (folder or server), without server-only metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SshNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: NodeKind,
    pub name: String,
    pub sort_order: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    /// Meaningful only for folders; the UI uses this to decide whether to hide
    /// child nodes. Persisted in SQLite so the state survives restarts.
    pub is_collapsed: bool,
}

/// Direction/semantics of an SSH port forward.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortForwardKind {
    Local,
    Remote,
    Dynamic,
}

/// A single port forward. For `Dynamic` (SOCKS) `target_host`/`target_port` are
/// unused and should be `None`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PortForward {
    pub kind: PortForwardKind,
    pub bind_host: String,
    pub bind_port: u16,
    #[serde(default)]
    pub target_host: Option<String>,
    #[serde(default)]
    pub target_port: Option<u16>,
    #[serde(default)]
    pub description: Option<String>,
}

impl PortForward {
    /// Render the forward as the value of an `ssh -L/-R/-D` option (the spec part
    /// only, without the flag). Returns `None` if required fields are missing.
    pub fn to_ssh_spec(&self) -> Option<String> {
        match self.kind {
            PortForwardKind::Local | PortForwardKind::Remote => {
                let target_host = self.target_host.as_deref()?;
                let target_port = self.target_port?;
                Some(format!(
                    "{}:{}:{}:{}",
                    self.bind_host, self.bind_port, target_host, target_port
                ))
            }
            PortForwardKind::Dynamic => Some(format!("{}:{}", self.bind_host, self.bind_port)),
        }
    }

    /// The `ssh` flag (`-L`, `-R`, `-D`) for this forward's kind.
    pub fn ssh_flag(&self) -> &'static str {
        match self.kind {
            PortForwardKind::Local => "-L",
            PortForwardKind::Remote => "-R",
            PortForwardKind::Dynamic => "-D",
        }
    }
}

/// Records where a node was imported from in `~/.ssh/config`, so it can later be
/// re-synced one-way (config → node). `path` is the config file that was read at
/// import time; `alias` is the literal `Host` alias the node was created from
/// (it also equals the node's `host` field — see the importer).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportProvenance {
    pub path: String,
    pub alias: String,
}

/// Extensible per-server configuration persisted as a single JSON blob
/// (`ssh_servers.advanced_config`). New options (jump host, X11/agent forwarding,
/// ciphers, tab color, ...) are added here without a new migration per option.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshAdvancedConfig {
    #[serde(default)]
    pub port_forwards: Vec<PortForward>,
    /// Set when the node was imported from `~/.ssh/config`; `None` for manually
    /// created nodes. Enables the one-way "Sync with ~/.ssh/config" action.
    #[serde(default)]
    pub imported_from: Option<ImportProvenance>,
}

/// Connection configuration for a server node. `password` / `passphrase` are
/// not stored here — they go through the keychain.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshServerInfo {
    pub node_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub key_path: Option<String>,
    pub credential_id: Option<String>,
    pub startup_command: Option<String>,
    pub notes: Option<String>,
    pub last_connected_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub advanced: SshAdvancedConfig,
}

impl SshServerInfo {
    pub fn new_default(node_id: String) -> Self {
        Self {
            node_id,
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_type: AuthType::Password,
            key_path: None,
            credential_id: None,
            startup_command: None,
            notes: None,
            last_connected_at: None,
            advanced: SshAdvancedConfig::default(),
        }
    }

    /// Clone configuration from an existing server, assigning a new node_id.
    pub fn clone_from_template(source: &Self, new_node_id: String) -> Self {
        Self {
            node_id: new_node_id,
            host: source.host.clone(),
            port: source.port,
            username: source.username.clone(),
            auth_type: source.auth_type,
            key_path: source.key_path.clone(),
            credential_id: source.credential_id.clone(),
            startup_command: source.startup_command.clone(),
            notes: source.notes.clone(),
            last_connected_at: None,
            advanced: source.advanced.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SshOneKeyCredential {
    pub id: String,
    pub label: String,
    pub username: String,
    pub kind: OneKeyCredentialKind,
    pub key_path: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl SshOneKeyCredential {
    pub fn display_label(&self) -> String {
        if self.username.is_empty() {
            self.label.clone()
        } else {
            format!("{} ({})", self.label, self.username)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSshAuth {
    pub username: String,
    pub auth_type: AuthType,
    pub key_path: Option<String>,
    pub secret_lookup_id: String,
    pub secret_kind: crate::secrets::SecretKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forward(kind: PortForwardKind) -> PortForward {
        PortForward {
            kind,
            bind_host: "127.0.0.1".into(),
            bind_port: 8000,
            target_host: Some("example.com".into()),
            target_port: Some(80),
            description: None,
        }
    }

    #[test]
    fn local_and_remote_forward_specs_include_target() {
        assert_eq!(
            forward(PortForwardKind::Local).to_ssh_spec().as_deref(),
            Some("127.0.0.1:8000:example.com:80")
        );
        assert_eq!(
            forward(PortForwardKind::Remote).to_ssh_spec().as_deref(),
            Some("127.0.0.1:8000:example.com:80")
        );
    }

    #[test]
    fn dynamic_forward_spec_is_bind_only() {
        let mut f = forward(PortForwardKind::Dynamic);
        f.target_host = None;
        f.target_port = None;
        assert_eq!(f.to_ssh_spec().as_deref(), Some("127.0.0.1:8000"));
    }

    #[test]
    fn local_forward_without_target_yields_none() {
        let mut f = forward(PortForwardKind::Local);
        f.target_port = None;
        assert_eq!(f.to_ssh_spec(), None);
    }

    #[test]
    fn ssh_flag_matches_kind() {
        assert_eq!(forward(PortForwardKind::Local).ssh_flag(), "-L");
        assert_eq!(forward(PortForwardKind::Remote).ssh_flag(), "-R");
        assert_eq!(forward(PortForwardKind::Dynamic).ssh_flag(), "-D");
    }

    #[test]
    fn advanced_config_json_round_trip() {
        let cfg = SshAdvancedConfig {
            port_forwards: vec![forward(PortForwardKind::Local)],
            imported_from: None,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: SshAdvancedConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn advanced_config_default_deserializes_from_empty_object() {
        let back: SshAdvancedConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(back, SshAdvancedConfig::default());
    }

    #[test]
    fn advanced_config_without_imported_from_defaults_to_none() {
        // Legacy rows written before M5 only carry `port_forwards`; the new
        // `imported_from` field must default to None rather than fail to parse.
        let back: SshAdvancedConfig =
            serde_json::from_str(r#"{"port_forwards":[]}"#).unwrap();
        assert_eq!(back.imported_from, None);
    }

    #[test]
    fn advanced_config_with_provenance_round_trips() {
        let cfg = SshAdvancedConfig {
            port_forwards: vec![],
            imported_from: Some(ImportProvenance {
                path: "/home/me/.ssh/config".into(),
                alias: "prodbox".into(),
            }),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: SshAdvancedConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }
}
