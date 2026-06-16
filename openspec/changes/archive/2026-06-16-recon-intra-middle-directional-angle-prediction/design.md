## Context

`splot-recon` already exposes source-backed intra prediction primitives for DC,
subsampled DC, IBP DC, PAETH, smooth, H/V cardinal directional prediction, and
the non-IDIF one-sided directional-angle cases `45`, `67`, and `203`.

The remaining non-IDIF middle branch of AV2 7.13.2.8 is different from the
one-sided branch because it can read both `AboveRow` and `LeftCol`, and those
reads use signed logical indices. The smallest honest next step is a standalone
prepared-edge primitive for pAngles `113`, `135`, and `157`, not full
`predict_intra()` dispatch.

## Goals / Non-Goals

**Goals:**

- Add Feature ID `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION`.
- Add a public `splot-recon` API for non-IDIF middle directional-angle
  prediction over caller-prepared logical edge ranges.
- Support pAngles `113`, `135`, and `157` with AV2 7.13.2.8 bilinear math.
- Validate all public inputs, logical edge coverage, sample ranges, output
  shape, and arithmetic before output mutation.
- Extend unit tests, fuzz coverage, matrices, generated docs, and OpenSpec
  records for the narrow capability.

**Non-Goals:**

- Full AV2 7.13.2.1 edge availability/fallback preparation.
- Full AV2 7.13.2.7 mode dispatch, `Mode_To_Angle`, angle deltas, MRL, or
  wide-angle mapping.
- Luma IDIF and the `Dr_Interp_Filter` table.
- Directional IBP, current-frame workspace edge synthesis, runtime `splot
  decode`, AVM or dav2d integration, new dependencies, or encoder-facing
  changes.

## Decisions

1. Add a middle-angle API instead of broadening the one-sided API.

   The existing one-sided row intentionally says pAngles `113`, `135`, and
   `157` are excluded. A separate middle-angle API keeps that row true and gives
   the new Feature ID its own tests, matrix row, and documentation. Shared
   helpers can still live near `intra_directional_angle.rs` if that keeps the
   implementation compact.

2. Represent prepared edges as fixed logical-indexed slices.

   The middle branch can read `AboveRow[-1]` and negative `LeftCol` positions.
   The API accepts exact slices where `slice[0]` maps to the spec's logical
   `-1` sample and subsequent elements map to logical indices `0..`. It then
   validates that every referenced `base` and `base + 1` index is covered before
   writing output. This avoids pretending that negative spec indices are normal
   zero-based Rust slices while keeping the current narrow API simple.

3. Validate by walking the requested block before mutation.

   Validation will compute the exact branch, base, shift, edge kind, and sample
   coverage for every `(i, j)` output sample using signed checked arithmetic.
   Only after this pass succeeds will the write loop mutate caller storage.

4. Keep derivative constants local to `splot-recon`.

   The slice needs AV2 9.2 entries `Dr_Intra_Derivative[23] = 170`,
   `[45] = 64`, and `[67] = 24`. Encoding those spec-defined entries locally
   avoids adding a `splot-core` dependency to `splot-recon`. A future broader
   directional implementation can introduce a recon-local table if the local
   constant set becomes unwieldy.

5. Keep runtime decode and workspace synthesis out of scope.

   Current-frame workspace helpers do not yet prepare the full AV2 7.13.2.1
   logical edge ranges. The primitive remains caller-prepared and memory-only;
   runtime integration can land once edge preparation and block dispatch are
   modeled honestly.

## Risks / Trade-offs

- Signed index arithmetic is easy to get subtly wrong -> use `i64` checked
  arithmetic, arithmetic right shift, and focused tests with negative above and
  left bases.
- Logical edge ranges are less ergonomic than plain slices -> they are necessary
  to represent the spec without copying or hiding negative edge positions.
- Separate APIs add surface area -> this keeps Feature IDs and matrix rows
  honest; a later full dispatcher can wrap both APIs.
- Hardcoded derivative constants can drift as scope grows -> cite AV2 9.2 in
  code/docs and keep tests tied to the supported pAngles.
