## Context

`DECODE-AC0EJ3-LR-SOURCE-BOUNDS-FRONTIER` retained active AV2 §7.20.1
source-bound facts for ac0ej3-proven frame-level Wiener NS loop-restoration
units. The next frontier needs to resolve those facts into §7.20.2 source
sample selections against the current frame and CDEF frame buffers, then fail
closed before any §7.20.3 Wiener NS filtering or output path can run.

The required source selection primitives already live in `splot-recon`:
`loop_restoration_source_sample` resolves whether a coordinate reads `CurrFrame`
or `CdefFrame`, and `loop_restoration_source_sample_value` reads the selected
sample from caller-provided immutable frame views. This brick wires those
primitives only far enough to prove the runtime handoff can read supported
source samples transactionally.

## Goals / Non-Goals

**Goals:**

- Add a distinct `DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER` matrix/support row.
- Resolve §7.20.2 source sample selections for retained active source-bound
  facts using the existing `splot-recon` source-read primitives.
- Preserve fail-closed behavior: the local ac0ej3 fixture must still stop before
  filtering, decoded-frame allocation, reference refresh, hash, raw, or Y4M
  output.
- Keep source-read state transactional and bounded by existing decode limits.

**Non-Goals:**

- Applying §7.20.3 luma or chroma Wiener NS filters.
- PC-Wiener classification, switchable LR, temporal/reference Wiener state,
  GDF/CDEF/deblock orchestration, 10-bit output, or successful ac0ej3 decode.
- Adding public APIs, dependencies, or scheduler behavior.

## Decisions

- Reuse `splot-recon` source-read primitives instead of duplicating §7.20.2
  selection logic in `splot-decode`. This keeps coordinate clipping and source
  selection centralized in the reconstruction crate.
- Keep the runtime diagnostic as the product of this brick. The new diagnostic
  should prove source reads were reached, then explicitly name the later
  filtering boundary as unsupported.
- Read only from existing immutable current/CDEF frame views supplied by the
  runtime. If the current ac0ej3 path cannot yet provide a reconstructed
  10-bit-compatible source buffer, the brick must retain a precise fail-closed
  diagnostic rather than fabricating samples.
- Do not expose decoded Wiener NS coefficients or filtered samples from the tile
  traversal frontier. Coefficient syntax and source reads are prerequisites for
  filtering, not a filtering claim.

## Risks / Trade-offs

- Source reads need frame buffers that the current ac0ej3 path may not allocate
  because it still fails before output. Mitigation: keep the implementation
  narrow and fail closed with a precise source-read/filter boundary diagnostic.
- It is easy to overclaim this as loop-restoration reconstruction. Mitigation:
  matrices, OpenSpec, and diagnostics must say source reads only; filtering and
  output remain unsupported.
- The active source block list can be large. Mitigation: reuse existing resource
  limits and keep the retained/read evidence root-frontier scoped.
