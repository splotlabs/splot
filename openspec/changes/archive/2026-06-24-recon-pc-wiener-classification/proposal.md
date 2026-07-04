## Why

The live local decoder mission decoder frontier now resolves AV2 §7.20.4 classified-Wiener
dependency coordinates but still rejects before reading source sample values,
reading `LrTxSkip` values, and deriving `FilterClass`. The next aligned brick is
a scheduler-free `splot-recon` primitive for the §7.20.4 skip-filter
classification math that later runtime wiring can call once current/CDEF frame
storage and `LrTxSkip` state are available.

## What Changes

- Add Feature ID `RECON-PC-WIENER-CLASSIFICATION` to track the
  pixel-classified Wiener classification primitive.
- Expose generated AV2 §9.8 loop-restoration tables through the dependency-free
  `splot-tables` crate so `splot-recon` can use the normative
  `Pc_Wiener_Lut_To_Class` table without depending on `splot-core`.
- Add a scheduler-free `splot-recon` helper that implements AV2 §7.20.4
  skip-filter classification over caller-provided source samples and
  `LrTxSkip` values, returning the derived class and intermediate feature facts.
- Keep full §7.20 traversal, frame storage reads, runtime decode wiring,
  `FilterClass` frame-grid storage, §7.20.3 filtering invocation, 10-bit output,
  and successful local decoder mission decode out of scope for this brick.

## Capabilities

### New Capabilities

- `recon-pc-wiener-classification`: AV2 §7.20.4 pixel-classified Wiener
  skip-filter classification over caller-resolved source samples, `LrTxSkip`
  values, frame quantizer, and bit depth.

### Modified Capabilities

- `decoder-support`: Track `RECON-PC-WIENER-CLASSIFICATION` as narrow
  reconstruction progress without claiming runtime loop-restoration wiring or
  successful local decoder mission decode.

## Impact

- Affected code: `crates/splot-recon`, `crates/splot-tables`,
  `xtask/src/gen_tables.rs`, feature/support matrices, and generated status
  docs.
- Public API: additive `splot-recon` classification helper and parameter/result
  types; additive generated `splot-tables::tables::loop_restoration` module.
- Dependencies: no new external dependencies and no crate dependency-graph
  change.
- Runtime behavior: no decode output change in this brick; local decoder mission remains a
  structured fail-closed diagnostic until runtime storage, `LrTxSkip`, class-grid
  retention, and LR filtering are wired and verified.
