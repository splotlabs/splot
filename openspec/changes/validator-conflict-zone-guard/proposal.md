# Change: validator-conflict-zone-guard

## Feature IDs

- `XTASK-CONFLICT-ZONE-GUARD`

## Why

The validator/inspector productization work runs concurrently with a separate
decoder stream. To guarantee zero merge conflicts, validator-stream changes must
never create, edit, or delete decoder-owned files (`crates/splot-decode/**`,
`crates/splot-recon/**`, `docs/DECODER-*`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
`fuzz/fuzz_targets/decode*`, `crates/splot-cli/src/commands/decode.rs`, or any
AVM/dav2d integration). Today that boundary is enforced only by eye. A committed,
mechanical guard removes that risk and lets every productization PR prove — with a
single command — that its diff against `main` touches nothing the decoder owns.

## Scope

- Spec sections: none (project automation / tooling gate).
- Crates/modules: `xtask/src/conflict_zone.rs` (new); `xtask/src/main.rs`
  (additive: module decl, `Task` variant, dispatch arm, `run_ci` step).
- CLI/docs/tests: `cargo xtask check-conflict-zone`; `.github/workflows/ci.yml`
  (additive decoder-safe step); `AGENTS.md` §4 command list;
  `docs/IMPLEMENTATION-MATRIX.toml` row; unit tests in the new module.

## Non-goals

- Does not change any AV2 parser/validator semantics or diagnostics.
- Does not touch, rebase, or comment on decoder-owned files, branches, or PRs.
- Does not introduce a permanent ban on decoder edits: the guard is
  merge-base-relative and decoder-stream branches are exempt (see design).
- Does not add a new dependency or a new CI workflow file.

## Acceptance criteria

- [ ] Implementation matrix row `XTASK-CONFLICT-ZONE-GUARD` exists.
- [ ] `cargo xtask check-conflict-zone` exits non-zero when the diff vs `main`
      touches any conflict-zone path, and zero otherwise.
- [ ] The guard is folded into `cargo xtask ci` and runs as a decoder-safe step
      in CI.
- [ ] The guard degrades gracefully (skip-with-notice) when no `main` base is
      resolvable, on `main` itself, and on decoder-stream branches.
- [ ] Unit tests cover the forbidden/allowed path classification, including the
      `av2`-vs-`avm` and OpenSpec-`avm-*` false-positive guards.
- [ ] `cargo xtask check-feature-status` and `cargo xtask ci` pass.
