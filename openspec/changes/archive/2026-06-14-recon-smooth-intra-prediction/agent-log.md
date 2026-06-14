# Agent Log: recon-smooth-intra-prediction

## Orchestrator Plan

- Change: `recon-smooth-intra-prediction`.
- Feature ID: `RECON-INTRA-SMOOTH-PREDICTION`.
- Scope: scheduler-free AV2 §7.13.2.13 smooth intra prediction in
  `splot-recon`, plus narrow current-frame workspace helpers and
  docs/matrix/OpenSpec updates.
- Non-goals: runtime `splot decode` success, full `predict_intra()` dispatch,
  §7.13.2.1 edge availability/fallback preparation, directional prediction,
  DIP, subsampled DC, IBP, CfL, transform syntax, dequantization, inverse
  transforms, residual addition, runtime hash output, runtime Y4M output,
  reference refresh, `splot-decode -> splot-recon` dependency, scheduler state
  in `splot-recon`, and AVM/dav2d repo integration.
- Concurrency model: preserve PR #101 policy. Future decoder orchestration owns
  parallelism through `DecodeContext` and `splot_parallel::WorkerPool`;
  `splot-recon` remains pool-agnostic and scheduler-free.

## Planning Agents

### @architect

- Agent: `019ec85f-e289-7492-94e8-b3d695c63d81` (`Popper the 4th`).
- Objective: evaluate whether AV2 §7.13.2.13 smooth prediction is the right
  next PR-sized slice and identify boundary risks.
- Output:
  - Confirmed smooth prediction is appropriate after DC and PAETH if it stays a
    prepared-edge `splot-recon` primitive.
  - Required a new `intra_smooth.rs` module rather than further growth in
    `intra.rs`.
  - Required no `splot-decode -> splot-recon` dependency, no scheduler state,
    and no AV2 edge-availability or fallback policy.
  - Flagged signed rounding, sentinel edge requirements, and status overclaim
    risks.

### @spec-reader

- Agent: `019ec85f-e580-71b3-a6d7-ac2a0634cac8` (`Galileo the 4th`).
- Objective: extract pinned AV2 requirements for §7.13.2.13 smooth intra
  prediction.
- Output:
  - §7.13.2.13 consumes prepared `LeftCol[0..h]` and `AboveRow[0..w]`.
    `LeftCol[h]` is the bottom-left sentinel `bl`; `AboveRow[w]` is the
    top-right sentinel `tr`.
  - The primitive supports only `SMOOTH_PRED`, `SMOOTH_V_PRED`, and
    `SMOOTH_H_PRED`.
  - `BLEND_WEIGHT_MAX` is 32 from AV2 §3.
  - The formula uses AV2 §4.8 plain `Round2`, not `Round2Signed`, even for
    signed intermediate products. Implementation must not use unsigned
    subtraction or Rust division toward zero for negative values.
  - §7.13.2.1 preparation, `predict_intra()` dispatch, MRL, chroma clamping,
    fallback edge samples, and writing into `CurrFrame` are out of scope.

### @api-designer

- Agent: `019ec85f-e839-7740-ac83-fcfa3572dbe6` (`Ptolemy the 4th`).
- Objective: recommend the minimal public/internal API shape.
- Output:
  - Recommended public `IntraSmoothMode`, `IntraSmoothEdge`, and
    `IntraSmoothEdges<'a, T>` types.
  - Recommended writer signature:
    `predict_intra_smooth_rect_into(bit_depth, size, mode, edges, output, stride)`.
  - Recommended smooth-specific errors:
    `IntraSmoothEdgeLengthMismatch`, `IntraSmoothSampleOutOfRange`,
    `IntraSmoothPredictionOutOfRange`, and
    `WorkspaceSmoothIntraPredictionEdgeUnavailable`.
  - Recommended not widening `IntraDcEdge`, `IntraPaethEdge`, or
    `WorkspaceIntraPredictionEdgeUnavailable`.
  - Recommended a workspace helper only for strict in-storage left, above,
    bottom-left, and top-right prepared samples.

### @reference-oracle

- Agent: `019ec85f-ebc2-7c91-b4c7-35577a32785c` (`McClintock the 4th`).
- Objective: decide whether local AVM/dav2d evidence is needed.
- Output:
  - No AVM/dav2d reading or runs are needed. The local AV2 mirror fully defines
    the smooth formula, mode selection, sentinel use, and constant.
  - Use self-contained Rust tests against the mirrored spec as proof.
  - Preserve the strict boundary: no AVM/dav2d source, snippets, dependencies,
    wrappers, scripts, CI jobs, required tests, or repo integration.

## PR #113 Review Carry-Forward

The Codex review at
<https://github.com/splotlabs/splot/pull/113#pullrequestreview-4492663492>
was rechecked before this change. Its four comments are already fixed on the
base branch by later byte-planner work:

- unsupported-prefix errors keep precedence over later raw OBU limits;
- `IvfFrameCursor::next_frame_record` preserves truncated initial frame-header
  error state across retry;
- CI seeds `decode_plan_bytes` with prefixed fixture/conformance corpus inputs;
- `DecodeContext` docs describe raw-byte and parsed-stream planning accurately.

This smooth reconstruction slice does not touch byte-planner, IVF cursor,
fuzz-seed, or `DecodeContext` documentation code.

## Implementation Agents

Pending.

## Test Agents

Pending.

## Review Agents

### spec-conformance

- Agent: `019ec872-c62b-7480-9e2f-41e68d0b520e` (`Planck the 4th`).
- Scope: AV2 §7.13.2.13, §3 `BLEND_WEIGHT_MAX`, and §4.8 `Round2`.
- Result: no findings. The reviewer confirmed signed plain `Round2`, sentinel
  handling, mode selection, validation boundaries, and matrix/OpenSpec claims
  are scoped correctly to prepared-edge smooth prediction.
- Reviewer checks: `cargo test -p splot-recon smooth --locked` and
  `git diff --check`.

### performance-concurrency

- Agent: `019ec872-c90b-7be3-a333-184dec3e979a` (`Sagan the 4th`).
- Scope: PR #101 concurrency model and hot-path performance.
- Finding: the first implementation computed every smooth sample once for
  validation and again for writing in `predict_intra_smooth_rect_into`.
- Resolution: removed the duplicate validation pass. Public inputs are still
  validated before writes; computed range validation remains in the internal
  value helper as a defensive check.
- Reviewer confirmed no Rayon, crossbeam, global pool, nested pool, ad-hoc
  thread, dependency, or scheduler-state issues.

### safety-security

- Agent: `019ec872-cbe0-7b02-93fe-bd54b2895c52` (`Mendel the 4th`).
- Scope: panics, overflow, out-of-bounds indexing, partial writes, typed
  errors, sample validation, and public API safety.
- Result: no findings. The reviewer confirmed production paths introduce no
  `unwrap` or panic use, public inputs are validated before writes, indexing is
  guarded, errors are typed, and workspace writes check edge availability before
  mutation.
- Reviewer check: `CARGO_TARGET_DIR=/private/tmp/splot-review-target cargo test
  -p splot-recon smooth --locked`.

### encoder-boundary

- Agent: `019ec872-cf6d-7ab2-ba05-28864bc762cf` (`Jason the 4th`).
- Scope: encoder/dependency boundary, copied-source risk, and runtime policy.
- Result: no findings. The reviewer found no dependency changes,
  `splot-decode -> splot-recon` edge, direct scheduler usage, AVM/dav2d/rav1e/
  SVT copied-source or copied-prose indicators, or runtime decode policy leak.

### general-correctness

- Agent: `019ec872-d236-7fd1-a6e4-7fd150131d4e` (`Linnaeus the 4th`).
- Scope: tests, docs, matrix/status claims, OpenSpec consistency, and API
  maintenance.
- Finding: task 4.1 was unchecked even though docs/status artifacts were updated.
- Resolution: marked task 4.1 complete.
- Reviewer checks: `cargo test -p splot-recon --locked smooth`,
  `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`,
  `openspec validate recon-smooth-intra-prediction --strict`,
  `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`, and
  `cargo fmt --all -- --check`.

## Verification Evidence

- `openspec validate recon-smooth-intra-prediction --strict` passed before
  feature branch creation and implementation.
- `cargo test -p splot-recon --locked` passed with 123 unit tests and 0
  doctests.
- `cargo clippy -p splot-recon --all-targets --locked -- -D warnings` passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p splot-recon --no-deps --locked`
  passed.
- `cargo xtask check-source-lines` passed; it reported only pre-existing source
  line advisory warnings.
- `cargo xtask check-dependency-direction` passed.
- `cargo xtask check-concurrency-policy` passed.
- `cargo xtask check-feature-status` passed with 173 feature rows.
- `cargo xtask check-decoder-support` passed with 32 decoder-support rows.
- `openspec archive recon-smooth-intra-prediction --yes` synced the
  `decoder-support` spec and archived the change to
  `openspec/changes/archive/2026-06-14-recon-smooth-intra-prediction`.

## Local Reference Evidence

Not used. This primitive is source-backed by AV2 §7.13.2.13, §3
`BLEND_WEIGHT_MAX`, and §4.8 `Round2`.
