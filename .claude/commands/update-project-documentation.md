---
name: update-project-documentation
description: Workflow command scaffold for update-project-documentation in zap.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /update-project-documentation

Use this workflow when working on **update-project-documentation** in `zap`.

## Goal

Updates or translates project documentation files, such as plans or code maps, often to reflect new conventions, milestones, or language policies.

## Common Files

- `AGENTS.md`
- `specs/ssh-editor-revamp/PLAN.md`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Edit the relevant documentation file (e.g., AGENTS.md, PLAN.md) to add translations, update milestones, or change conventions.
- Commit the changes with a descriptive message indicating the nature of the documentation update.

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.