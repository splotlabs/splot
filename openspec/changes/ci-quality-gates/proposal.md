# Proposal: Complete the blocking CI quality gates (docs build + coverage threshold)

## Feature IDs

- `XTASK-CI-QUALITY-GATES` (new automation row)
- `CONF-FUZZ-NO-PANIC` (stale fuzz-in-CI note fix only; no stage change)

## Why

Two acceptance-gate dimensions are missing or non-binding, and three in-repo
claims about the gates are false, which misdirects both human reviewers and the
review automation:

1. **No docs-build gate exists anywhere** — neither `cargo xtask ci`
   (`xtask/src/main.rs::run_ci`) nor `.github/workflows/ci.yml` runs
   `cargo doc`, and a strict build fails today: `RUSTDOCFLAGS="-D warnings"
   cargo doc --workspace --no-deps --locked` reports 10 rustdoc errors in
   `splot-core` (an unresolved intra-doc link and a private-item link in
   `headers/operating_point_set.rs`, two private-item links in
   `headers/quantizer_matrix.rs`, and six redundant explicit link targets in
   `headers/quantizer_matrix.rs` and `tile.rs`). Broken public docs contradict
   the "every public item has a doc comment" convention in `AGENTS.md` § 5.
2. **The coverage job can never gate.** The `coverage` job in `ci.yml` sets
   job-level `continue-on-error: true` and enforces no threshold, with a TODO
   to add one "once a baseline is established". The baseline now exists:
   `splot-validate` measures ≈97% line coverage (workspace 92.8%), so a
   blocking ≥90%-lines gate on `splot-validate` — the crate whose diagnostics
   are the product — lands green with ~7 points of headroom.
3. **Gate drift and stale claims:**
   - `cargo xtask ci` runs `cargo-deny check bans licenses sources` without
     `--all-features`; the CI supply-chain job passes `--all-features`, so the
     local gate can pass where CI fails.
   - `.github/workflows/claude-review.yml` tells the reviewer "The cargo-deny
     license/advisory job is advisory-only — it runs with
     `continue-on-error: true` and never blocks merge", but the deterministic
     bans/licenses/sources check has been a blocking job since the split; only
     the advisory-DB check is informational.
   - `fuzz/Cargo.toml` says the fuzz crate "is not part of normal CI" and the
     `CONF-FUZZ-NO-PANIC` matrix note says the cargo-fuzz target "is not in
     CI", but `ci.yml` has run a blocking 60-second `parse_obu` fuzz smoke on
     every PR since the fuzz-smoke job landed.

## Scope

- Spec sections: none (no AV2 syntax change; repository process only).
- Crates/modules:
  - `crates/splot-core` — fix the 10 rustdoc errors (doc comments only; no
    code-behavior change).
  - `xtask/src/main.rs` — `run_ci` gains a strict docs step
    (`cargo doc --workspace --no-deps --locked` with
    `RUSTDOCFLAGS=-D warnings`); `run_cargo_deny_offline` gains
    `--all-features`; the `coverage` task gains the same `--fail-under-lines`
    gate CI enforces so local runs match CI.
  - `.github/workflows/ci.yml` — `ci` job gains the strict docs step; the
    `coverage` job drops `continue-on-error`, switches to a
    `--no-report` run followed by `report` invocations (workspace summary +
    lcov artifact, unchanged in spirit), and adds a blocking
    `cargo llvm-cov report --fail-under-lines 90` scoped to `splot-validate`
    via `--ignore-filename-regex`.
  - `.github/workflows/claude-review.yml` — correct the cargo-deny gate
    description.
  - `fuzz/Cargo.toml` — correct the "not part of normal CI" comment.
- CLI/docs/tests: `AGENTS.md` § 4 command list gains the docs-build command;
  matrix row `XTASK-CI-QUALITY-GATES` added; `CONF-FUZZ-NO-PANIC` note
  corrected; generated `FEATURE-STATUS.md`/`SPEC-COVERAGE.md` regenerated;
  audit ledger re-recorded.

## Non-goals

- Running `openspec validate --all` inside `cargo xtask ci`: blocked on fixing
  the two main-spec keyword failures, deferred to the `openspec-matrix-hygiene`
  change so this change stays green end to end.
- New fuzz targets or smoke-job changes (the `fuzz-validator-targets` change).
- Coverage thresholds for crates other than `splot-validate`, per-function/
  region thresholds, or Codecov upload (the lcov artifact stays).
- Changing the advisory-DB cargo-deny check from informational to blocking
  (its non-gating rationale in `ci.yml` is correct and stays).

## Acceptance criteria

- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`
  passes locally and as a blocking step in both `cargo xtask ci` and the CI
  `ci` job.
- [ ] The CI `coverage` job has no `continue-on-error`, and a
  `--fail-under-lines 90` report scoped to `splot-validate` files gates the
  merge; the workspace summary and lcov artifact are still produced.
- [ ] `cargo xtask coverage` enforces the same threshold locally
  (run-if-present semantics for `cargo-llvm-cov` unchanged).
- [ ] `cargo xtask ci` cargo-deny invocation matches CI
  (`--all-features`).
- [ ] The claude-review prompt, `fuzz/Cargo.toml`, and the
  `CONF-FUZZ-NO-PANIC` matrix note accurately describe the gates.
- [ ] Implementation matrix row `XTASK-CI-QUALITY-GATES` exists with proof;
  `cargo xtask check-feature-status` and `cargo xtask ci` pass.
