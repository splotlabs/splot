# Change: encoder-reference-gate

## Feature IDs

- `DOC-ENCODER-REFERENCE-GATE`

## Why

Future encoder work needs a clear policy for using rav1e and SVT-AV1 as research references without
copying AV1 syntax, source code, tables, constants, entropy CDFs, comments, or prose into `splot`.

## Scope

- Spec sections: none (docs/workflow only).
- Crates/modules: no Rust code.
- CLI/docs/tests: `docs/references/`, `AGENTS.md`, README, architecture, spec mapping, code review,
  Copilot instructions, and PR template guidance.

## Non-goals

- No AV2 syntax, reconstruction, reference-state, or layer behavior is implemented.
- No third-party source code or upstream documentation is vendored.
- No crate dependency graph changes.

## Acceptance criteria

- [x] Implementation matrix row exists.
- [x] Reference docs describe rav1e/SVT-AV1 as inspiration only.
- [x] Agent and reviewer instructions require consulting the reference docs before encoder work.
- [x] PR template captures decoder-visible behavior and `docs/SPEC-MAPPING.md` update status.
- [x] `cargo xtask check-feature-status` passes.
