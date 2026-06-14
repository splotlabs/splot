## Context

`splot-recon` currently has scheduler-free square/rectangular DC prediction in
`intra.rs`, basic/PAETH prediction in `intra_basic.rs`, and current-frame
workspace helpers for in-storage DC and PAETH writes. Stage 8 still lists
smooth prediction as planned. AV2 §7.13.2.13 smooth prediction is a prepared
edge-sample process: it consumes `LeftCol`, `AboveRow`, `LeftCol[h]`, and
`AboveRow[w]` after the broader §7.13.2.1 intra process has decided
availability and fallback samples.

The PR #101 concurrency model remains binding. `splot-decode` owns runtime
orchestration through `DecodeContext` and `splot_parallel::WorkerPool`.
`splot-recon` must stay deterministic, pool-agnostic, and scheduler-free.

## Goals / Non-Goals

**Goals:**

- Add source-backed AV2 §7.13.2.13 smooth prediction for rectangular transform
  regions.
- Support the three smooth modes: `SMOOTH_PRED`, `SMOOTH_V_PRED`, and
  `SMOOTH_H_PRED`.
- Keep the primitive allocation-free over caller-owned prepared edge slices and
  caller-owned output storage.
- Validate edge lengths, sample ranges, sample type, output stride, and output
  length before writing output.
- Add a workspace helper only for in-storage left/above plus bottom-left and
  top-right sentinel samples.
- Update decoder support and feature tracking without claiming runtime decode.

**Non-Goals:**

- No full `predict_intra()` dispatcher.
- No §7.13.2.1 edge availability/fallback preparation, MRL, tile-boundary,
  superblock, palette, or CfL policy.
- No directional, DIP, subsampled DC, IBP, transform, dequantization, residual,
  filter, runtime output, reference refresh, or tile syntax support.
- No runtime `splot decode` success path.
- No `splot-decode -> splot-recon` dependency.
- No AVM/dav2d repo integration or required reference-tool execution.
- No crate dependency changes.

## Decisions

1. Add a new `crates/splot-recon/src/intra_smooth.rs` module.

   Rationale: smooth prediction is a separate §7.13.2.13 primitive and should
   not grow `intra.rs` further. The module can reuse public
   `IntraRectBlockSize`, `BitDepth`, `ReconSample`, and shared output-shape
   patterns without introducing a new crate dependency.

2. Model the prepared edge contract explicitly.

   Add `IntraSmoothEdges<'a, T>` with:

   - `left: &'a [T]` of length `h + 1`, where `left[h]` is `bl`;
   - `above: &'a [T]` of length `w + 1`, where `above[w]` is `tr`.

   Rationale: §7.13.2.13 names `LeftCol[h]` and `AboveRow[w]` directly. Using
   `h + 1` and `w + 1` keeps bottom-left/top-right sentinels source-visible and
   avoids inventing availability/fallback behavior from §7.13.2.1.

3. Provide one allocation-free writer first.

   Add `predict_intra_smooth_rect_into(bit_depth, size, mode, edges, output,
   stride)`. The argument order mirrors the existing DC and PAETH caller-owned
   writers while adding explicit mode selection. The function validates all
   inputs first, then writes the predicted rectangle. It returns typed
   `ReconError` values and never emits `decode/*` diagnostics.

4. Use signed intermediate arithmetic for the spec formula.

   `predH`, `predV`, `predH2`, and `predV2` use subtractions such as
   `left - tr` and `top - bl`, so the implementation uses signed intermediates
   and a local helper that matches AV2 §4.8 plain `Round2` with mathematical
   floor division for signed values. It must not substitute AV2
   `Round2Signed`, Rust integer division toward zero, or unsigned shifts.
   Outputs are converted back to the caller's sample type only after the
   predicted value is checked against the active bit depth.

5. Workspace helper stays policy-free.

   Add `CurrentFrameWorkspace::predict_intra_smooth_rect` only if the target
   rectangle has all four required in-storage prepared inputs: left column,
   above row, bottom-left sentinel, and top-right sentinel. Missing storage
   returns a typed edge-unavailable error; the helper does not synthesize
   fallback samples.

## Risks / Trade-offs

- [Risk] `SMOOTH_PRED` can be mistaken for full smooth dispatch support. ->
  API docs and matrix notes cite only §7.13.2.13 and explicitly exclude
  §7.13.2.1 and full `predict_intra()` dispatch.
- [Risk] Signed rounding mismatches could be subtle. -> Tests include
  non-uniform edges and cases where horizontal and vertical formulas differ.
- [Risk] Workspace sentinel requirements may be stricter than eventual AV2 edge
  preparation. -> This helper is documented as in-storage-only; future
  `predict_intra()` can prepare fallback edges above it.
- [Risk] Public edge/error enum reuse could leak mode-specific variants through
  older APIs. -> Use smooth-specific edge and mode types rather than widening
  `IntraDcEdge` or `IntraPaethEdge`.
