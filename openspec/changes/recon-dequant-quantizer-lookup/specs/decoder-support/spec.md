## ADDED Requirements

### Requirement: Dequantization quantizer-value lookup primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
AV2 § 7.14.2 dequantization quantizer-value lookup, tracked by
`RECON-DEQUANT-QUANTIZER-LOOKUP`. The primitive SHALL provide the 25-entry
`Ac_Qlookup` base table, the `qlookup` shift-extension function, a
`max_quantizer_index` function that derives the AV2 § 6.4.1 Table 6.3 `MaxQ`
(8-bit decoded output uses 255 and 10-bit decoded output uses 303, the only
decoded bit depths AV2 v1.0.0 defines), and a `quantizer_value` function that
implements § 7.14.2 `get_q( qindex, delta )` by returning `Ac_Qlookup[0]` when
the resolved quantizer index is 0 and the signed delta is non-positive and
otherwise returning `qlookup` of the index plus delta clamped to `1..=MaxQ`.
The primitive SHALL take caller-resolved inputs, SHALL be total and panic-free
for every input by using widened clamp intermediates, and SHALL read no frame,
segment, or tile state. The primitive SHALL NOT implement § 7.14.2 `get_qindex`
index resolution, the per-plane `get_dc_quant` / `get_ac_quant` composition,
the § 7.14.4 dequantization process, quantizer-matrix weighting, the § 7.14.3
reconstruct process, inverse transforms, residual addition, tile syntax
traversal, runtime decode output, or reference-refresh semantics.

#### Scenario: Quantizer lookup succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon dequant --locked` runs
- **THEN** the test suite covers the `Ac_Qlookup` table, the `qlookup` shift
  extension at the 8-bit and 10-bit `MaxQ` extremes, the qindex-0 special case,
  delta addition, and both `1..=MaxQ` clamp directions
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Quantizer lookup is total and panic-free

- **WHEN** callers pass any resolved quantizer index, signed delta, and active
  bit depth, including out-of-contract extremes
- **THEN** `splot-recon` returns a clamped quantizer value computed with
  widened intermediates
- **AND** library code does not panic, overflow, unwrap, or emit `decode/*`
  diagnostics

#### Scenario: Full dequantization remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the dequantization quantizer-value lookup as
  supported
- **AND** broader reconstruction remains partial until § 7.14.2 `get_qindex`
  resolution, the per-plane `get_dc_quant` / `get_ac_quant` composition, the
  § 7.14.4 dequantization process, quantizer-matrix weighting, inverse
  transforms, and residual addition are implemented and proven
