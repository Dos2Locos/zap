```markdown
# zap Development Patterns

> Auto-generated skill from repository analysis

## Overview
This skill covers the core development patterns of the `zap` Rust codebase. You'll learn the project's coding conventions, commit styles, and the main workflows for updating documentation and extending the SSH manager feature. This guide enables contributors to follow established practices for consistency, maintainability, and collaboration.

## Coding Conventions

### File Naming
- Use **snake_case** for all file and module names.
  - Example: `ssh_command.rs`, `server_view_tests.rs`

### Imports
- Use **relative imports** within modules.
  - Example:
    ```rust
    use super::types::SshCommandType;
    use crate::repository::Repository;
    ```

### Exports
- Use **named exports** for public items.
  - Example:
    ```rust
    pub struct SshCommand { ... }
    pub fn execute_command(...) { ... }
    ```

### Commit Messages
- Follow **conventional commit** format.
  - Prefixes: `docs`, `fix`, `feat`, `test`
  - Example: `feat: add new sync provider for SSH manager`

## Workflows

### Update Project Documentation
**Trigger:** When you need to update, translate, or mark progress in project documentation or planning files.  
**Command:** `/update-docs`

1. Edit the relevant documentation file (e.g., `AGENTS.md`, `specs/ssh-editor-revamp/PLAN.md`) to:
    - Add translations
    - Update milestones
    - Change conventions or document new policies
2. Commit your changes with a descriptive message, such as:
    ```
    docs: update PLAN.md with new milestone dates
    ```
3. Push your changes and open a pull request if required.

**Example:**
```markdown
# AGENTS.md

## New Agent Conventions
- All agent names must be unique.
- Agents should be documented in both English and French.
```

---

### Extend SSH Manager Feature
**Trigger:** When you want to add a new capability or fix issues in the SSH manager integration.  
**Command:** `/extend-ssh-manager`

1. Modify or add Rust source files in `crates/warp_ssh_manager/src/`:
    - Examples: `db.rs`, `repository.rs`, `types.rs`, `ssh_command.rs`, `sync_provider.rs`
2. Update or add related test files:
    - Examples: `ssh_command_tests.rs`, `server_view_tests.rs`
3. If persistence changes are needed:
    - Update `crates/persistence/src/schema.rs`
    - Create new migration SQL files in `crates/persistence/migrations/`
4. Update application code to use new or changed features:
    - Example: `app/src/ssh_manager/`
5. Run and verify tests to ensure correctness.

**Example:**
```rust
// crates/warp_ssh_manager/src/ssh_command.rs
pub fn new_sync_command(...) -> SshCommand {
    // implementation
}

// crates/warp_ssh_manager/src/ssh_command_tests.rs
#[test]
fn test_new_sync_command() {
    // test implementation
}
```

## Testing Patterns

- Test files follow the pattern: `*.test.*` (e.g., `ssh_command_tests.rs`)
- Testing framework is not explicitly specified; use Rust's built-in test framework.
- Example test:
    ```rust
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_execute_command_success() {
            // Arrange
            // Act
            // Assert
        }
    }
    ```

## Commands

| Command             | Purpose                                                          |
|---------------------|------------------------------------------------------------------|
| /update-docs        | Update, translate, or mark progress in documentation or planning |
| /extend-ssh-manager | Add or improve SSH manager features and related persistence      |
```
