## Context

`DECODE-AC0EJ3-LR-SOURCE-BOUNDS-FRONTIER` retained active AV2 §7.20.1
source-bound facts for ac0ej3-proven frame-level Wiener NS loop-restoration
units. The next frontier needs to stop classified luma honestly before the
§7.20.4 pixel-classified Wiener process, and otherwise enumerate the §7.20.3
source-selection calls known at this boundary, resolve them through the §7.20.2
source sample selector, then fail closed before frame-buffer value reads,
§7.20.3 Wiener NS filtering, or any output path can run.

The required source selection primitive already lives in `splot-recon`:
`loop_restoration_source_sample` resolves whether a coordinate selects
`CurrFrame` or `CdefFrame`; `loop_restoration_source_sample_value` remains a
later frame-buffer read step. This brick wires only the selection primitive far
enough to prove the runtime handoff reaches supported output-sample center,
Wiener tap, and chroma luma-source sample selection transactionally.

## Goals / Non-Goals

**Goals:**

- Add a distinct `DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER` matrix/support row.
- Validate §7.20.2 source sample selections for retained active source-bound
  facts using the existing `splot-recon` source-selection primitive.
- Gate active classified luma before source-read derivation when §7.20.1 invokes
  §7.20.4 first.
- Preserve fail-closed behavior: the local ac0ej3 fixture must still stop before
  filtering, decoded-frame allocation, reference refresh, hash, raw, or Y4M
  output.
- Keep source-read state transactional and bounded by a dedicated decode limit.

**Non-Goals:**

- Applying §7.20.3 luma or chroma Wiener NS filters.
- §7.20.4/PC-Wiener classification, switchable LR, temporal/reference Wiener state,
  GDF/CDEF/deblock orchestration, 10-bit output, or successful ac0ej3 decode.
- Adding public APIs, dependencies, or scheduler behavior.

## Decisions

- Reuse the `splot-recon` source-selection primitive instead of duplicating
  §7.20.2 selection logic in `splot-decode`. This keeps coordinate clipping and
  source selection centralized in the reconstruction crate.
- Enumerate the §7.20.3 source-selection calls that are known at this boundary:
  output sample centers, Wiener tap coordinates, and chroma luma-source
  coordinates. Coefficient-dependent filtering remains unsupported.
- Reject active luma with `NumFilterClasses > 1` before source-read derivation,
  because §7.20.1 invokes §7.20.4 before the non-separable Wiener process.
- Charge source-read enumeration to `max_loop_restoration_source_reads`, not to
  `max_luma_samples_per_frame`, because chroma and luma-source reads can exceed
  the coded luma sample count for valid frames.
- Keep the runtime diagnostic as the product of this brick. The new diagnostic
  should either prove the source-read boundary was reached for unclassified
  blocks or name the earlier classified-luma boundary, then explicitly name the
  later filtering surface as unsupported.
- Do not fabricate frame-buffer reads. Because the current ac0ej3 path cannot yet
  provide reconstructed 10-bit-compatible current/CDEF source buffers, the brick
  must retain a precise fail-closed diagnostic rather than reading sample values.
- Do not expose decoded Wiener NS coefficients or filtered samples from the tile
  traversal frontier. Coefficient syntax and complete tap/luma-source reads are
  prerequisites for filtering, not a filtering claim.

## Risks / Trade-offs

- Source reads need frame buffers that the current ac0ej3 path may not allocate
  because it still fails before output. Mitigation: keep the implementation
  narrow and fail closed with a precise source-read/filter boundary diagnostic.
- It is easy to overclaim this as loop-restoration reconstruction. Mitigation:
  matrices, OpenSpec, and diagnostics must say source selection/read state only;
  sample value reads, filtering, and output remain unsupported.
- The active source block list can be large. Mitigation: use the dedicated
  source-read operation limit and keep the retained/read evidence root-frontier
  scoped.
