## Why

The decoder reconstruction frontier needs the residual path after intra
prediction: dequantization, inverse transforms, and residual addition
(`docs/DECODER-SUPPORT-MATRIX.toml`). AV2 § 7.14.2 defines the quantizer-value lookup
that every dequantized coefficient depends on, and it is small, pure,
table-driven, and scheduler-free — the natural first residual-path primitive in
one PR, directly reusable by future decode and encoder reconstruction.

## What Changes

- Add Feature ID `RECON-DEQUANT-QUANTIZER-LOOKUP`.
- Add a scheduler-free `splot-recon` AV2 § 7.14.2 dequantization
  quantizer-value lookup core: the 25-entry `Ac_Qlookup` base table, the
  `qlookup` shift-extension function, `max_quantizer_index` deriving the
  § 6.4.1 Table 6.3 `MaxQ`, and `quantizer_value` implementing § 7.14.2
  `get_q( qindex, delta )`.
- Take caller-resolved inputs (resolved quantizer index, signed delta, active
  bit depth); make every input total and panic-free with `i64` clamp
  intermediates.
- Preserve the current runtime `splot decode` unsupported behavior.
- Add self-contained spec-exact unit tests.
- Update decoder support, feature tracking, roadmap, generated status docs, and
  OpenSpec artifacts.

Non-goals:

- No § 7.14.2 `get_qindex` segment / `delta_q` index resolution.
- No per-plane `get_dc_quant` / `get_ac_quant` composition.
- No § 7.14.4 dequantization process, quantizer-matrix weighting, or § 7.14.3
  reconstruct process.
- No inverse transforms, residual addition, tile-syntax decode, runtime decode
  output, hashes, Y4M, or reference refresh.
- No `splot-decode -> splot-recon` dependency change and no scheduler state in
  `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records source-backed AV2 § 7.14.2 dequantization
  quantizer-value lookup support while broader reconstruction (the full
  dequantization process, inverse transforms, and residual addition) remains
  partial.

## Impact

- `crates/splot-recon/src/dequant.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `openspec/specs/decoder-support/spec.md`
