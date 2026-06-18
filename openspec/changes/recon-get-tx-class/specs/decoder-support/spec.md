## ADDED Requirements

### Requirement: Coefficient scan class get_tx_class

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2 § 8.3.2 `get_tx_class` transform-class derivation, tracked by `RECON-GET-TX-CLASS`. The `tx_class` function SHALL return, for a `PlaneTxType`, the `TransformClass` that selects the § 5.20.7.30 coefficient scan: the vertical-only transforms `V_DCT`, `V_ADST`, and `V_FLIPADST` to the vertical class, the horizontal-only transforms `H_DCT`, `H_ADST`, and `H_FLIPADST` to the horizontal class, and — per the spec `else` branch — every other value, including all 2D and identity transforms and any out-of-range input, to the 2D class. The function SHALL be total and panic-free over all inputs and SHALL reuse the existing `TransformClass` enum without adding an error variant. The primitive SHALL NOT implement the § 5.20.7.29 `compute_tx_type` transform-type computation that produces `PlaneTxType`, the coefficient decode loop, the wiring of the class into a decode path, or runtime decode output.

#### Scenario: get_tx_class succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon coefficient_scan --locked` runs
- **THEN** the test suite covers each vertical transform type (10, 12, 14)
  mapping to the vertical class, each horizontal transform type (11, 13, 15)
  mapping to the horizontal class, every `0..=9` value mapping to the 2D class,
  and out-of-range inputs mapping to the 2D class
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: get_tx_class is total and panic-free

- **WHEN** callers pass any `PlaneTxType`, including values outside the named
  `TX_TYPE` range
- **THEN** `tx_class` returns a `TransformClass` via the spec `else` branch
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Coefficient decode remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the `get_tx_class` transform-class derivation as
  supported
- **AND** the coefficient decode loop and broader reconstruction remain partial
  until the § 5.20.7.29 `compute_tx_type` transform-type computation, the decode
  loop, and the runtime wiring are implemented and proven
