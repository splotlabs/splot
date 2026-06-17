## Why

The §5.20.7.30 `get_scan` coefficient scan order (`RECON-COEFFICIENT-SCAN-ORDER`)
selects its scan pattern from a `TransformClass`. The AV2 §8.3.2 `get_tx_class`
process derives that class from a `PlaneTxType`. It is a pure, self-contained
mapping — the companion that lets a caller pick the scan for a given transform
type — and is verifiable now against the spec definition.

## What Changes

- Add Feature ID `RECON-GET-TX-CLASS`.
- Add `tx_class(plane_tx_type)` to the existing
  `crates/splot-recon/src/coefficient_scan.rs` module, a `const fn` implementing
  §8.3.2 `get_tx_class(txType)`: the vertical-only transforms `V_DCT` (10),
  `V_ADST` (12), and `V_FLIPADST` (14) map to `TransformClass::Vertical`; the
  horizontal-only transforms `H_DCT` (11), `H_ADST` (13), and `H_FLIPADST` (15)
  map to `TransformClass::Horizontal`; and the spec `else` branch maps every other
  value (all 2D and identity transforms, and any out-of-range input) to
  `TransformClass::TwoD`.
- Reuse the existing `TransformClass` enum; add no new error variant (the mapping
  is total over all `usize` inputs via the spec `else`).
- Export `tx_class` from `lib.rs` and update the crate `//!` lists.
- Update the implementation matrix, decoder-support matrix, conformance-coverage
  group, roadmap, generated status docs, and OpenSpec artifacts.

Non-goals:

- No §5.20.7.29 `compute_tx_type` transform-type computation that produces
  `PlaneTxType`, no coefficient decode loop, no wiring into a decode path, and no
  runtime decode output.
- No new fixture and no output change — no caller exists yet.
- No reconstruction expansion, hashes, Y4M, reference refresh, public API beyond
  the new function, AVM/dav2d invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the §8.3.2 `get_tx_class` transform-class derivation
  primitive, while the coefficient decode loop and broader reconstruction remain
  partial.

## Impact

- `crates/splot-recon/src/coefficient_scan.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- `docs/DECODER-ROADMAP.md`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
