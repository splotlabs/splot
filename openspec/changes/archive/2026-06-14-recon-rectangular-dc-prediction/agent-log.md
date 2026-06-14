# Agent Log: recon-rectangular-dc-prediction

## Orchestrator Plan

- Change: `recon-rectangular-dc-prediction`.
- Feature ID: `RECON-INTRA-DC-RECTANGULAR-PREDICTION`.
- Scope: scheduler-free rectangular DC intra prediction primitives in
  `splot-recon`, plus current-frame workspace helpers and docs/matrix updates.
- Non-goals: runtime `splot decode` success, tile syntax traversal,
  dequantization, inverse transforms, residual addition, decoded-frame hash
  output, runtime Y4M output, reference refresh, `splot-decode -> splot-recon`
  dependency, scheduler state in `splot-recon`, and AVM/dav2d repo integration.
- Concurrency model: preserve PR #101 policy. Future decoder orchestration owns
  parallelism through `DecodeContext` and `splot_parallel::WorkerPool`;
  `splot-recon` remains pool-agnostic and scheduler-free.

## Planning Agents

### @architect

- Agent: `019ec818-f3e6-72a3-bdb2-baf1fbf32430` (`Plato the 4th`).
- Model / effort: GPT-5.5 / xhigh.
- Objective: evaluate whether rectangular DC is the right next PR-sized mission
  slice, identify touched files, tests, matrix/docs updates, and concurrency
  risks.
- Output:
  - Confirmed rectangular DC is the right next slice if it stays a pure
    `splot-recon` primitive.
  - Required keeping runtime decode unchanged, adding no
    `splot-decode -> splot-recon` dependency, and adding no scheduler/Rayon/
    queue state inside `splot-recon`.
  - Expected touch points: `crates/splot-recon/src/intra.rs`,
    `crates/splot-recon/src/error.rs`, `crates/splot-recon/src/workspace.rs`,
    `crates/splot-recon/src/lib.rs`, decoder support docs/status, and
    implementation matrix.
  - Main risk: source-backed `Div_Lut`/`resolve_divisor` must be correct and
    cited because square DC avoided that table.

### @spec-reader

- Agent: `019ec818-c2b5-7233-8a46-ede1d3ae15ff` (`Archimedes the 4th`).
- Model / effort: GPT-5.5 / xhigh.
- Objective: extract pinned AV2 requirements for rectangular DC prediction.
- Output:
  - AV2 §7.13.2.10 takes `haveLeft`, `haveAbove`, `log2W`, and `log2H`; derives
    `w = 1 << log2W` and `h = 1 << log2H`; fills `pred[i][j]` over the
    rectangular region.
  - Both-edge regular DC sums `LeftCol[0..h]` and `AboveRow[0..w]`, then uses
    `Clip1(approx_divide(sum, w + h))`.
  - Left-only uses `Round2(sum, log2H)`, above-only uses `Round2(sum, log2W)`,
    and no-edge uses `1 << (BitDepth - 1)`.
  - `approx_divide` uses §7.13.3.22 `resolve_divisor` and `Round2(num * scale,
    shift)`. Constants are `DIV_LUT_BITS = 7`, `DIV_LUT_NUM = 129`, and
    `DIV_LUT_PREC_BITS = 9`.
  - Rectangular regular DC must not use ordinary integer division. Subsampled DC
    (§7.13.2.11) and IBP DC remain out of scope.
  - No AVM/dav2d evidence is needed for this source-backed primitive.

### @api-designer

- Agent: `019ec818-daa6-7c91-b985-27595c6ff870` (`Euler the 4th`).
- Model / effort: GPT-5.5 / xhigh.
- Objective: recommend the narrow public/internal API shape.
- Output:
  - Recommended additive `IntraRectBlockSize { log2_width, log2_height, width,
    height, sample_count }` instead of replacing square APIs or using
    `PlaneSize`.
  - Keep `IntraSquareBlockSize` and current square functions as compatibility
    wrappers.
  - Reuse `IntraDcEdge` and `IntraDcEdges<'_, T>`, validating left length against
    height and above length against width.
  - Add allocation-free public APIs first:
    `predict_intra_dc_rect_value` and `predict_intra_dc_rect_into`.
  - Defer a new owned rectangular block type until a real caller needs it.
  - Add workspace helpers `intra_dc_edges_for_rect` and
    `predict_intra_dc_rect`; keep existing square workspace helpers as wrappers.
  - Reuse existing errors where possible; add only one rectangular block-size
    error variant.

## Implementation Agents

### @implementer / @module-implementer

- Agent: `@orchestrator` with planning subagent outputs applied directly in the
  branch worktree.
- Model / effort: GPT-5.5 / xhigh.
- Objective: implement the `splot-recon` rectangular DC primitive and workspace
  helpers from the validated OpenSpec change.
- Files changed:
  - `crates/splot-recon/src/intra.rs`
  - `crates/splot-recon/src/error.rs`
  - `crates/splot-recon/src/workspace.rs`
  - `crates/splot-recon/src/workspace_tests.rs`
  - `crates/splot-recon/src/lib.rs`
- Output:
  - Added `IntraRectBlockSize` for source-backed 4 through 64 sample
    per-dimension AV2 transform block geometry.
  - Added `predict_intra_dc_rect_value` and `predict_intra_dc_rect_into`.
  - Implemented private AV2 §7.13.3.22 `resolve_divisor` and
    `approx_divide` using the §3 divisor constants and §7.13.3.22 `Div_Lut`.
  - Preserved square DC public APIs as wrappers over the rectangular
    implementation.
  - Added workspace rectangular block writes, in-storage rectangular edge
    extraction, and rectangular DC prediction helpers.
  - Added `ReconError::InvalidIntraRectBlockLog2`.

### @integration-implementer

- Agent: `@orchestrator`.
- Model / effort: GPT-5.5 / xhigh.
- Objective: update repository-owned tracking docs without adding runtime decode
  integration.
- Files changed:
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/DECODER-SUPPORT-MATRIX.toml`
  - `docs/DECODER-SUPPORT-STATUS.md`
  - `docs/DECODER-ROADMAP.md`
  - OpenSpec artifacts under this change directory.
- Output:
  - Added `RECON-INTRA-DC-RECTANGULAR-PREDICTION`.
  - Added decoder-support row `intra-dc-rectangular-prediction`.
  - Updated square DC, current-frame workspace, scalar intra reconstruction, and
    roadmap text to remove stale rectangular-DC planned claims.
  - Regenerated feature status, spec coverage, and decoder support status.

## Test Agents

### @test-writer / @fixture-author / @fuzz-author

- Agent: `@orchestrator`.
- Model / effort: GPT-5.5 / xhigh.
- Objective: add self-contained tests for rectangular DC prediction without
  adding fixtures or fuzz target scope.
- Output:
  - Added rectangular block-size tests.
  - Added no-edge, left-only, above-only, and both-edge non-square DC tests.
  - Added invalid edge length, sample range, stride, and output length tests.
  - Added square compatibility test proving old square APIs still match the
    rectangular path.
  - Added workspace rectangular edge extraction, prediction write, frozen-frame
    hash-input interop, rectangular block write length, and out-of-bounds tests.
  - No fixture or fuzz update was needed because no byte-consuming decode entry
    point changed.

## Verification Evidence

- `cargo test -p splot-recon --locked`: passed, 99 tests.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p splot-recon --no-deps --locked`:
  passed.
- `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo xtask check-source-lines`: passed, existing unrelated advisories only.
- `cargo xtask check-dependency-direction`: passed.
- `cargo xtask check-concurrency-policy`: passed.
- `cargo xtask check-decoder-support`: passed, 30 rows.
- `cargo xtask check-feature-status`: passed, 171 features.
- `openspec validate recon-rectangular-dc-prediction --strict`: passed.
- `openspec validate --all --no-interactive`: passed, 16 items.
- `git diff --check`: passed.
- Synced the decoder-support delta spec into
  `openspec/specs/decoder-support/spec.md`.
- Archived the change at
  `openspec/changes/archive/2026-06-14-recon-rectangular-dc-prediction` with
  `--skip-specs` because the main spec was already synced.
- `cargo xtask ci`: passed, including fmt, clippy, build, tests, doctests,
  rustdoc, typos, machete, deny, OpenSpec validation, license headers,
  source-line, dependency-direction, concurrency-policy, spec mirror, fuzz
  target, generated table/explain, feature-status, reference-evidence,
  decoder-support, diagnostic-registry, and fixture checks.

## Review Agents

### @security-reviewer

- Agent: `019ec826-a157-78a3-afd6-2b1feebcd95b` (`Epicurus the 4th`).
- Result: no findings.
- Boundary: no AVM/dav2d source, snippets, binaries, submodules,
  dependencies, build probes, wrappers, CI jobs, required scripts, required
  xtask commands, or mandatory tests were added.

### @general-reviewer

- Agent: `019ec826-8870-7903-aa87-85fd174f08fc` (`Ohm the 4th`).
- Finding: broken rustdoc intra-doc link in `workspace.rs` to the removed
  `predict_intra_dc_square_into` import.
- Fix: updated the workspace square prediction docs to link to
  `Self::predict_intra_dc_rect` and verified
  `RUSTDOCFLAGS="-D warnings" cargo doc -p splot-recon --no-deps --locked`.

### @performance-reviewer

- Agent: `019ec826-ed68-7aa0-b493-a206a76e846e` (`Singer the 4th`).
- Finding: `CurrentFrameWorkspace::predict_intra_dc_rect` allocated temporary
  left/above edge vectors before filling each predicted block.
- Fix: added an internal workspace edge-sum helper and changed workspace DC
  prediction to compute trusted `left_sum` / `above_sum` from immutable plane
  storage, call the scalar rectangular DC helper, and fill the target rectangle
  directly.

### @encoder-impact-reviewer

- Agent: `019ec826-d64f-7fc2-8eb8-521b56189902` (`Chandrasekhar the 4th`).
- Result: no findings for the current slice.
- Notes: actual `splot-encode` reuse still needs an explicit future
  architecture decision; `splot-recon` remains scheduler-free and no
  `splot-encode -> splot-recon` dependency was added.

### @spec-conformance-reviewer

- Agent: `019ec826-bbc7-7693-89eb-c350f88bd37e` (`Bacon the 4th`).
- Finding: the original both-edge rectangular proof vector returned the same
  value for AV2 `approx_divide` and truncating integer division.
- Fix: changed the test vector to a non-square `sum=7, den=12` case, which
  expects AV2 approximate division output `1` while truncating integer division
  would produce `0`.

## PR #113 Review Carry-Forward

- Inspected Codex review `4492663492` on
  `https://github.com/splotlabs/splot/pull/113`.
- Comments in that review covered unsupported-prefix error precedence, IVF
  cursor state after first-frame header errors, `decode_plan_bytes` fuzz seed
  prefixing, and stale `DecodeContext` raw-byte planning docs.
- Current `main` already contains the follow-up fixes: byte planning records
  first unsupported structures before later limits, `IvfFrameCursor` preserves
  fatal first-frame header errors on retry, CI seeds prefixed
  `decode_plan_bytes` corpus inputs, and `DecodeContext` docs describe
  byte-consuming planning.
- Carry-forward for this recon slice: no byte-consuming planner, IVF cursor, or
  fuzz target is changed here; the applicable lessons are covered by
  single-sourced rectangular/square prediction wrappers and rustdoc validation.

## Local Reference Evidence

Not used. Planning agents agreed AVM/dav2d are unnecessary for this
source-backed primitive. No AVM/dav2d source, snippets, binaries, wrappers,
scripts, build probes, dependencies, CI jobs, mandatory tests, or runtime
invocations are part of this change.

## Findings And Fixes

- Fixed the rustdoc link found by review.
- Removed per-block temporary edge allocation from workspace rectangular DC
  prediction.
- Strengthened the both-edge rectangular DC proof vector so it distinguishes
  AV2 approximate division from ordinary truncating division.
- Kept `intra.rs` under the 1000-line soft budget after the review changes.

## Final Acceptance

Accepted locally. The change is archived, the main decoder-support spec is
synced, review findings are addressed, and `cargo xtask ci` passed.
