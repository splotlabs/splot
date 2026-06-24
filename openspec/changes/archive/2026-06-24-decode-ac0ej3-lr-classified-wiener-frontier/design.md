## Context

`DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER` proved §7.20.3 source-read coordinate
selection for non-classified Wiener NS LR paths and deliberately stopped the
local ac0ej3 luma path before §7.20.4. The ac0ej3 key frame uses frame-level
luma Wiener NS with `NumFilterClasses > 1`, so §7.20.1 invokes the
pixel-classified Wiener process with `skipFilter == 1` before §7.20.3 filtering.

The current runtime still has no decoded 10-bit current/CDEF frame storage and
does not retain `LrTxSkip` values. This change can therefore only model the
coordinate dependencies of classification, not the class values or filtered
samples.

## Goals / Non-Goals

**Goals:**

- Add a distinct `DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-FRONTIER` row.
- Retain tile MI bounds on active LR source blocks for §7.20.4 `BlockEndX`,
  `MiRowStart`, `MiRowEnd`, and `get_tx_skip` clipping.
- Enumerate the §7.20.4 `get_box_features` window for skip-filter
  classification: 36 feature points, 7 luma `get_source_sample` calls per point,
  and one clipped `LrTxSkip` lookup per point.
- Continue resolving the later §7.20.3 source-read coordinate frontier, then
  fail closed before source sample values, `LrTxSkip` values, `FilterClass`, or
  LR output writes.

**Non-Goals:**

- Computing feature values, qvals, lookup-table class values, or `FilterClass`.
- Applying §7.20.3 Wiener NS filtering or §7.20.4 PC-Wiener filtering.
- Adding frame-buffer allocation, 10-bit sample storage, reference refresh, raw
  or Y4M output, new dependencies, or public APIs.

## Decisions

- Keep classification as runtime dependency derivation, not reconstruction. The
  diagnostic remains the product until real source buffers and tx-skip state are
  available.
- Reuse `splot_recon::loop_restoration_source_sample` for the §7.20.2 source
  selector instead of duplicating clipping/source-selection rules in
  `splot-decode`.
- Store tile MI bounds on `WienerNsLrSourceBlock`; deriving them later from
  frame-wide source bounds would be incorrect when loopfilters are allowed
  across tiles because §7.20.4 still uses `MiColEnd`, `MiRowStart`, and
  `MiRowEnd`.
- Preflight the combined classified-luma and §7.20.3 source-read count against
  `MaxLoopRestorationSourceReads` before runtime coordinate enumeration.

## Risks / Trade-offs

- Overclaiming classification support -> Mitigation: diagnostics, tests, and the
  matrix explicitly say dependency/source-read frontier only.
- Large active-block lists -> Mitigation: exact source-read count is checked
  before enumerating coordinates.
- Tile-bound off-by-one errors -> Mitigation: focused tests cover `BlockEndX`,
  first source selection, and `get_tx_skip` clipping.
