# Change: fixtures-manifest-and-check

## Feature IDs

- `XTASK-CHECK-FIXTURES`

## Why

The hand-crafted `tests/fixtures/*.av2` corpus had no machine-checkable manifest:
its hashes were unpinned (a fixture could be silently mutated), and its expected
validator outcomes lived only in prose in `tests/fixtures/README.md` (drift-prone).
A committed manifest plus a hermetic gate makes the corpus tamper-evident and its
documented outcomes verifiable, mirroring the discipline the AVM conformance corpus
already has — with no external decoder.

## Scope

- Spec sections: none (test-corpus integrity tooling, not AV2 syntax).
- Crates/modules: `tests/fixtures/MANIFEST.toml` (new); `xtask/src/fixtures.rs`
  (new); `xtask/src/main.rs` (additive: module, `Task::CheckFixtures`, dispatch arm,
  `run_ci` step); `crates/splot-cli/tests/fixture_manifest.rs` (new in-process
  outcome test); `docs/FIXTURES.md` (new).
- CLI/docs/tests: `cargo xtask check-fixtures`; `.github/workflows/ci.yml` step;
  `AGENTS.md` §4 command list; `docs/IMPLEMENTATION-MATRIX.toml` row.

## Non-goals

- Does not change any AV2 parser/validator semantics or diagnostics.
- `check-fixtures` does not run the validator or any decoder (hermetic hash +
  metadata only); outcome verification is the in-process test's job.
- Does not hash the AVM conformance vectors (`tests/conformance/**`) — that corpus
  has its own manifest and runner; a shared hash mechanism is a possible follow-up.
- No `--update` rewrite mode: a hash mismatch reports the on-disk hash to paste.

## Acceptance criteria

- [ ] Matrix row `XTASK-CHECK-FIXTURES` exists.
- [ ] `tests/fixtures/MANIFEST.toml` lists every committed `.av2` fixture with a
      pinned `sha256`, `category`, and `expect`.
- [ ] `cargo xtask check-fixtures` verifies presence, hashes, uniqueness,
      category/expect consistency, and no orphans — hermetically — and is folded
      into `cargo xtask ci` plus a CI step.
- [ ] An in-process test verifies each `expect` against the real validator.
- [ ] `docs/FIXTURES.md` documents the format and the add/update workflow.
- [ ] `cargo xtask check-feature-status` and `cargo xtask ci` pass.
