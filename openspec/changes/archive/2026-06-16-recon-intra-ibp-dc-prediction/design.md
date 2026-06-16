## Context

Before this change, `splot-recon` supported AV2 §7.13.2.10 square and
rectangular DC prediction, §7.13.2.11 subsampled DC prediction, §7.13.2.2
basic/PAETH prediction, §7.13.2.13 smooth prediction, and the cardinal pAngle
90/180 subset of §7.13.2.8 directional prediction. AV2 §7.13.2.12 IBP DC is
still missing from the scalar prepared-edge reconstruction primitives.

This change is source-backed reconstruction work only. It does not parse tile
syntax, decide the §7.13.2.1 IBP invocation predicate, or expand the runtime
`splot decode` tier.

## Goals / Non-Goals

**Goals:**

- Add `RECON-INTRA-IBP-DC-PREDICTION` as a supported source-backed primitive
  row.
- Implement a scheduler-free `splot-recon` modifier for AV2 §7.13.2.12 over an
  existing caller-owned DC prediction buffer and prepared `LeftCol[0..h]` /
  `AboveRow[0..w]` samples.
- Use AV2 §3 `IBP_WEIGHT_MAX` and `IBP_WEIGHT_SHIFT`, AV2 §4.8 `Round2`, and the
  §7.13.2.12 `Ibp_Weights` table directly in local Rust code derived from the
  spec mirror.
- Add a current-frame workspace helper that can write DC prediction and then
  apply IBP DC from in-storage neighboring edges without inventing AV2 edge
  availability.
- Extend the existing recon intra fuzz target and status docs.

**Non-Goals:**

- No full `predict_intra()` dispatcher or runtime §7.13.2.1 gate evaluation.
- No generalized directional-mode IBP or §7.13.2.9 dynamic IBP weights process.
- No data-driven prediction, CfL/CCTX/MHCCP, palette, edge filtering, transform,
  residual, loop filtering, film grain, reference refresh, or broad runtime
  decode support.
- No AVM/dav2d integration, new dependencies, dependency graph changes, or
  copied reference code.

## Decisions

1. Put the primitive in a new `splot-recon` module.
   `splot-recon` owns decoded-frame storage and scalar reconstruction
   primitives. A dedicated `intra_ibp_dc.rs` keeps `intra.rs` from growing and
   mirrors the recent `intra_dc_subsampled.rs` and `intra_directional.rs`
   structure.

2. Model IBP DC as a modifier over caller-owned prediction storage.
   AV2 §7.13.2.12 modifies an existing DC `pred` array. The direct API should
   therefore take a mutable strided output buffer, validate shape and edges, and
   modify only the affected rows/columns. It should not compute the DC average
   itself or allocate an owned prediction block.

3. Reuse the existing DC edge type for prepared left/above edges.
   `IntraDcEdges<'_, T>` already models optional prepared `LeftCol[0..h]` and
   `AboveRow[0..w]` inputs with typed validation. Reusing it avoids a duplicate
   public edge type for the same prepared-edge shape.

4. Keep workspace semantics in-storage and no-dispatch.
   The workspace helper should first call the existing rectangular DC helper
   behavior, then apply IBP DC using in-storage edges when present. It may leave
   a no-edge top-left block as normal DC midpoint, but it must not decide
   `enable_ibp`, `useDip`, mode, chroma/CfL exceptions, tile boundaries, or
   fallback sample preparation.

5. Extend `recon_intra_prediction_bytes` instead of adding a new fuzz target.
   The existing target already normalizes arbitrary bytes into bounded direct
   and workspace intra-prediction cases. Adding an IBP DC branch keeps fuzz
   coverage self-contained and avoids redundant CI target wiring.

## Risks / Trade-offs

- The `Ibp_Weights` table is a spec-defined numeric table. Mitigation: encode
  only the small normative AV2 numeric table as implementation data from the
  local mirror, cite §7.13.2.12 in docs and matrix rows, and keep
  third-party/reference code, comments, and prose out.
- IBP DC support could be mistaken for full IBP or full intra dispatch.
  Mitigation: narrow names, OpenSpec non-goals, matrix notes, and unchanged
  partial broad rows.
- The overlap skip rules differ for tall, wide, and square blocks. Mitigation:
  add focused tests for above-only, left-only, tall both-edge, wide/square
  both-edge behavior, and the no-op no-edge case.
- Workspace helper semantics for missing edges can be overinterpreted.
  Mitigation: document that workspace edge availability is storage-local only
  and not full §7.13.2.1 fallback preparation.

## Planning Review Notes

- `@spec-mapper` confirmed this is a narrow source-backed slice when scoped to
  AV2 §7.13.2.12 DC prediction modification, with §7.13.2.1 dispatch gates,
  §7.13.2.10 base DC prediction, §3 IBP constants, and §4.8 `Round2` cited only
  as context. The review explicitly excludes §7.13.2.9 directional IBP weights
  from this change.
- `@decoder-architect` recommended a new `intra_ibp_dc.rs` primitive module and
  a separate workspace extension module so existing large files stay below the
  source-line budget. The API shape should be an in-place direct modifier plus a
  workspace helper that writes base DC then applies IBP DC.
- `@security` required pre-mutation validation of sample type, edge lengths,
  edge sample ranges, output shape, and every `pred` sample that may be read.
  The workspace helper should extract any edge scratch before writing base DC,
  and the implementation should avoid full-block temporary allocations.
