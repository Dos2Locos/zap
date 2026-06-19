# SSH Connection Editor Revamp — Implementation Plan (Phase 1)

> Status: DRAFT for review. No code written yet.
> Goal: make SSH connection creation/editing more useful, inspired by Tabby's
> connection editor, while keeping changes minimal, migration-safe, and aligned
> with `AGENTS.md`.

## 1. Phase 1 scope (agreed)

1. **Tabbed editor** — restructure the current flat form (`server_view.rs`) into
   tabs: **General**, **Port forwarding**, (placeholders for future Advanced /
   Ciphers / Colors).
2. **Port forwarding** — Local / Remote / Dynamic (SOCKS) forwards with an
   optional description, mapped to `ssh -L / -R / -D`.
3. **`~/.ssh/config` integration**
   - **Connect without importing** — launch a connection straight from a host
     discovered in `~/.ssh/config` (no persisted node).
   - **Sync imported nodes with the config** — keep nodes that were imported
     from `~/.ssh/config` up to date when the file changes.

Out of phase 1 (planned later): Advanced tab (ProxyJump/X11/agent/keepalive/
ControlMaster), Ciphers tab, per-connection / per-tab Colors (nice-to-have).

## 2. Architecture decisions

- **Data model: single JSON blob.** Add one nullable column
  `advanced_config TEXT` to `ssh_servers`, holding a serialized
  `SshAdvancedConfig`. This avoids one migration per option and keeps the schema
  stable as we add Advanced/Ciphers/Colors later. Core fields (host/port/user/
  auth/key) stay as columns.
- **Backward compatibility.** `advanced_config` is nullable; `NULL`/absent
  deserializes to `SshAdvancedConfig::default()`. The sync payload field is
  `#[serde(default)]` so older clients and older rows keep working.
- **Connect-without-import uses OpenSSH itself.** Launching `ssh <alias>` lets
  OpenSSH resolve the full `~/.ssh/config` entry (IdentityFile, ProxyJump, etc.)
  for free — no need to replicate every directive in our model.

## 3. Data model changes

### 3.1 `crates/warp_ssh_manager/src/types.rs`
- New types:
  ```rust
  #[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
  pub struct SshAdvancedConfig {
      #[serde(default)]
      pub port_forwards: Vec<PortForward>,
      // Reserved for later phases (kept out of Phase 1 UI):
      // pub jump_host: Option<String>,
      // pub forward_x11: bool,
      // pub forward_agent: bool,
      // pub tab_color: Option<String>,
  }

  #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
  pub struct PortForward {
      pub kind: PortForwardKind,        // Local | Remote | Dynamic
      pub bind_host: String,            // e.g. 127.0.0.1
      pub bind_port: u16,
      pub target_host: Option<String>,  // None for Dynamic (SOCKS)
      pub target_port: Option<u16>,     // None for Dynamic
      #[serde(default)]
      pub description: Option<String>,
  }

  #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
  pub enum PortForwardKind { Local, Remote, Dynamic }
  ```
- Extend `SshServerInfo` with `pub advanced: SshAdvancedConfig`, and update
  `new_default` / `clone_from_template`.
- `PortForward` gets a validation + an `to_ssh_arg()` helper:
  - Local  → `-L bind_host:bind_port:target_host:target_port`
  - Remote → `-R bind_host:bind_port:target_host:target_port`
  - Dynamic→ `-D bind_host:bind_port`

### 3.2 `crates/warp_ssh_manager/src/repository.rs`
- `server_from_row`: read `advanced_config` (Option<String>) and
  `serde_json::from_str` into `SshAdvancedConfig` (default on `None`/parse error,
  log a warning on parse error — do not fail the whole load).
- Insert/update server: serialize `advanced` to JSON, write to the new column.

### 3.3 Migration
- New dir `migrations/<timestamp>_ssh_servers_advanced_config/`:
  - `up.sql`: `ALTER TABLE ssh_servers ADD COLUMN advanced_config TEXT;`
  - `down.sql`: SQLite has no easy DROP COLUMN pre-3.35; use the standard
    table-rebuild pattern, or document that down is best-effort.
- Regenerate `app/src/persistence/schema.rs` via `diesel print-schema`
  (per `AGENTS.md §5.5`, never hand-edit `schema.rs`).
- Mirror the column in `crates/persistence` model structs
  (`NewSshServer` / the queryable row struct).

### 3.4 Sync (`crates/warp_ssh_manager/src/sync_provider.rs`)
- `SyncServer` gains `#[serde(default)] pub advanced_config: Option<String>`
  (raw JSON string, passed through verbatim).
- `collect_data`: serialize the server's `advanced` into the field.
- `apply_data`: write the raw JSON back to the `advanced_config` column.
- No secrets involved here — forwards are not sensitive. Existing keychain logic
  is untouched.

## 4. Connection command (`crates/warp_ssh_manager/src/ssh_command.rs`)
- Extend `build_ssh_args` (or a new `build_connection_args`) to append the
  port-forward options **before** the `--`/destination separator we just added,
  so they are parsed as options, not as the remote command.
- Order: `ssh [-p] [-i] [-L/-R/-D ...] -- user@host`.
- The connection-test path (`test_connection`) does **not** need forwards; keep
  it building the bare target. Forwards apply only to the real connection.
- Add unit tests asserting the emitted `-L/-R/-D` strings for each kind.

## 5. UI — tabbed editor (`app/src/ssh_manager/server_view.rs`)
Current: one flat `Column` form (~2347 loc). Refactor into:
- A small tab-bar component (reuse `ui_components` / `warpui` primitives already
  used elsewhere; no new dependency).
- **General tab**: the existing fields (name/group/host/port/user/auth/key/
  password/onekey/startup/notes) moved verbatim — no behavior change.
- **Port forwarding tab**: a list of forwards + an "add forward" row with
  Local/Remote/Dynamic toggle, bind host/port, target host/port (hidden for
  Dynamic), description; delete per row. Backed by editor state, persisted into
  `advanced.port_forwards` on Save.
- Keep tab state in the view model; default to General.
- Split the file if it approaches the 800-line guidance per tab module
  (e.g. `server_view/general.rs`, `server_view/forwarding.rs`).

## 6. `~/.ssh/config` integration (`app/src/ssh_manager/candidates.rs` + panel)
Existing: `load_candidates()` → `Vec<SshConfigCandidate>`, plus an import flow
(gh-110). New work:

### 6.1 Connect without importing
- Surface config candidates in the panel as a distinct, read-only section
  ("From ~/.ssh/config"), separate from persisted nodes.
- A **Connect** action on a candidate spawns `ssh <alias>` directly in a new
  terminal tab (no node persisted). OpenSSH resolves the rest from the file.
- No keychain interaction (OpenSSH handles auth per the file).

### 6.2 Sync imported nodes with the config
- When importing, record provenance on the node: source path + source alias
  (store in `advanced` JSON: `imported_from: { path, alias }`).
- Add a "Sync with ~/.ssh/config" action that re-parses the file and, for nodes
  with provenance, updates host/port/user/identity from the matching `Host`
  block. **One-way (config → node)** in Phase 1; surface a diff/confirmation
  before applying. Writing back to `~/.ssh/config` is explicitly out of scope.
- Detect drift (config alias removed / changed) and report it rather than
  silently deleting nodes.

## 7. Testing
- Unit (in-crate): `PortForward::to_ssh_arg` for all three kinds + validation;
  `SshAdvancedConfig` JSON round-trip; `server_from_row` default-on-null;
  `build_ssh_args` emits forwards before `--`.
- Sync: round-trip `advanced_config` through collect/apply; legacy payload
  without the field deserializes to default.
- Repository: insert/read a server with forwards.
- Manual/integration: tabbed editor renders, forwards persist, connect-from-config
  launches `ssh <alias>`, sync updates an imported node.
- Verification gate per `AGENTS.md`: `cargo check`; plus
  `cargo test -p warp_ssh_manager` and `cargo nextest` for the app where touched.

## 8. Risks & notes
- **`TerminalModel` lock** (`AGENTS.md §5.3`): the connect-from-config path
  spawns a terminal session — follow existing connect code paths; do not add new
  `model.lock()` without checking the call stack.
- **Sync schema compatibility**: older clients must tolerate the new field
  (handled via `#[serde(default)]`); new clients must tolerate rows/payloads
  without it (handled via nullable column + default).
- **`schema.rs` is generated** — regenerate, don't hand-edit.
- **Subprocess spawning** goes through `crates/command` (`AGENTS.md §5.7`).
- **English-only** comments/docs (`AGENTS.md §5.1`).
- File-size: split `server_view.rs` if tabs push it past ~800 lines.

## 9. Milestones (atomic-commit boundaries)
1. `feat(ssh_manager): add advanced_config JSON column + model/migration` — data
   model, migration, schema regen, repository read/write, sync passthrough (no UI
   yet). Verifiable by tests.
2. `feat(ssh_manager): port forwarding model + ssh arg emission` —
   `PortForward`, `to_ssh_arg`, `build_ssh_args` integration + tests.
3. `feat(ssh_manager): tabbed connection editor` — refactor `server_view` into
   tabs (General unchanged) + Port forwarding tab UI.
4. `feat(ssh_manager): connect to ~/.ssh/config hosts without importing`.
5. `feat(ssh_manager): sync imported nodes with ~/.ssh/config (one-way)`.

Each milestone: `cargo check` + relevant tests green before committing.

## 10. Resolved decisions (reviewed)
1. **Connect-without-import auth**: rely entirely on `ssh <alias>`; do NOT prompt
   for or store passwords for these ephemeral connections. ✅
2. **Sync direction**: one-way (config → node) only, with a confirmation diff;
   write-back to `~/.ssh/config` is out of scope. ✅
3. **Sync trigger**: manual button (no auto file-watch in Phase 1). ✅
4. **Colors**: deferred to a later phase. ✅
5. **Tabs**: render only the tabs that are implemented (General + Port
   forwarding in Phase 1); no disabled/placeholder tabs. ✅
