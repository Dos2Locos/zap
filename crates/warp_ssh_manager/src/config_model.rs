//! Editable, **lossless** model of `~/.ssh/config` with SSH Config Editor (SCE)
//! metadata support.
//!
//! Unlike [`crate::ssh_config_parser`] — which extracts a lossy 5-field
//! candidate list for one-shot import — this module parses the file into a
//! document that round-trips byte-for-byte (comments, blank lines, indentation,
//! directive casing, unrecognized directives and `Match`/`Include` blocks are
//! all preserved) **and** exposes an editing API so Zap can act as the single
//! source of truth over the config file.
//!
//! ## SCE metadata (interoperable with the SSH Config Editor app)
//!
//! Organization that OpenSSH itself ignores is stored as `#SCE…` comments,
//! matching the on-disk format the user's SSH Config Editor already writes:
//!
//! | Metadata        | Format                          | Where                         |
//! |-----------------|---------------------------------|-------------------------------|
//! | Group (folder)  | `#SCE_GROUP:<UUID>:::<Name>`     | loose line, before its hosts  |
//! | Group membership| `#SCEGroup <UUID>`              | inside the `Host` block        |
//! | Icon            | `#SCEIcon <name>`               | inside the `Host` block        |
//! | Color / tag     | `#SCETags <Color>`             | inside the `Host` block        |
//! | Description     | `# <text>`                     | comment line above the `Host`  |
//!
//! ## Round-trip strategy
//!
//! Every preserved line keeps its **verbatim source text** (`raw`); serialization
//! emits that text unchanged. Editing operations rebuild only the affected line's
//! `raw` from the structured fields, so untouched content (including directive
//! casing such as `localforward` vs `LocalForward`) survives intact.

use std::path::Path;

use crate::types::PortForwardKind;

/// Indentation used for body lines of newly created hosts when none can be
/// detected from the existing document.
const DEFAULT_INDENT: &str = "    ";

// ---------------------------------------------------------------------------
// Public value types
// ---------------------------------------------------------------------------

/// A group (folder) declared by a `#SCE_GROUP` marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub uuid: String,
    pub name: String,
}

/// A single port-forward directive as stored in the config: the keyword's
/// direction plus the verbatim spec value (e.g. `"9999 localhost:9000"` for a
/// `LocalForward`, or `"1080"` for a `DynamicForward`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardEntry {
    pub kind: PortForwardKind,
    pub spec: String,
}

/// The SSH directives Zap manages for a host. Used by [`SshConfigDocument::upsert_host`].
///
/// Each scalar is `Some` to set/replace the directive and `None` to remove it.
/// `forwards` fully replaces the host's managed forward directives
/// (`LocalForward` / `RemoteForward` / `DynamicForward`); an empty vec removes
/// them. SCE metadata (group/icon/color/description) is intentionally **not**
/// part of this struct — it is managed by the dedicated setters so that an
/// `upsert_host` call never clobbers a host's organization.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoreHostFields {
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
    pub forwards: Vec<ForwardEntry>,
}

/// A top-level entry in document order, used to build the grouped tree view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutlineEntry {
    /// A `#SCE_GROUP` marker (folder).
    Group { uuid: String, name: String },
    /// A host and its declared group membership (`None` = ungrouped / root).
    Host {
        alias: String,
        group: Option<String>,
    },
}

/// A read-only snapshot of a host block, decoded for the UI / tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostView {
    /// The literal `Host` patterns (wildcards included), in order.
    pub patterns: Vec<String>,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
    pub forwards: Vec<ForwardEntry>,
    pub group: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub description: Option<String>,
}

impl HostView {
    /// Build the [`SshServerInfo`] used to open a connection (terminal or SFTP)
    /// for this host. `alias` is the `Host` pattern and the keychain lookup id.
    /// When `HostName` is omitted, the alias itself becomes the destination so
    /// OpenSSH still resolves it. Auth is `Key` when an `IdentityFile` is present,
    /// else `Password`; the actual secret lives in the keychain (by alias).
    pub fn to_server_info(&self, alias: &str) -> crate::types::SshServerInfo {
        use crate::types::{AuthType, PortForward, SshAdvancedConfig, SshServerInfo};
        let key_path = self.identity_file.clone().filter(|p| !p.is_empty());
        let auth_type = if key_path.is_some() {
            AuthType::Key
        } else {
            AuthType::Password
        };
        let port_forwards = self
            .forwards
            .iter()
            .filter_map(|f| PortForward::from_config_spec(f.kind, &f.spec))
            .collect::<Vec<_>>();
        SshServerInfo {
            node_id: alias.to_string(),
            host: self.hostname.clone().unwrap_or_else(|| alias.to_string()),
            port: self.port.unwrap_or(22),
            username: self.user.clone().unwrap_or_default(),
            auth_type,
            key_path,
            credential_id: None,
            startup_command: None,
            notes: self.description.clone(),
            last_connected_at: None,
            advanced: SshAdvancedConfig {
                port_forwards,
                imported_from: None,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Internal document representation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Item {
    /// A blank line.
    Blank,
    /// A loose comment line outside any host (verbatim, includes leading `#`).
    Comment(String),
    /// A `#SCE_GROUP:<uuid>:::<name>` marker line.
    GroupMarker {
        uuid: String,
        name: String,
    },
    /// A top-level directive outside any `Host`/`Match` (e.g. `Include`, global
    /// options). Stored verbatim.
    Global(String),
    Host(HostBlock),
    /// A `Match` block, preserved verbatim — Zap never edits these.
    Match(MatchBlock),
}

#[derive(Clone, Debug)]
struct HostBlock {
    /// Contiguous comment lines immediately above the `Host` line (verbatim).
    /// The last one is conventionally the human description.
    leading_comments: Vec<String>,
    /// Verbatim `Host` line; rebuilt only on rename / pattern change.
    raw_host_line: String,
    /// Leading whitespace before the `Host` keyword (usually empty).
    host_indent: String,
    /// Literal patterns after `Host` (inline comment stripped).
    patterns: Vec<String>,
    /// Indentation applied to body lines.
    indent: String,
    body: Vec<BodyLine>,
}

#[derive(Clone, Debug)]
struct MatchBlock {
    leading_comments: Vec<String>,
    /// The `Match` line plus all its body lines, verbatim.
    lines: Vec<String>,
}

#[derive(Clone, Debug)]
enum BodyLine {
    Directive(Directive),
    Sce(SceLine),
    /// A blank line or plain comment inside a host body, preserved verbatim.
    Raw(String),
}

#[derive(Clone, Debug)]
struct Directive {
    raw: String,
    keyword: String,
    value: String,
}

#[derive(Clone, Debug)]
struct SceLine {
    raw: String,
    kind: SceKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SceKind {
    Group(String),
    Icon(String),
    Tags(String),
}

// ---------------------------------------------------------------------------
// The document
// ---------------------------------------------------------------------------

/// A parsed, editable `~/.ssh/config`.
#[derive(Clone, Debug)]
pub struct SshConfigDocument {
    items: Vec<Item>,
    /// Indentation used for body lines of newly created hosts.
    default_indent: String,
    /// Whether the source ended with a trailing newline (preserved on serialize).
    trailing_newline: bool,
}

impl SshConfigDocument {
    /// Parse a config file body into an editable document. Pure — no IO.
    pub fn parse(content: &str) -> Self {
        Parser::new().run(content)
    }

    /// Serialize the document back to its textual form.
    pub fn to_config_string(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for item in &self.items {
            match item {
                Item::Blank => lines.push(String::new()),
                Item::Comment(raw) => lines.push(raw.clone()),
                Item::GroupMarker { uuid, name } => {
                    lines.push(format!("#SCE_GROUP:{uuid}:::{name}"));
                }
                Item::Global(raw) => lines.push(raw.clone()),
                Item::Host(h) => {
                    lines.extend(h.leading_comments.iter().cloned());
                    lines.push(h.raw_host_line.clone());
                    for b in &h.body {
                        lines.push(b.render());
                    }
                }
                Item::Match(m) => {
                    lines.extend(m.leading_comments.iter().cloned());
                    lines.extend(m.lines.iter().cloned());
                }
            }
        }
        let mut out = lines.join("\n");
        if self.trailing_newline {
            out.push('\n');
        }
        out
    }

    // ---- Groups --------------------------------------------------------

    /// All declared groups, in document order.
    pub fn groups(&self) -> Vec<Group> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::GroupMarker { uuid, name } => Some(Group {
                    uuid: uuid.clone(),
                    name: name.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    /// Create a new group with a freshly generated SCE-style UUID (uppercase v4),
    /// appending its marker (surrounded by blank lines) to the document.
    /// Returns the new group's UUID.
    pub fn create_group(&mut self, name: impl Into<String>) -> String {
        let uuid = uuid::Uuid::new_v4().to_string().to_uppercase();
        if !matches!(self.items.last(), None | Some(Item::Blank)) {
            self.items.push(Item::Blank);
        }
        self.items.push(Item::GroupMarker {
            uuid: uuid.clone(),
            name: name.into(),
        });
        self.items.push(Item::Blank);
        uuid
    }

    /// Rename an existing group. Returns `false` if no group with `uuid` exists.
    pub fn rename_group(&mut self, uuid: &str, new_name: impl Into<String>) -> bool {
        for item in &mut self.items {
            if let Item::GroupMarker { uuid: u, name } = item
                && u == uuid
            {
                *name = new_name.into();
                return true;
            }
        }
        false
    }

    /// Delete a group: remove its marker and strip the `#SCEGroup <uuid>`
    /// membership line from every host (orphaned hosts move to the root).
    /// Returns `false` if no group with `uuid` existed.
    pub fn delete_group(&mut self, uuid: &str) -> bool {
        let before = self.items.len();
        self.items
            .retain(|i| !matches!(i, Item::GroupMarker { uuid: u, .. } if u == uuid));
        let removed_marker = self.items.len() != before;

        for item in &mut self.items {
            if let Item::Host(h) = item {
                h.body.retain(|b| {
                    !matches!(b, BodyLine::Sce(SceLine { kind: SceKind::Group(g), .. }) if g == uuid)
                });
            }
        }
        removed_marker
    }

    // ---- Hosts ---------------------------------------------------------

    /// All host aliases (the first pattern of each host block), in order.
    pub fn host_aliases(&self) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::Host(h) => h.patterns.first().cloned(),
                _ => None,
            })
            .collect()
    }

    /// Decode a single host block by alias (exact pattern match).
    pub fn host_view(&self, alias: &str) -> Option<HostView> {
        self.find_host(alias).map(HostBlock::to_view)
    }

    /// The top-level entries (group markers and hosts) in document order. The
    /// alias of each host is its first pattern; hosts with no usable pattern are
    /// skipped.
    pub fn outline(&self) -> Vec<OutlineEntry> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::GroupMarker { uuid, name } => Some(OutlineEntry::Group {
                    uuid: uuid.clone(),
                    name: name.clone(),
                }),
                Item::Host(h) => {
                    let alias = h.patterns.first()?.clone();
                    let group = h.body.iter().find_map(|b| match b {
                        BodyLine::Sce(SceLine {
                            kind: SceKind::Group(g),
                            ..
                        }) => Some(g.clone()),
                        _ => None,
                    });
                    Some(OutlineEntry::Host { alias, group })
                }
                _ => None,
            })
            .collect()
    }

    /// Create the host if absent, otherwise update its managed SSH directives in
    /// place. SCE metadata and unrecognized directives are left untouched.
    pub fn upsert_host(&mut self, alias: &str, fields: &CoreHostFields) {
        if self.find_host(alias).is_none() {
            let indent = self.default_indent.clone();
            if !matches!(self.items.last(), None | Some(Item::Blank)) {
                self.items.push(Item::Blank);
            }
            self.items.push(Item::Host(HostBlock {
                leading_comments: Vec::new(),
                raw_host_line: format!("Host {alias}"),
                host_indent: String::new(),
                patterns: vec![alias.to_string()],
                indent,
                body: Vec::new(),
            }));
        }
        let host = self
            .find_host_mut(alias)
            .expect("host present after insert");
        host.set_scalar("HostName", fields.hostname.as_deref());
        host.set_scalar("User", fields.user.as_deref());
        host.set_scalar("Port", fields.port.map(|p| p.to_string()).as_deref());
        host.set_scalar("IdentityFile", fields.identity_file.as_deref());
        host.set_scalar("ProxyJump", fields.proxy_jump.as_deref());
        host.set_forwards(&fields.forwards);
    }

    /// Remove a host block entirely. Returns `false` if not found.
    pub fn remove_host(&mut self, alias: &str) -> bool {
        let before = self.items.len();
        self.items
            .retain(|i| !matches!(i, Item::Host(h) if h.patterns.iter().any(|p| p == alias)));
        self.items.len() != before
    }

    /// Rename a host alias (replaces the matching pattern token). Returns `false`
    /// if not found.
    pub fn rename_host(&mut self, old: &str, new: &str) -> bool {
        if let Some(host) = self.find_host_mut(old) {
            for p in &mut host.patterns {
                if p == old {
                    *p = new.to_string();
                }
            }
            host.rebuild_host_line();
            true
        } else {
            false
        }
    }

    /// Set or clear a host's group membership (`#SCEGroup`). Returns `false` if
    /// the host is not found.
    pub fn set_group(&mut self, alias: &str, uuid: Option<&str>) -> bool {
        self.set_host_sce(alias, SceKindTag::Group, uuid)
    }

    /// Set or clear a host's icon (`#SCEIcon`). Returns `false` if not found.
    pub fn set_icon(&mut self, alias: &str, icon: Option<&str>) -> bool {
        self.set_host_sce(alias, SceKindTag::Icon, icon)
    }

    /// Set or clear a host's color tag (`#SCETags`). Returns `false` if not found.
    pub fn set_color(&mut self, alias: &str, color: Option<&str>) -> bool {
        self.set_host_sce(alias, SceKindTag::Tags, color)
    }

    /// Set or clear a host's description (the comment line directly above `Host`).
    /// Returns `false` if the host is not found.
    pub fn set_description(&mut self, alias: &str, text: Option<&str>) -> bool {
        let Some(host) = self.find_host_mut(alias) else {
            return false;
        };
        host.set_description(text);
        true
    }

    // ---- internal helpers ---------------------------------------------

    fn set_host_sce(&mut self, alias: &str, tag: SceKindTag, value: Option<&str>) -> bool {
        let indent = self.default_indent.clone();
        let Some(host) = self.find_host_mut(alias) else {
            return false;
        };
        host.set_sce(tag, value, &indent);
        true
    }

    fn find_host(&self, alias: &str) -> Option<&HostBlock> {
        self.items.iter().find_map(|i| match i {
            Item::Host(h) if h.patterns.iter().any(|p| p == alias) => Some(h),
            _ => None,
        })
    }

    fn find_host_mut(&mut self, alias: &str) -> Option<&mut HostBlock> {
        self.items.iter_mut().find_map(|i| match i {
            Item::Host(h) if h.patterns.iter().any(|p| p == alias) => Some(h),
            _ => None,
        })
    }
}

/// Tag selecting which SCE body directive a setter operates on.
#[derive(Clone, Copy)]
enum SceKindTag {
    Group,
    Icon,
    Tags,
}

// ---------------------------------------------------------------------------
// BodyLine / HostBlock rendering & editing
// ---------------------------------------------------------------------------

impl BodyLine {
    fn render(&self) -> String {
        match self {
            BodyLine::Directive(d) => d.raw.clone(),
            BodyLine::Sce(s) => s.raw.clone(),
            BodyLine::Raw(s) => s.clone(),
        }
    }
}

fn is_forward_keyword(kw: &str) -> bool {
    kw.eq_ignore_ascii_case("LocalForward")
        || kw.eq_ignore_ascii_case("RemoteForward")
        || kw.eq_ignore_ascii_case("DynamicForward")
}

fn forward_keyword(kind: PortForwardKind) -> &'static str {
    match kind {
        PortForwardKind::Local => "LocalForward",
        PortForwardKind::Remote => "RemoteForward",
        PortForwardKind::Dynamic => "DynamicForward",
    }
}

fn forward_kind_from_keyword(kw: &str) -> Option<PortForwardKind> {
    if kw.eq_ignore_ascii_case("LocalForward") {
        Some(PortForwardKind::Local)
    } else if kw.eq_ignore_ascii_case("RemoteForward") {
        Some(PortForwardKind::Remote)
    } else if kw.eq_ignore_ascii_case("DynamicForward") {
        Some(PortForwardKind::Dynamic)
    } else {
        None
    }
}

impl HostBlock {
    fn to_view(&self) -> HostView {
        let mut view = HostView {
            patterns: self.patterns.clone(),
            hostname: None,
            user: None,
            port: None,
            identity_file: None,
            proxy_jump: None,
            forwards: Vec::new(),
            group: None,
            icon: None,
            color: None,
            description: None,
        };
        for b in &self.body {
            match b {
                BodyLine::Directive(d) => {
                    let kw = &d.keyword;
                    let v = d.value.clone();
                    if kw.eq_ignore_ascii_case("HostName") && view.hostname.is_none() {
                        view.hostname = Some(v);
                    } else if kw.eq_ignore_ascii_case("User") && view.user.is_none() {
                        view.user = Some(v);
                    } else if kw.eq_ignore_ascii_case("Port") && view.port.is_none() {
                        view.port = v.parse::<u16>().ok();
                    } else if kw.eq_ignore_ascii_case("IdentityFile")
                        && view.identity_file.is_none()
                    {
                        view.identity_file = Some(v);
                    } else if kw.eq_ignore_ascii_case("ProxyJump") && view.proxy_jump.is_none() {
                        view.proxy_jump = Some(v);
                    } else if let Some(kind) = forward_kind_from_keyword(kw) {
                        view.forwards.push(ForwardEntry { kind, spec: v });
                    }
                }
                BodyLine::Sce(s) => match &s.kind {
                    SceKind::Group(g) if view.group.is_none() => view.group = Some(g.clone()),
                    SceKind::Icon(i) if view.icon.is_none() => view.icon = Some(i.clone()),
                    SceKind::Tags(c) if view.color.is_none() => view.color = Some(c.clone()),
                    _ => {}
                },
                BodyLine::Raw(_) => {}
            }
        }
        view.description = self
            .leading_comments
            .last()
            .map(|c| c.trim_start().trim_start_matches('#').trim().to_string());
        view
    }

    fn rebuild_host_line(&mut self) {
        self.raw_host_line = format!("{}Host {}", self.host_indent, self.patterns.join(" "));
    }

    /// Index where a new SSH directive should be inserted: just before the first
    /// SCE line, so metadata stays at the end of the block.
    fn ssh_insert_index(&self) -> usize {
        self.body
            .iter()
            .position(|b| matches!(b, BodyLine::Sce(_)))
            .unwrap_or(self.body.len())
    }

    /// Set, replace, or remove a single-valued SSH directive (first occurrence
    /// wins; duplicates are removed when a value is set).
    fn set_scalar(&mut self, keyword: &str, value: Option<&str>) {
        let positions: Vec<usize> = self
            .body
            .iter()
            .enumerate()
            .filter_map(|(i, b)| match b {
                BodyLine::Directive(d) if d.keyword.eq_ignore_ascii_case(keyword) => Some(i),
                _ => None,
            })
            .collect();

        match value {
            None => {
                for &i in positions.iter().rev() {
                    self.body.remove(i);
                }
            }
            Some(v) => {
                if let Some(&first) = positions.first() {
                    if let BodyLine::Directive(d) = &mut self.body[first] {
                        d.value = v.to_string();
                        d.raw = format!("{}{} {}", self.indent, d.keyword, v);
                    }
                    for &i in positions.iter().skip(1).rev() {
                        self.body.remove(i);
                    }
                } else {
                    let idx = self.ssh_insert_index();
                    self.body.insert(
                        idx,
                        BodyLine::Directive(Directive {
                            raw: format!("{}{} {}", self.indent, keyword, v),
                            keyword: keyword.to_string(),
                            value: v.to_string(),
                        }),
                    );
                }
            }
        }
    }

    /// Replace all managed forward directives with `forwards`.
    fn set_forwards(&mut self, forwards: &[ForwardEntry]) {
        self.body
            .retain(|b| !matches!(b, BodyLine::Directive(d) if is_forward_keyword(&d.keyword)));
        let idx = self.ssh_insert_index();
        for (offset, f) in forwards.iter().enumerate() {
            let keyword = forward_keyword(f.kind);
            self.body.insert(
                idx + offset,
                BodyLine::Directive(Directive {
                    raw: format!("{}{} {}", self.indent, keyword, f.spec),
                    keyword: keyword.to_string(),
                    value: f.spec.clone(),
                }),
            );
        }
    }

    fn set_sce(&mut self, tag: SceKindTag, value: Option<&str>, _doc_indent: &str) {
        let matches_tag = |k: &SceKind| {
            matches!(
                (tag, k),
                (SceKindTag::Group, SceKind::Group(_))
                    | (SceKindTag::Icon, SceKind::Icon(_))
                    | (SceKindTag::Tags, SceKind::Tags(_))
            )
        };
        let pos = self
            .body
            .iter()
            .position(|b| matches!(b, BodyLine::Sce(s) if matches_tag(&s.kind)));

        match value {
            None => {
                if let Some(i) = pos {
                    self.body.remove(i);
                }
            }
            Some(v) => {
                let (keyword, kind) = match tag {
                    SceKindTag::Group => ("#SCEGroup", SceKind::Group(v.to_string())),
                    SceKindTag::Icon => ("#SCEIcon", SceKind::Icon(v.to_string())),
                    SceKindTag::Tags => ("#SCETags", SceKind::Tags(v.to_string())),
                };
                let raw = format!("{}{} {}", self.indent, keyword, v);
                match pos {
                    Some(i) => {
                        if let BodyLine::Sce(s) = &mut self.body[i] {
                            s.kind = kind;
                            s.raw = raw;
                        }
                    }
                    None => self.body.push(BodyLine::Sce(SceLine { raw, kind })),
                }
            }
        }
    }

    fn set_description(&mut self, text: Option<&str>) {
        match text {
            None => {
                self.leading_comments.pop();
            }
            Some(t) => {
                let line = format!("# {t}");
                if self.leading_comments.is_empty() {
                    self.leading_comments.push(line);
                } else {
                    let last = self.leading_comments.len() - 1;
                    self.leading_comments[last] = line;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    items: Vec<Item>,
    pending_comments: Vec<String>,
    current: Current,
    default_indent: Option<String>,
}

enum Current {
    None,
    Host(HostBlock),
    Match(MatchBlock),
}

impl Parser {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            pending_comments: Vec::new(),
            current: Current::None,
            default_indent: None,
        }
    }

    fn run(mut self, content: &str) -> SshConfigDocument {
        let trailing_newline = content.ends_with('\n');
        for line in content.lines() {
            self.feed(line);
        }
        self.finish_current();
        self.flush_pending();
        SshConfigDocument {
            items: self.items,
            default_indent: self
                .default_indent
                .unwrap_or_else(|| DEFAULT_INDENT.to_string()),
            trailing_newline,
        }
    }

    fn feed(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            self.finish_current();
            self.flush_pending();
            self.items.push(Item::Blank);
            return;
        }

        if trimmed.starts_with('#') {
            self.feed_comment(line, trimmed);
            return;
        }

        let (keyword, rest) = split_first_word(trimmed);
        if keyword.eq_ignore_ascii_case("Host") {
            self.finish_current();
            let leading = std::mem::take(&mut self.pending_comments);
            let host_indent = leading_whitespace(line).to_string();
            let patterns = parse_patterns(rest);
            self.current = Current::Host(HostBlock {
                leading_comments: leading,
                raw_host_line: line.to_string(),
                host_indent,
                patterns,
                indent: self
                    .default_indent
                    .clone()
                    .unwrap_or_else(|| DEFAULT_INDENT.to_string()),
                body: Vec::new(),
            });
            return;
        }

        if keyword.eq_ignore_ascii_case("Match") {
            self.finish_current();
            let leading = std::mem::take(&mut self.pending_comments);
            self.current = Current::Match(MatchBlock {
                leading_comments: leading,
                lines: vec![line.to_string()],
            });
            return;
        }

        // A directive line.
        match &mut self.current {
            Current::Host(h) => {
                self.default_indent
                    .get_or_insert_with(|| leading_whitespace(line).to_string());
                if h.body.is_empty() {
                    h.indent = leading_whitespace(line).to_string();
                }
                h.body.push(BodyLine::Directive(Directive {
                    raw: line.to_string(),
                    keyword: keyword.to_string(),
                    value: rest.to_string(),
                }));
            }
            Current::Match(m) => m.lines.push(line.to_string()),
            Current::None => {
                self.flush_pending();
                self.items.push(Item::Global(line.to_string()));
            }
        }
    }

    fn feed_comment(&mut self, line: &str, trimmed: &str) {
        // Group marker: `#SCE_GROUP:<uuid>:::<name>`.
        if let Some(rest) = trimmed.strip_prefix("#SCE_GROUP:")
            && let Some((uuid, name)) = rest.split_once(":::")
        {
            self.finish_current();
            self.flush_pending();
            self.items.push(Item::GroupMarker {
                uuid: uuid.to_string(),
                name: name.to_string(),
            });
            return;
        }

        // SCE body metadata inside the current host.
        if let Current::Host(h) = &mut self.current
            && let Some(kind) = parse_sce_body(trimmed)
        {
            if h.body.is_empty() {
                h.indent = leading_whitespace(line).to_string();
            }
            h.body.push(BodyLine::Sce(SceLine {
                raw: line.to_string(),
                kind,
            }));
            return;
        }

        match &mut self.current {
            Current::Host(h) => h.body.push(BodyLine::Raw(line.to_string())),
            Current::Match(m) => m.lines.push(line.to_string()),
            Current::None => self.pending_comments.push(line.to_string()),
        }
    }

    fn finish_current(&mut self) {
        match std::mem::replace(&mut self.current, Current::None) {
            Current::Host(h) => self.items.push(Item::Host(h)),
            Current::Match(m) => self.items.push(Item::Match(m)),
            Current::None => {}
        }
    }

    fn flush_pending(&mut self) {
        for c in self.pending_comments.drain(..) {
            self.items.push(Item::Comment(c));
        }
    }
}

/// Parse an SCE body comment (`#SCEGroup`, `#SCEIcon`, `#SCETags`) into its kind.
fn parse_sce_body(trimmed: &str) -> Option<SceKind> {
    let rest = trimmed.strip_prefix('#')?;
    let (kw, value) = split_first_word(rest);
    let value = value.trim().to_string();
    if kw.eq_ignore_ascii_case("SCEGroup") {
        Some(SceKind::Group(value))
    } else if kw.eq_ignore_ascii_case("SCEIcon") {
        Some(SceKind::Icon(value))
    } else if kw.eq_ignore_ascii_case("SCETags") {
        Some(SceKind::Tags(value))
    } else {
        None
    }
}

/// Split a trimmed line into `(first_word, rest_trimmed)`.
fn split_first_word(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(idx) => (&s[..idx], s[idx..].trim_start()),
        None => (s, ""),
    }
}

/// The literal `Host` patterns, dropping any inline `# comment` and keeping
/// wildcard/negation tokens (they round-trip and must not be treated as aliases).
fn parse_patterns(rest: &str) -> Vec<String> {
    rest.split_whitespace()
        .take_while(|t| !t.starts_with('#'))
        .map(|s| s.to_string())
        .collect()
}

fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

// ---------------------------------------------------------------------------
// IO: load + atomic save
// ---------------------------------------------------------------------------

/// Read and parse a config file. A missing file yields an empty document so the
/// caller can edit and create it on first save.
pub fn load_document_from(path: &Path) -> std::io::Result<SshConfigDocument> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(SshConfigDocument::parse(&s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SshConfigDocument::parse("")),
        Err(e) => Err(e),
    }
}

/// Persist a document atomically: back up any existing file to `<path>.bak`,
/// write to a sibling temp file (same directory, so the final `rename` stays on
/// one filesystem) with `0600` permissions, then rename it over the target.
pub fn save_document_atomic(path: &Path, doc: &SshConfigDocument) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;

    if path.exists() {
        let backup = backup_path(path);
        std::fs::copy(path, &backup)?;
    }

    let contents = doc.to_config_string();
    let tmp = dir.join(format!(".zap-ssh-config-{}.tmp", uuid::Uuid::new_v4()));

    // Scope the handle so it is closed before the rename, and clean up the temp
    // file on any failure between creation and the successful rename.
    let write_result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.flush()?;
        drop(file);
        set_owner_only_permissions(&tmp)?;
        std::fs::rename(&tmp, path)
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_result
}

/// `<path>.bak` sibling used for the pre-write backup.
fn backup_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "config_model_tests.rs"]
mod tests;
