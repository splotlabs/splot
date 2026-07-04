# Change: doc-budget-gate

## Feature IDs

- `XTASK-DOC-BUDGET`

## Why

The repository had accumulated generated status renders, roadmap prose, audit
writeups, and archived OpenSpec output that made the real operating docs harder
to find. The repo needs a small committed manual-doc set and an automated gate so
the shrink does not immediately regress.

## Scope

- Add `cargo xtask check-doc-budget`.
- Add `tools/docs/budget.toml` with counted, excluded, allowed, banned, and
  generated-on-demand markdown paths.
- Wire the gate into `cargo xtask ci` and GitHub CI.
- Keep generated status renders available through xtask commands while banning
  the committed markdown outputs.

## Non-goals

- No AV2 syntax, diagnostics, codec behavior, dependency graph, license, or
  third-party spec mirror changes.
- No deletion of active OpenSpec process state.

## Acceptance criteria

- [ ] `XTASK-DOC-BUDGET` exists in the implementation matrix with proof.
- [ ] `cargo xtask check-doc-budget` counts manual markdown and fails on budget,
      generated-status, roadmap/status/coverage, or archive/old/notes violations.
- [ ] `cargo xtask ci` and `.github/workflows/ci.yml` run the gate.
- [ ] Generated status commands still work on demand and drift-check committed
      renders if a local file exists.
- [ ] The final change records before/after markdown file and line counts.
