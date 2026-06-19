---
name: extend-ssh-manager-feature
description: Workflow command scaffold for extend-ssh-manager-feature in zap.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /extend-ssh-manager-feature

Use this workflow when working on **extend-ssh-manager-feature** in `zap`.

## Goal

Implements new features or improvements in the SSH manager, typically involving changes to Rust source files, types, persistence layer, and tests.

## Common Files

- `crates/warp_ssh_manager/src/*.rs`
- `app/src/ssh_manager/*.rs`
- `crates/persistence/migrations/*.sql`
- `crates/persistence/src/schema.rs`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Modify or add Rust source files in crates/warp_ssh_manager/src (e.g., db.rs, repository.rs, types.rs, ssh_command.rs, sync_provider.rs).
- Update or add related test files (e.g., ssh_command_tests.rs, server_view_tests.rs).
- If persistence changes are needed, update schema.rs and create new migration SQL files.
- Update application code to use new or changed features (e.g., app/src/ssh_manager/*).
- Run and verify tests.

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.