## Context

`splot-recon` already owns source-backed scalar intra prediction primitives for
DC, basic/PAETH, smooth prediction, and a mutable current-frame workspace that
can extract in-storage edges and write predicted blocks. Existing tests cover
hand-picked examples and error paths, while the fuzz crate now covers decode
planning, minimal runtime hash output, and Y4M output serialization.

This change adds no-panic fuzz coverage for the existing reconstruction
surface. The fuzz input is a compact description of bounded prediction
geometry, bit depth, sample type, edge samples, destination strides, workspace
format, and optional operation variants. It is not an AV2 bitstream and does
not claim broader intra reconstruction.

## Goals / Non-Goals

**Goals:**

- Add `recon_intra_prediction_bytes` to the fuzz crate.
- Exercise direct prediction APIs for:
  - DC square and rectangular value/block/buffer prediction.
  - PAETH rectangular prediction.
  - Smooth, smooth vertical, and smooth horizontal prediction.
- Exercise `CurrentFrameWorkspace` edge extraction and workspace prediction for
  bounded Y, U, or V plane regions when the generated workspace has the plane.
- Cover both 8-bit `u8` and 8-/10-bit `u16` storage where the public API allows
  it.
- Keep all dimensions, buffers, and operation counts bounded for CI fuzz smoke.

**Non-Goals:**

- Parsing AV2 bitstreams or invoking `splot decode`.
- Claiming complete §7.13 intra reconstruction, directional prediction, data
  driven intra prediction, IBP, filter intra, CfL/CCTX, palette, residual,
  transform, quantization, loop filtering, reference refresh, or output
  scheduling support.
- Adding AVM, dav2d, ffmpeg, filesystem, network, subprocess, corpus, or new
  dependency behavior.

## Decisions

1. Fuzz `splot-recon` directly.

   Rationale: The target is structured reconstruction primitive robustness.
   Driving through `DecodeContext` would mostly re-fuzz the minimal runtime
   frontier and would not vary prepared edges, destination strides, or workspace
   geometry.

2. Generate valid sizes and sample values before calling prediction APIs.

   Rationale: The main no-panic invariant should spend time inside prediction
   and workspace code. Separate small variants may intentionally perturb edge
   lengths or output sizes to exercise typed errors, but the primary path uses
   constructor-accepted geometry and in-range samples.

3. Bound work to existing public transform block dimensions.

   Rationale: `IntraRectBlockSize` accepts log2 dimensions 2 through 6. The
   fuzzer can cover that full public range while capping workspace dimensions
   and output buffers to predictable CI-safe limits.

4. Keep workspace fuzzing source-backed and local.

   Rationale: `CurrentFrameWorkspace` only models in-storage edge extraction and
   block writes. It does not own AV2 edge availability, tile/superblock policy,
   or fallback edge synthesis, so the matrix row must remain scoped to
   source-backed workspace primitives.

## Risks / Trade-offs

- [Risk] The new row is mistaken for broad intra reconstruction support.
  Mitigation: name it as `recon`/intra prediction fuzz and keep
  `intra-reconstruction` partial.

- [Risk] Random workspace coordinates mostly hit edge-unavailable typed errors.
  Mitigation: normalize one mode to in-bounds interior blocks with prepared
  neighboring samples, and keep separate modes for boundary/error paths.

- [Risk] Fuzz buffers grow too large when 64x64 blocks and stride padding are
  combined.
  Mitigation: cap operation count, stride padding, workspace dimensions, and
  direct output allocation sizes.
