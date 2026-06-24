# Agent Workflow

Use this file for the normal contribution flow. Keep root `AGENTS.md` limited to
the rules agents need before deciding which detailed document to open.

## Before Editing

1. Run `git status --short`.
2. Inspect the files you are about to change.
3. Preserve existing user work. Do not discard uncommitted changes unless the
   maintainer explicitly asks for that operation.

## Feature Tracking

Every non-trivial change uses a stable Feature ID from:

```text
docs/IMPLEMENTATION-MATRIX.toml
```

Before implementing:

1. Find or create the matrix row.
2. Find or create an OpenSpec change under `openspec/changes/` unless the work
   is trivial.
3. Use the Feature ID in code comments, diagnostics, tests, and PR text.
4. Use `TODO(spec: <FEATURE-ID>): ...` for intentionally unmapped AV2 details.

Before finishing:

```bash
cargo xtask feature-status
cargo xtask check-feature-status
cargo xtask ci
```

Do not mark a stage `done` unless proof is recorded in the matrix row. The
schema and status model live in [../FEATURE-TRACKING.md](../FEATURE-TRACKING.md)
and [../IMPLEMENTATION-MATRIX.schema.md](../IMPLEMENTATION-MATRIX.schema.md).

## Generated Documentation

Do not hand-edit generated status files. Regenerate them with the relevant
`cargo xtask` command from [commands.md](./commands.md).

Generated files include:

- `docs/FEATURE-STATUS.md`
- `docs/SPEC-COVERAGE.md`
- `docs/spec-coverage-writer.md`
- `docs/DECODER-SUPPORT-STATUS.md`
- `docs/DECODER-SPEC-COVERAGE.md`

## Commit and PR Titles

Use Conventional Commits for every commit subject and pull request title:

```text
<type>[optional scope][!]: <description>
```

Allowed types:

```text
build chore ci docs feat fix perf refactor revert style test
```

Local checks:

```bash
cargo xtask check-conventional-commits
cargo xtask check-conventional-title "feat: add example"
```

Sync pushed feature branches by merging `main`; do not force-push a pushed
branch just to sync it. The conventional-commit checker skips git-generated
multi-parent sync commits whose subject starts with `Merge `.

Use squash or rebase merges only when merging to `main`; generated GitHub merge
commits are not Conventional Commit subjects.

## When to Ask

Ask the maintainer before:

- Making algorithmic encoder choices.
- Resolving ambiguous AV2 spec interpretation.
- Adding a third-party dependency.
- Changing the crate dependency graph.
- Changing legal or licensing terms.
