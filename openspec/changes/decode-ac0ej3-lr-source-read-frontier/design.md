## Context

`DECODE-AC0EJ3-LR-SOURCE-BOUNDS-FRONTIER` retained active AV2 §7.20.1
source-bound facts for ac0ej3-proven frame-level Wiener NS loop-restoration
units. The next frontier needs to reach §7.20.2 source sample selection and
validate the block-center selector path, then fail closed before full Wiener tap
reads, chroma luma-source reads, §7.20.3 Wiener NS filtering, or any output path
can run.

The required source selection primitive already lives in `splot-recon`:
`loop_restoration_source_sample` resolves whether a coordinate selects
`CurrFrame` or `CdefFrame`; `loop_restoration_source_sample_value` remains a
later frame-buffer read step. This brick wires only the selection primitive far
enough to prove the runtime handoff reaches supported center source sample
selection transactionally.

## Goals / Non-Goals

**Goals:**

- Add a distinct `DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER` matrix/support row.
- Validate §7.20.2 block-center source sample selections for retained active
  source-bound facts using the existing `splot-recon` source-selection
  primitive.
- Preserve fail-closed behavior: the local ac0ej3 fixture must still stop before
  filtering, decoded-frame allocation, reference refresh, hash, raw, or Y4M
  output.
- Keep source-read boundary state transactional without applying a luma-sample
  limit to accumulated multi-plane source-read evidence.

**Non-Goals:**

- Applying §7.20.3 luma or chroma Wiener NS filters.
- PC-Wiener classification, switchable LR, temporal/reference Wiener state,
  GDF/CDEF/deblock orchestration, 10-bit output, or successful ac0ej3 decode.
- Adding public APIs, dependencies, or scheduler behavior.

## Decisions

- Reuse the `splot-recon` source-selection primitive instead of duplicating
  §7.20.2 selection logic in `splot-decode`. This keeps coordinate clipping and
  source selection centralized in the reconstruction crate.
- Keep the runtime diagnostic as the product of this brick. The new diagnostic
  should prove the source-read boundary was reached, then explicitly name the
  later tap/luma-source read and filtering surfaces as unsupported.
- Do not fabricate frame-buffer reads. Because the current ac0ej3 path cannot yet
  provide reconstructed 10-bit-compatible current/CDEF source buffers, the brick
  must retain a precise fail-closed diagnostic rather than reading sample values.
- Do not expose decoded Wiener NS coefficients or filtered samples from the tile
  traversal frontier. Coefficient syntax and complete tap/luma-source reads are
  prerequisites for filtering, not a filtering claim.

## Risks / Trade-offs

- Complete source reads need frame buffers that the current ac0ej3 path may not
  allocate because it still fails before output. Mitigation: keep the
  implementation narrow and fail closed with a precise source-read/filter
  boundary diagnostic.
- It is easy to overclaim this as loop-restoration reconstruction or complete
  source-read resolution. Mitigation: matrices, OpenSpec, and diagnostics must
  say source-read boundary only; full tap/luma-source reads, filtering, and output
  remain unsupported.
- The active source block list can be large. Mitigation: reuse existing LR-unit
  traversal limits and keep the retained selection evidence root-frontier scoped.
