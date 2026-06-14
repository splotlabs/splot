# Agent Log: recon-basic-paeth-prediction

## Orchestrator Plan

- Change: `recon-basic-paeth-prediction`.
- Feature ID: `RECON-INTRA-BASIC-PAETH-PREDICTION`.
- Scope: scheduler-free AV2 §7.13.2.2 basic/PAETH intra prediction in
  `splot-recon`, plus narrow current-frame workspace helpers and
  docs/matrix/OpenSpec updates.
- Non-goals: runtime `splot decode` success, full `predict_intra()` dispatch,
  directional prediction, smooth prediction, DIP, subsampled DC, IBP, CfL,
  transform syntax, dequantization, inverse transforms, residual addition,
  runtime hash output, runtime Y4M output, reference refresh, `splot-decode ->
  splot-recon` dependency, scheduler state in `splot-recon`, and AVM/dav2d repo
  integration.
- Concurrency model: preserve PR #101 policy. Future decoder orchestration owns
  parallelism through `DecodeContext` and `splot_parallel::WorkerPool`;
  `splot-recon` remains pool-agnostic and scheduler-free.

## Planning Agents

### @architect

- Agent: `019ec83c-eecc-7680-8e1f-26ab9b2241da` (`Peirce the 4th`).
- Objective: evaluate whether AV2 §7.13.2.2 basic/PAETH prediction is the right
  next PR-sized slice and identify boundary risks.
- Output:
  - Confirmed basic/PAETH prediction is the right next slice after rectangular
    DC if it stays a prepared-edge `splot-recon` primitive.
  - Required a new `intra_basic.rs` module because `intra.rs` is at the
    source-line budget.
  - Required no `splot-decode -> splot-recon` dependency, no scheduler state,
    and no AV2 edge-availability policy in the workspace helper.
  - Flagged typed error modeling for top-left/missing-edge cases and signed
    arithmetic for negative `base`.

### @spec-reader

- Agent: `019ec83c-f1fb-74b1-82b5-a23789ae3dc4` (`Rawls the 4th`).
- Objective: extract pinned AV2 requirements for §7.13.2.2 basic intra
  prediction.
- Output:
  - §7.13.2.1 dispatches to the basic process only for `PAETH_PRED`; full
    `predict_intra()` edge availability and fallback preparation remain outside
    this primitive.
  - §7.13.2.2 consumes prepared `LeftCol[0..h)`, `AboveRow[0..w)`, and
    `AboveRow[-1]`.
  - Per sample, `base = AboveRow[j] + LeftCol[i] - AboveRow[-1]`; compute
    absolute distances to left, above, and top-left candidates.
  - Tie order is left first, then above, then top-left.
  - §7.13.2.2 does not clip; prediction returns one of the prepared edge
    samples, so implementation should validate prepared edge samples against the
    active bit depth and use signed intermediates for `base`.

### @api-designer

- Agent: `019ec83c-f4bc-7f11-aa81-f904c03dde9a` (`Hubble the 4th`).
- Objective: recommend narrow public/internal APIs.
- Output:
  - Recommended public `IntraPaethEdges<'a, T>` with mandatory `left`, `above`,
    and `top_left` prepared samples.
  - Recommended one allocation-free public writer:
    `predict_intra_paeth_rect_into`.
  - Recommended no square wrapper and no owned rectangular block type yet;
    square callers can use `IntraRectBlockSize::from(square)`.
  - Recommended a workspace helper `predict_intra_paeth_rect` that uses only
    in-storage top-left/left/above neighbors and returns typed errors when the
    target touches the top or left storage boundary.
  - Recommended a generic intra edge enum including `TopLeft`.

### @reference-oracle

- Agent: `019ec83c-f854-7143-b2d8-df6322be83b5` (`Lovelace the 4th`).
- Objective: decide whether local AVM/dav2d evidence is needed.
- Output:
  - No AVM/dav2d reading or runs are needed because the local AV2 mirror defines
    the formula and tie order directly.
  - Do not add local reference evidence for this feature.
  - Preserve the strict boundary: no AVM/dav2d source, snippets, dependencies,
    wrappers, scripts, CI jobs, required tests, or repo integration.

## Implementation Agents

- None used yet; implementation was kept local to `splot-recon` primitives and
  matrix/docs updates after the planning-agent boundary checks.

## Test Agents

Pending.

## Verification Evidence

- PR #113 Codex review
  <https://github.com/splotlabs/splot/pull/113#pullrequestreview-4492663492>
  was rechecked before this PR work continued. Its four comments are already
  present in this branch's base via `5f7a900 fix(decode): address byte planner
  review feedback`:
  - `plan_bytes` preserves an earlier unsupported-structure error before a
    later `max_obus` limit
    (`unsupported_prefix_is_reported_before_later_obu_limit`);
  - `IvfFrameCursor::next_frame_record` preserves truncated initial frame-header
    error state across retry
    (`frame_cursor_retry_preserves_truncated_initial_frame_header_error`);
  - CI seeds `decode_plan_bytes` with prefixed fixture and conformance corpus
    inputs so the bitstream payload remains unshifted;
  - `DecodeContext` docs now describe raw-byte and parsed-stream planning entry
    points without saying the context never reads raw bitstream bytes.
- Focused PAETH verification:
  - `cargo fmt --all -- --check` passed.
  - `cargo test -p splot-recon --locked` passed: 111 tests plus doc-tests.
  - `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`
    passed.
  - `RUSTDOCFLAGS="-D warnings" cargo doc -p splot-recon --no-deps --locked`
    passed.
  - `cargo xtask check-source-lines` passed with existing advisory warnings only.
  - `cargo xtask check-dependency-direction` passed.
  - `cargo xtask check-concurrency-policy` passed.
  - `cargo xtask check-decoder-support` passed.
  - `cargo xtask check-feature-status` passed.
  - `openspec validate recon-basic-paeth-prediction --strict` passed.
  - `openspec validate --all --no-interactive` passed.
  - `git diff --check` passed.
- Post-review verification after applying review fixes:
  - `cargo fmt --all -- --check` passed.
  - `cargo test -p splot-recon --locked` passed: 111 tests plus doc-tests.
  - `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`
    passed.
  - `RUSTDOCFLAGS="-D warnings" cargo doc -p splot-recon --no-deps --locked`
    passed.
  - `cargo xtask check-decoder-support` passed.
  - `cargo xtask check-feature-status` passed.
  - `git diff --check` passed.

## Review Agents

- @spec-conformance: `019ec84c-1b3b-7032-9852-3f21e79343da`
  (`Feynman the 4th`).
  - Finding: PAETH feature rows overclaimed §7.13.2.1 support even though the
    implementation starts from prepared edges and intentionally does not
    implement §7.13.2.1 availability/fallback preparation.
  - Fix: removed §7.13.2.1 from `RECON-INTRA-BASIC-PAETH-PREDICTION` in
    `docs/IMPLEMENTATION-MATRIX.toml` and `docs/DECODER-SUPPORT-MATRIX.toml`,
    regenerated `docs/SPEC-COVERAGE.md` and `docs/DECODER-SUPPORT-STATUS.md`.
- @performance-concurrency: `019ec84c-2cb9-7712-9c9c-c1e984450af9`
  (`Dirac the 4th`).
  - Finding: workspace PAETH recomputed the invariant above-row sample index in
    the per-pixel loop.
  - Fix: precomputed the above row range once before the row/column loops; no
    concurrency-policy findings.
- @safety-security: `019ec84c-3da7-7e90-b917-49c2f941bebe`
  (`Poincare the 4th`).
  - Finding: changing the public `IntraDcEdge` enum into an alias for a broader
    enum exposed a non-DC `TopLeft` variant through the DC name.
  - Fix: restored `IntraDcEdge` as the DC-only enum and added dedicated
    `IntraPaethEdge`, `IntraPaethEdgeLengthMismatch`, and
    `IntraPaethSampleOutOfRange` for PAETH validation.
- @encoder-boundary: `019ec84c-cf0a-73d0-8d24-22dcae8e5c89`
  (`Pasteur the 4th`).
  - Result: no findings; no encoder-facing files, manifests, dependency graph,
    third-party material, or license boundaries were changed.
- @general-correctness: `019ec84c-e0d7-7300-b404-a40577293a79`
  (`Laplace the 4th`).
  - Findings: initial post-fix tree needed rustfmt; the tie-order test did not
    make left-first behavior observable; proof rows omitted invalid-input tests.
  - Fixes: ran rustfmt and verified `cargo fmt --all -- --check`; changed the
    tie test to an observable left/top-left tie; added invalid-input PAETH tests
    to the implementation matrix and decoder support matrix evidence, then
    regenerated status docs.

## Local Reference Evidence

Not used yet. The planned primitive is source-backed by AV2 §7.13.2.2 and does
not require AVM/dav2d evidence unless a spec ambiguity appears.

## Findings And Fixes

- Addressed review findings:
  - Removed PAETH §7.13.2.1 support overclaim from feature/support rows while
    keeping notes that edge preparation remains unsupported.
  - Preserved the public DC-only `IntraDcEdge` API and introduced PAETH-specific
    edge/error types.
  - Precomputed the invariant workspace above-row range in PAETH prediction.
  - Strengthened tie-order tests and matrix proof evidence.

## Final Acceptance

- OpenSpec delta synced to `openspec/specs/decoder-support/spec.md`.
- Change archived to
  `openspec/changes/archive/2026-06-14-recon-basic-paeth-prediction/`.
- Final local gates are run after archive and before commit/PR.
