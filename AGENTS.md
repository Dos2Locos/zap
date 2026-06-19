# AGENTS.md

> This document is a navigation guide for AI/automation agents working in this repository. It summarizes the overall architecture, the responsibility of each crate in the Cargo workspace, the boundaries between the submodules under the `app/` main binary, and the engineering conventions you must follow before making any change.
>
> Use this file as the **code map**: read the architecture overview first, then jump to the right crate / module via the navigation sections below.

---

## 1. Repository Overview

Warp is a primarily Rust-based **agentic terminal / development environment**: on top of an in-house UI framework (WarpUI), it integrates terminal emulation, an AI Agent, cloud sync (Drive), code review, completions, Notebooks, settings, IPC, and more.

Top-level directories:

| Directory | Purpose |
|------|------|
| `app/` | Main binary crate (`warp`); wires together all subsystems, UI, database migrations, and platform glue layers |
| `crates/` | 67 workspace members, library crates split by responsibility |
| `command-signatures-v2/` | Standalone subproject (excluded via `--exclude` during nextest runs) |
| `script/` | Cross-platform bootstrap, build, and presubmit scripts |
| `resources/` | Runtime resources such as fonts, icons, shell integration scripts, shaders |
| `docker/` | Containerized build tooling |
| `specs/` | Product/technical spec documents |
| `.agents/skills`, `.claude/skills` | Skill descriptions for agent workflows (create PR, fix errors, feature gating, etc.) |
| `.warp/`, `.config/`, `.cargo/`, `.vscode/` | Various tool configurations |

Build system: Cargo workspace, `resolver = "2"`. `default-members` is deliberately narrowed to the subset that is frequently compiled/tested (see `Cargo.toml`). `serve-wasm` and `integration` are intentionally not part of `default-members`.

License split:
- `crates/warpui` and `crates/warpui_core` → MIT
- Everything else → AGPL-3.0-only

---

## 2. Top-Level Architecture Layers

Roughly 4 layers from bottom to top. When adding code or locating a bug, first determine which layer the change belongs to, and **do not introduce inverted cross-layer dependencies**.

```
app/  (main binary: assembly, entry points, platform glue, persistence migrations, UI view root)
  ↑
Product-domain crates: ai / computer_use / vim / onboarding /
                       warp_completer / lsp / languages / code-review …
  ↑
Framework crates: warpui / warpui_core / warpui_extras / editor /
                  ui_components / sum_tree / syntax_tree
  ↑
Infrastructure crates: warp_core / warp_util / http_client /
                       websocket / ipc / jsonrpc / persistence / graphql /
                       managed_secrets / virtual_fs / watcher / asset_cache …
```

Key architectural patterns:

1. **Entity-Handle system**: the global `App` owns all view/model entities; views reference each other through `ViewHandle<T>` rather than owning them directly.
2. **Element / Action**: the UI is composed of a declarative Element tree plus an Action event system (Flutter-style).
3. **Cross-platform**: native implementations for macOS / Windows / Linux plus a WASM target; platform code is isolated with `#[cfg(...)]`.
4. **AI integration**: Agent Mode and context indexing; the code is concentrated in `app/src/ai` (389 files) and `crates/ai`.
5. **Cloud sync**: `Drive` keeps objects synchronized across multiple devices; see `app/src/drive` and `crates/warp_files`.
6. **Feature Flags**: runtime gating is preferred over `#[cfg]`; the enum is defined in `crates/warp_core/src/features.rs`.

---

## 3. `crates/` At a Glance

The table below lists all 67 crates grouped by topic. Each row gives only a **one-line responsibility**; for implementation details, open the corresponding `crates/<name>/src/lib.rs` (many crates have `//!` module docs at the top of `lib.rs`).

### 3.1 UI Framework / View Layer

| Crate | Responsibility |
|-------|------|
| `warpui_core` | WarpUI framework core (MIT): infrastructure such as `App` / `Entity` / `ViewHandle` / `AppContext` |
| `warpui` | WarpUI higher-level components, Element tree, layout, render pipeline (MIT) |
| `warpui_extras` | Optional WarpUI extensions; not all features are enabled by default |
| `ui_components` | High-level component library reused across views (buttons, inputs, lists, modals, etc.) |
| `editor` (`warp_editor`) | Text editor: buffers, selections, cursors, key mappings, undo stack |
| `sum_tree` | Persistent balanced B-tree; the core data structure for the editor / Notebook / large lists |
| `syntax_tree` | Tree-sitter wrapper and syntax-highlighting support |
| `markdown_parser` | Markdown parsing (used for AI messages, document views, Notebooks, etc.) |
| `vim` | Vim-mode key bindings and operation semantics |
| `voice_input` | Voice input support |

### 3.2 Terminal

| Crate | Responsibility |
|-------|------|
| `warp_terminal` | Terminal emulation core: PTY management, ANSI/VT parsing, grid, scrolling, shell integration hooks |
| `input_classifier` | Terminal input intent classification (plain command / natural language / AI prompt) |
| `natural_language_detection` | Natural-language detection (works with `input_classifier`) |

### 3.3 AI / Agent

| Crate | Responsibility |
|-------|------|
| `ai` | AI model clients, prompt orchestration, the Agent protocol, the tool-calling framework |
| `computer_use` | Rust-side implementation of "Computer Use" tool capabilities (screenshots, clicks, typing, etc.) |
| `command-signatures-v2` | Command signatures v2 (command-classification metadata for AI); standalone project, not part of the main workspace test set |
| `onboarding` | New-user onboarding flow data/state |

### 3.4 Networking / Protocols / IPC

| Crate | Responsibility |
|-------|------|
| `http_client` | Workspace-wide unified HTTP client wrapper |
| `http_server` | Embedded HTTP server (local RPC, login callbacks, etc.) |
| `websocket` | WebSocket abstraction shared by native and WASM, adapting `graphql_ws_client` |
| `ipc` | Generic typed IPC request/response protocol (inter-process) |
| `jsonrpc` | JSON-RPC implementation |
| `lsp` | Language Server Protocol client implementation |
| `remote_server` | Server-side logic for the remote sshd mode |
| `serve-wasm` | Helper server that hosts the WASM build artifacts (not compiled by default) |
| `firebase` | Firebase client utilities (Crash/analytics channels, etc.) |

### 3.5 Persistence / Files / Resources

| Crate | Responsibility |
|-------|------|
| `persistence` | Diesel + SQLite persistence layer foundation; **migrations live in `app/migrations/`, the schema in `app/src/persistence/schema.rs`** |
| `warp_files` | Syncable file objects such as Drive files, Workflows, Notebooks |
| `virtual_fs` | Abstract filesystem (test mock and production real FS share the same interface) |
| `repo_metadata` | Repository metadata: file-tree construction, `.gitignore` handling, filesystem watching |
| `watcher` | Filesystem watcher (a wrapper around `notify`) |
| `asset_cache` | Disk/memory cache for assets |
| `asset_macro` | Asset-reference macros such as `bundled!` / `theme!` |
| `managed_secrets` / `managed_secrets_wasm` | Keychain / DPAPI / Linux Keyring abstraction + WASM proxy |

### 3.6 Configuration / Settings

| Crate | Responsibility |
|-------|------|
| `settings` | Settings storage and change dispatch |
| `settings_value` | The `SettingsValue` trait: controls TOML serialization semantics |
| `settings_value_derive` | The `#[derive(SettingsValue)]` proc macro (e.g., converting enum variants to snake_case) |
| `warp_features` | High-level feature-flag API (consumer side) |
| `channel_versions` | Release channels (stable/preview/dogfood) and version comparison |

### 3.7 Commands / Completions / Languages

| Crate | Responsibility |
|-------|------|
| `command` | Safe cross-platform process-spawning wrapper; **specifically handles Windows' `no_window` flag**. All newly spawned child processes must go through here |
| `warp_completer` | Completion engine (supports `--features v2`) |
| `languages` | Language / file-extension / Tree-sitter grammar registry |
| `warp_ripgrep` | Thin ripgrep wrapper used by `warp_cli` |
| `warp_cli` | CLI subcommand parsing inside the binary (`warp <subcmd>`) |
| `fuzzy_match` | Fuzzy matching + glob-style wildcards, used for path search and the command palette |

### 3.8 Platform / System Services

| Crate | Responsibility |
|-------|------|
| `app-installation-detection` | Detects apps already installed on the system (for launcher integration) |
| `prevent_sleep` | Suppresses sleep (during long tasks / AI Agent runs) |
| `isolation_platform` | Compatibility layer for running inside sandboxes such as Docker / GitHub Actions |
| `node_runtime` | Automatically installs/manages Node.js and npm (macOS/Linux/Windows × multiple architectures) |
| `warp_js` | Helper abstractions for manipulating JavaScript values/functions from the Rust side |

### 3.9 General Utilities / Communication

| Crate | Responsibility |
|-------|------|
| `warp_core` | The lowest-level "core" in the workspace: platform abstractions, plus the `FeatureFlag` enum and `DOGFOOD/PREVIEW/RELEASE_FLAGS` in `features.rs` |
| `warp_util` | General utility functions reused across many crates |
| `warp_logging` | Unified entry point for logging configuration |
| `simple_logger` | Simple async file logger for stderr-only processes such as `remote_server` |
| `warp_web_event_bus` | Web-side event bus (for the embedded web view) |
| `field_mask` | gRPC/Proto-style FieldMask utilities |
| `string-offset` | Offset primitive types (byte/char/utf16) |
| `handlebars` | Handlebars template-engine wrapper |
| `integration` | Integration-testing framework, used for testing only |

> Naming gotcha: the package name of `crates/editor` is `warp_editor`; `crates/isolation_platform` is `warp_isolation_platform`; `crates/managed_secrets` is `warp_managed_secrets`; `crates/virtual_fs` is `virtual-fs` (hyphen); `crates/string-offset` is `string-offset` (hyphen).

---

## 4. `app/` Submodule Navigation

`app/src/` contains 60+ flat product-domain directories, each roughly corresponding to one product feature line. The list below is grouped by topic; the number in parentheses is the approximate `.rs` file count, useful for estimating module size:

### 4.1 Startup / Assembly / Global
- `bin/` (7) — multiple binary entry points (main program, companion tools).
- `lib.rs` / `app_state.rs` / `app_state_tests.rs` — application state root.
- `app_menus.rs`, `app_services/`, `app_id_test.rs`
- `appearance.rs`, `gpu_state.rs`, `font_fallback.rs`, `global_resource_handles.rs`
- `dynamic_libraries.rs`, `alloc.rs`, `tracing.rs`, `profiling.rs`
- `crash_recovery.rs`, `crash_reporting/` (4)
- `features.rs` — the `app/`-side consumer of `warp_core::FeatureFlag`; adding a flag usually requires wiring it up in both places.
- `channel.rs`, `download_method.rs`, `autoupdate/` (8)

### 4.2 Terminal
- `terminal/` (427) — the main body: shell processes, PTY, grid, blocks, shell integration, command execution, I/O pipeline.
- `default_terminal/` (2) — default terminal startup logic.
- `shell_indicator.rs`, `prefix.rs` / `prefix_test.rs` (command-prefix parsing), `vim_registers.rs`

### 4.3 AI / Agent
- `ai/` (389) — includes Agent UI, conversation models, Agent management, tools/MCP, Cloud Agent, Plan/Diff views, artifacts, blocklist, execution profiles, and more. **This is the largest subtree in the repository**; before changing it, grep within this directory for the specific subtopic (`agent_*`, `conversation_*`, `cloud_agent_*`, `mcp`, `tool_*`).
- `ai_assistant/` (9) — legacy AI assistant entry point/adapter.
- `chip_configurator/`, `context_chips/` (22) — Agent context-chip selection/construction.
- `coding_entrypoints/` (5), `coding_panel_enablement_state.rs`
- `prompt/` (2), `tips/` (3), `voice/` (2), `completer/` (3)

### 4.4 Editor / Code / Review
- `editor/` (38) — main editor integration.
- `code/` (52) — code view, diff, navigation.
- `code_review/` (36) — Code Review flow.
- `notebooks/` (30), `workflows/` (22)

### 4.5 Search
- `search/` (172) — multi-target search (files, commands, Agent history, etc.).
- `search_bar.rs`

### 4.6 Server Communication / Drive / Sync
- `server/` (55) — HTTP/WS interaction with the warp backend (corresponds to the local dev mode `with_local_server`).
- `drive/` (45) — cloud-object sync entry point.
- `cloud_object/` (12) — cloud-object abstraction layer (workflow, notebook, etc.).
- `remote_server/` (5) — client-side glue for connecting to the remote-mode sshd.

### 4.7 Settings / User Config / Themes / Onboarding
- `settings/` (46), `settings_view/` (63)
- `user_config/` (6), `themes/` (11), `appearance.rs`
- `experiments/` (7), `tab_configs/` (15), `launch_configs/` (4)
- `tips/`, `banner/` (3), `quit_warning/` (1), `wasm_nux_dialog.rs`, `referral_theme_status.rs`

### 4.8 Auth / Billing / Usage
- `auth/` (22) — login, tokens, SSO.
- `billing/` (3), `pricing/` (1), `usage/` (1), `reward_view.rs`

### 4.9 Persistence
- `persistence/` (9) — Diesel migration assembly, `schema.rs` (generated by Diesel), the migration runner.
- Migration files live in the top-level `migrations/` directory (managed by the Diesel CLI).

### 4.10 Platform / System Integration
- `platform/` (2), `system/` (3) / `system.rs`
- `login_item/` (3), `antivirus/` (3), `network.rs`
- `external_secrets/` (1), `env_vars/` (14)
- `keyboard.rs` / `keyboard_test.rs`, `safe_triangle.rs` / `safe_triangle_tests.rs` (menu-hover safe triangle)

### 4.11 View Root / Panes / General UI
- `root_view.rs` / `root_view_tests.rs`
- `pane_group/` (35) — split-screen / split-pane layout.
- `tab.rs`, `command_palette.rs`, `modal.rs`, `menu.rs` / `menu_test.rs`
- `palette.rs`, `notification.rs`, `resource_center/` (10)
- `view_components/` (20), `ui_components/` (14)
- `workspace/` (54), `workspaces/` (10), `voltron.rs` (multi-window / multi-workspace coordination)
- `session_management.rs`, `undo_close/` (3), `word_block_editor.rs`
- `suggestions/` (2), `input_suggestions.rs` / `input_suggestions_test.rs`
- `plugin/` (21) — plugin-system integration.
- `uri/` (7) — `warp://` URL handling.
- `debug_dump.rs`, `debounce.rs`, `interval_timer.rs`, `throttle.rs`
- `linear.rs`, `resource_limits.rs`, `warp_managed_paths_watcher.rs`
- `preview_config_migration.rs` / `preview_config_migration_tests.rs`
- `window_settings.rs`, `projects.rs`

### 4.12 Test Infrastructure
- `integration_testing/` (79) — end-to-end integration test support.
- `test_util/` (6) — shared unit-test utilities.

---

## 5. Engineering Discipline (Hard Constraints for Agents)

> These reflect the project's engineering conventions. For this file, the verification requirement for agents is governed by `cargo check`.

### 5.1 Must-Read Conventions
- **All code comments and documentation must be written in English.**
- For searching/grepping within the git index, use the `fff` tool or `rg -n "<keyword>" <path>`; use `read_file` only for images/binaries.
- Before opening a PR / pushing a new commit, the **only** requirement is that `cargo check` passes.
- Changes must be precise: **every modified line must be traceable to the user's request**. Do not casually "improve" unrelated code, comments, or formatting.
- Prefer simplicity: do not introduce abstractions, configuration, error handling, or extra features for a single-use case.
- Explain options and expose uncertainty rather than silently making choices on the user's behalf.
- Worktree path: `.worktrees/<worktree_name>/`

### 5.2 Rust Style
- Do not add redundant type annotations to closure parameters.
- Consolidate `use` statements at the top; do not write long path-qualified prefixes inline; `#[cfg]` branches are an exception.
- Name the context parameter `ctx` and place it last; if there is also a closure parameter, the closure goes last.
- **Delete** unused parameters rather than prefixing them with `_`, and update call sites accordingly.
- Use inline format arguments in macros like `println!` / `format!` (`"{x}"` instead of `"{}", x`) to satisfy `uninlined_format_args`.
- **Do not use the `_` wildcard in `match` statements** (unless genuinely necessary); keep matches exhaustive.
- Do not delete/modify existing comments because of an unrelated change.

### 5.3 Terminal Model Lock (High Priority!)
- Calling `TerminalModel::lock()` very easily causes deadlocks (which manifest on macOS as a frozen UI / spinning beachball).
- Before adding a new `model.lock()`, you must confirm that no caller higher in the stack already holds the lock; prefer passing an already-locked reference down the call stack instead of locking again.
- Minimize the locked scope; do not call functions that may lock again while holding the lock.

### 5.4 Feature Flags
- Adding one: add a variant to the `FeatureFlag` enum in `crates/warp_core/src/features.rs`; add it to `DOGFOOD_FLAGS` / `PREVIEW_FLAGS` / `RELEASE_FLAGS` as needed.
- Using one: **prefer** the runtime `FeatureFlag::Xxx.is_enabled()` over `#[cfg(...)]`; use `cfg` only when the code would not compile without it (platform/optional dependencies).
- Wrap an entire product feature, not every individual call site; once it is stable in production, **clean up the flag and the dead branches**.
- The UI entry point and the code path must use the same flag.

### 5.5 Database
- ORM: Diesel + SQLite.
- Adding/changing the schema must go through a migration: add a new directory under `migrations/` (`up.sql` / `down.sql`). Do not hand-edit `app/src/persistence/schema.rs` (it is generated by `diesel print-schema`).

### 5.6 Testing
- Use `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`.
- Put unit tests in `${filename}_tests.rs` or `mod_test.rs`, and at the end of the original file use:

  ```rust
  #[cfg(test)]
  #[path = "filename_tests.rs"]
  mod tests;
  ```

- Use the `crates/integration` framework for integration tests; examples are in `app/src/integration_testing/`.

### 5.7 Cross-Process Commands
- Do not call `std::process::Command::new(...)` directly (especially on Windows, where it pops up a window); always go through `crates/command`.

### 5.8 Subagents / Multi-Agent
- Split a large task into parallel subtasks with **non-overlapping write domains**; information-gathering tasks can run in parallel.
- Do simple tasks directly; do not over-decompose.

---

## 6. Common Entry Points Cheat Sheet

| What you want to do | Starting point |
|---------|------|
| Change terminal grid / shell integration | `crates/warp_terminal/src/`, together with `app/src/terminal/` |
| Change Agent UI / conversation | grep within `app/src/ai/` by topic (`agent_*` / `conversation_*`) |
| Change command completions | `crates/warp_completer/` (note `--features v2`) |
| Change AI models / tool-calling protocol | `crates/ai/` |
| Add a new setting | `crates/settings_value*`, `crates/settings`; UI in `app/src/settings_view/` |
| Add a Feature Flag | `crates/warp_core/src/features.rs` + usage sites |
| Change cloud-sync objects | `crates/warp_files` + `app/src/drive/` + `app/src/cloud_object/` |
| Change the persistence schema | add a migration under `migrations/` + `crates/persistence` |
| Add a new binary tool | `app/src/bin/` |
| Platform-specific code | use `#[cfg(target_os = "...")]`; platform UI glue is in `app/src/platform/` |
| Vim mode | `crates/vim` + `app/src/vim_registers.rs` |
| Notebook / Workflow | `app/src/notebooks/`, `app/src/workflows/`, `crates/warp_files` |
| Cross-platform process spawning | `crates/command` |
| File search / watching | `crates/repo_metadata`, `crates/watcher`, `crates/warp_ripgrep` |

---

## 7. Pre-Change Checklist

Before you touch the keyboard, ask yourself once:

1. Which layer / crate / `app/src/<submodule>` does this belong to? Will the change cross a layer boundary?
2. Do you need a new dependency? If an existing workspace dependency can be reused, prefer reusing `[workspace.dependencies]` in `Cargo.toml`.
3. Is this a product feature? Does it need to be wrapped in a Feature Flag?
4. Does it involve the terminal model? Does the current call stack already hold the `TerminalModel` lock?
5. Does it involve a child process? Does it go through `crates/command`?
6. Does it involve persistence? Does it need a migration?
7. Have you written the corresponding `${file}_tests.rs`?
8. Is `cargo check` green?
9. Can every modified line be mapped one-to-one to the user's request? Should any incidental "small refactor" be reverted?

Run through all 9 items above before delivering.
