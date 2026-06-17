## ADDED Requirements

### Requirement: Dequantization quantization-matrix weighting

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.14.4 step-2 built-in quantization-matrix weighting, tracked by
`RECON-DEQUANT-QM-WEIGHT`. The `quantization_matrix_weight` function SHALL return
the weight `m = Quantizer_Matrix[segLvl][plane > 0][Qm_Offset[txSz] + i * tw + j]`
from the generated § 9.4 tables, returning a typed `ReconError` when `segLvl`,
`txSz`, or the derived position is out of range. The `qm_weighted_quantizer`
function SHALL compute the § 7.14.4 step-2 weighted quantizer
`q2 = Round2(q * m, 5)`, total and panic-free (widened intermediate, clamped into
`u32`). To make the § 9.4 tables consumable by `splot-recon` without a dependency
on `splot-core`, the generated § 9.4 `quantizer` table module SHALL be emitted
into the dependency-free `splot-tables` crate (the same `output_dir_for` routing
used for the § 9.6 / § 9.7 kernels), with no change to the generated table
contents. The primitive SHALL read no frame, segment, or tile state and SHALL NOT
implement the `useQm` / `useUserQm` / `segLvl` gating, the user-defined `UserQm`
matrices, the `shift` / `useFsc` derivation, the coefficient entropy decode that
produces `Quant`, the § 7.15.4 inverse transform, tile syntax traversal, runtime
decode output, or reference-refresh semantics.

#### Scenario: QM weighting succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon dequant_process --locked` runs
- **THEN** the test suite covers the `Round2(q * m, 5)` weighting (including the
  totality extreme) and the built-in `Quantizer_Matrix` lookup (luma and chroma
  planes and an offset position) against the generated table
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Relocated quantizer tables stay byte-identical

- **WHEN** `cargo xtask gen-tables --check` runs after the § 9.4 module is routed
  to `splot-tables`
- **THEN** the regenerated tables match the committed files with no drift, and the
  236-table determinism count is unchanged
- **AND** `cargo xtask check-dependency-direction` confirms `splot-tables` remains
  a dependency-free leaf and `splot-recon` depends only on it

#### Scenario: Invalid QM index is typed

- **WHEN** callers request a `seg_level`, `tx_size`, or coefficient position
  outside the generated `Quantizer_Matrix`
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Full reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the quantization-matrix weighting as supported
- **AND** broader reconstruction remains partial until the `useQm` / `UserQm`
  gating, the coefficient entropy decode, and the § 7.15.4 inverse-transform
  invocation are implemented and proven
