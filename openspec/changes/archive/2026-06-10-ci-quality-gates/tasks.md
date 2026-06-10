# Tasks: Complete the blocking CI quality gates

## 1. Pre-implementation bookkeeping

- [x] 1.1 docs/IMPLEMENTATION-MATRIX.toml: add the `XTASK-CI-QUALITY-GATES`
  automation row (`openspec_change = "ci-quality-gates"`); correct the stale
  "not in CI" sentence in the `CONF-FUZZ-NO-PANIC` note (no stage change).
- [x] 1.2 Register the change in `openspec/changes/README.md` (Active changes
  table).

## 2. Docs-build gate

- [x] 2.1 Fix the 10 rustdoc errors in `splot-core`
  (`headers/operating_point_set.rs`: unresolved `Error` link, private
  `OPS_MLAYER_INFO_IDC_RESERVED` link; `headers/quantizer_matrix.rs`: two
  private `user_defined_qm` links and redundant explicit link targets;
  `tile.rs`: redundant explicit link targets). Doc-comment-only edits.
- [x] 2.2 `xtask/src/main.rs::run_ci`: add a strict docs step running
  `cargo doc --workspace --no-deps --locked` with `RUSTDOCFLAGS=-D warnings`
  (merge with any caller-provided RUSTDOCFLAGS is out of scope; set it for the
  child process only).
- [x] 2.3 `.github/workflows/ci.yml` `ci` job: add the same strict docs step.
- [x] 2.4 `AGENTS.md` § 4: add the docs-build command to the command list.

## 3. Coverage threshold gate

- [x] 3.1 `.github/workflows/ci.yml` `coverage` job: remove
  `continue-on-error: true`; restructure as `cargo llvm-cov --workspace
  --all-features --locked --no-report` followed by `report` steps: workspace
  `--summary-only`, lcov `--output-path lcov.info` (artifact unchanged), and a
  blocking `--fail-under-lines 90` with `--ignore-filename-regex` excluding
  everything except `crates/splot-validate/`.
- [x] 3.2 `xtask/src/main.rs` coverage task: enforce the same
  `--fail-under-lines 90` splot-validate-scoped report after the HTML report,
  and drop the now-stale "no threshold is enforced here" TODO comment.
- [x] 3.3 `xtask/src/main.rs::run_coverage`: fix the run-if-present probe.
  `tool_available("cargo-llvm-cov")` always failed because the binary rejects a
  bare `--version` (it expects the `llvm-cov` subcommand first), so the threshold
  report silently skipped even when the tool was installed. Add a
  `tool_available_with_args(bin, args)` helper (with `tool_available` delegating
  to it for `--version`) and probe `["llvm-cov", "--version"]` in `run_coverage`,
  keeping the skip-with-install-hint path for a genuinely absent tool.

## 4. Parity and stale-claim fixes

- [x] 4.1 `xtask/src/main.rs::run_cargo_deny_offline`: add `--all-features` to
  match the CI supply-chain job.
- [x] 4.2 `.github/workflows/claude-review.yml`: correct the cargo-deny gate
  description (bans/licenses/sources block; only the advisory-DB check is
  informational).
- [x] 4.3 `fuzz/Cargo.toml`: replace "It is not part of normal CI." with an
  accurate note about the blocking 60s `parse_obu` smoke job in `ci.yml`.

## 5. Registry, docs, and generated artifacts

- [x] 5.1 Regenerate `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`;
  re-record the audit ledger (`cargo xtask audit-scope --all --write-ledger`).

## 6. Verification

- [x] 6.1 `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`
  passes.
- [x] 6.2 `cargo llvm-cov report --fail-under-lines 90` scoped to
  `splot-validate` passes locally (baseline ≈97%).
- [x] 6.3 `cargo xtask feature-status` and `cargo xtask check-feature-status`
  pass with the new row.
- [x] 6.4 `cargo xtask ci` passes end to end with
  `RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin`.
