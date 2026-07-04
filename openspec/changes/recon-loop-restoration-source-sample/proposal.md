## Why

The current local decoder mission decoder frontier retains active Wiener NS loop-restoration
unit selections and still rejects before reconstruction. `splot-recon` already
has the scheduler-free luma Wiener NS tap primitive, but the surrounding AV2
section 7.20 process also needs the section 7.20.2 source-sample selector that
clips source coordinates and chooses whether a sample comes from `CurrFrame` or
`CdefFrame`.

## What Changes

- Add Feature ID `RECON-LOOP-RESTORATION-SOURCE-SAMPLE` to the implementation
  matrix, decoder support matrix, and decoder conformance coverage group.
- Add a scheduler-free `splot-recon` helper that implements AV2 section 7.20.2
  source-sample coordinate clipping and `CurrFrame` versus `CdefFrame` source
  selection over caller-resolved luma bounds.
- Keep frame storage reads, loop-restoration traversal, Wiener NS filtering,
  PC-Wiener classification, GDF, BRU, runtime wiring, and local decoder mission output out of
  scope.

## Capabilities

### New Capabilities

- `recon-loop-restoration-source-sample`: AV2 section 7.20.2 loop-restoration
  source-sample selector over caller-resolved luma bounds and sequence
  subsampling.

### Modified Capabilities

- `decoder-support`: Track `RECON-LOOP-RESTORATION-SOURCE-SAMPLE` as partial
  loop-restoration reconstruction progress without claiming full loop
  restoration, frame reads, local decoder mission decode, or runtime wiring.

## Impact

- Affected code: `crates/splot-recon`, decoder support/implementation tracking
  docs, generated status/coverage docs, and OpenSpec artifacts.
- Public API: one additive `splot-recon` helper, bounds type, resolved sample
  type, and source enum.
- Dependencies: no new dependencies and no dependency graph changes.
- Runtime behavior: no decode output change in this brick; the minimal runtime
  still rejects active local decoder mission loop-restoration units before output.
