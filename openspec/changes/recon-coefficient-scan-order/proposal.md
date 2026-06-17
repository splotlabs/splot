## Why

The §5.20.7 coefficient decode loop (which produces `Quant`) and the §7.14.4
coefficient placement both need the AV2 §5.20.7.30 `get_scan` coefficient scan
order. It is a pure, self-contained permutation — a prerequisite that unblocks the
coefficient decode loop — and is verifiable now against the spec algorithm.

## What Changes

- Add Feature ID `RECON-COEFFICIENT-SCAN-ORDER`.
- Add `coefficient_scan_order(w, h, class, out)` to a new
  `crates/splot-recon/src/coefficient_scan.rs` module, implementing §5.20.7.30
  `get_scan(txSz, txClass)`: it writes the scan order (each `out[c]` is the
  flattened `y*w + x` position of the c-th scanned coefficient) for `TX_CLASS_VERT`
  (row-major raster), `TX_CLASS_HORIZ` (column-major transpose), and `TX_CLASS_2D`
  (the anti-diagonal scan).
- Add the `TransformClass` enum (the spec `txClass`).
- Add typed `ReconError::InvalidScanShape` and `ReconError::ScanLengthMismatch`.
- Block shape is caller-resolved (`w = Min(Tx_Width[txSz], 32)` /
  `h = Min(Tx_Height[txSz], 32)`, each 4/8/16/32), consistent with the §7.15
  transforms, since `splot-recon` cannot reach `splot-core`'s §9.2 tables; scan
  positions fit `u16` (max `w*h-1 <= 1023`).
- Update the implementation matrix, decoder-support matrix, conformance-coverage
  group, roadmap, generated status docs, and OpenSpec artifacts.

Non-goals:

- No `get_tx_class` (PlaneTxType -> txClass), no coefficient decode loop, no
  wiring into a decode path, no §7.15.3 secondary-transform `Stx_Scan`, and no
  runtime decode output.
- No new fixture and no output change — no caller exists yet.
- No reconstruction expansion, hashes, Y4M, reference refresh, public API beyond
  the new function, AVM/dav2d invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the §5.20.7.30 `get_scan` coefficient scan order
  primitive, while the coefficient decode loop and broader reconstruction remain
  partial.

## Impact

- `crates/splot-recon/src/coefficient_scan.rs` (new)
- `crates/splot-recon/src/error.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- `docs/DECODER-ROADMAP.md`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
