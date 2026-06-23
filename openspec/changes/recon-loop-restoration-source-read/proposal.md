## Why

The active ac0ej3 loop-restoration path now retains active restoration-unit
syntax and has a scheduler-free AV2 section 7.20.2 source-sample selector in
`splot-recon`. The next narrow step is to compose that selector with immutable
frame views so later Wiener NS orchestration can request one source sample
without duplicating `CurrFrame` versus `CdefFrame` selection logic.

## What Changes

- Add Feature ID `RECON-LOOP-RESTORATION-SOURCE-READ` to the implementation
  matrix, decoder support matrix, and decoder conformance coverage group.
- Add a scheduler-free `splot-recon` helper that resolves AV2 section 7.20.2
  source coordinates and reads the selected sample from caller-supplied
  `CurrFrame` / `CdefFrame` immutable `FrameRef` views.
- Keep loop-restoration traversal, Wiener NS invocation, chroma Wiener NS,
  PC-Wiener classification, GDF, BRU, runtime wiring, and ac0ej3 output out of
  scope.

## Capabilities

### New Capabilities

- `recon-loop-restoration-source-read`: AV2 section 7.20.2 loop-restoration
  source-sample frame read over caller-resolved luma bounds, sequence
  subsampling, and immutable current/CDEF frame views.

### Modified Capabilities

- `decoder-support`: Track `RECON-LOOP-RESTORATION-SOURCE-READ` as partial
  loop-restoration reconstruction progress without claiming full loop
  restoration, runtime wiring, or ac0ej3 output.

## Impact

- Affected code: `crates/splot-recon`, decoder support/implementation tracking
  docs, generated status/coverage docs, and OpenSpec artifacts.
- Public API: one additive `splot-recon` helper and resolved sample-value type.
- Dependencies: no new dependencies and no dependency graph changes.
- Runtime behavior: no decode output change in this brick; the minimal runtime
  still rejects active ac0ej3 loop-restoration units before output.
