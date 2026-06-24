# recon-pc-wiener-classification Specification

## Purpose
Define the scheduler-free `splot-recon` primitive for AV2 §7.20.4
pixel-classified Wiener skip-filter classification, without claiming runtime
loop-restoration wiring or successful decode output.

## Requirements

### Requirement: PC-Wiener classification primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
AV2 §7.20.4 pixel-classified Wiener skip-filter classification process, tracked
by `RECON-PC-WIENER-CLASSIFICATION`. The primitive SHALL accept caller-resolved
source samples, caller-resolved `LrTxSkip` values, active bit depth, and
`base_q_idx`; it SHALL evaluate the 6x6 feature window, the seven
second-derivative source reads per feature point, feature normalization, the
`get_qval_given_tskip` contribution, 3-bit feature quantization, `lutInput`
construction, and the normative `Pc_Wiener_Lut_To_Class` lookup. The primitive
SHALL return the derived class and intermediate feature facts. The caller SHALL
resolve §7.20.2 source selection, frame/restoration-unit traversal,
`BlockStartX`/`BlockEndX` clipping, stripe/tile clipping, and `LrTxSkip` grid
storage. The primitive SHALL NOT implement runtime decode wiring, `FilterClass`
grid storage, `SubclassLookup` derivation, §7.20.3 filter invocation, frame
storage, `LrTxSkip` derivation, or ac0ej3 output.

#### Scenario: Classification math is covered by focused tests

- **WHEN** `cargo test -p splot-recon pc_wiener --locked` runs
- **THEN** the test suite covers flat-source classification, hand-computed
  feature accumulation, `LrTxSkip` quantizer contribution, 8-bit and 10-bit
  normalization, and source samples outside the active bit-depth range
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Normative tables are shared without dependency inversion

- **WHEN** `cargo xtask gen-tables --check` and
  `cargo xtask check-dependency-direction` run
- **THEN** the generated AV2 §9.8 loop-restoration table module is available to
  `splot-recon` through `splot-tables`
- **AND** `splot-recon` does not depend on `splot-core`
- **AND** existing `splot-core` loop-restoration table consumers continue to
  build without a new `splot-core` dependency on `splot-tables`

#### Scenario: Invalid inputs are rejected without mutation

- **WHEN** the classifier is called with unsupported sample storage for the
  active bit depth, a source sample outside the active bit-depth range, or an
  `LrTxSkip` value outside the §7.20.4 boolean skip domain
- **THEN** it returns a typed `ReconError`
- **AND** the primitive mutates no caller-owned output or grid state
