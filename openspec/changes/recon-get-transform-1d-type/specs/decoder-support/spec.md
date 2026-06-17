## ADDED Requirements

### Requirement: Inverse transform get_transform_1d_type derivation

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.15.4 `get_transform_1d_type` row and column transform-type derivation, tracked
by `RECON-GET-TRANSFORM-1D-TYPE`. The `get_transform_1d_type` function SHALL
return `Transform_1d_Type[PlaneTxType][dir]` (as the `InverseTransform2dDim` the
2D inverse transform consumes, with `IDT` mapped to `Identity` and the kernel
types to `Kernel`), and SHALL apply the spec `useDdt` substitution: when the
caller-resolved `use_ddt` is set, the base type is `ADST` or `FDST`, and the pass
size is not 4, the type SHALL be replaced by `DDTX` or `FDDT` respectively.
Because `Transform_1d_Type` is a § 7.15.4 process-body constant absent from the
generated `all_tables.h` § 9 attachment, it SHALL be a hand-written, spec-cited
`splot-recon` constant rather than a `gen-tables` output. The function SHALL
return a typed `ReconError` for a `PlaneTxType` outside the `TX_TYPES` range
(`0..16`), and SHALL be total and panic-free. The primitive SHALL read no frame,
segment, or tile state beyond its caller-resolved arguments and SHALL NOT
implement the DPCM-direction selection, the combined transform-parameter resolve
helper, the wiring of `get_transform_1d_type` into the runtime decode path, the
§ 7.15.3 secondary transform, or the coefficient entropy decode that produces
`Quant`.

#### Scenario: get_transform_1d_type succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon transform_params --locked` runs
- **THEN** the test suite covers every `PlaneTxType` row against the verbatim spec
  `Transform_1d_Type` table for both passes, the `useDdt` substitution (eligible
  and ineligible cases), and the out-of-range rejection
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid PlaneTxType is typed

- **WHEN** callers request a `PlaneTxType` outside the `TX_TYPES` range
- **THEN** `get_transform_1d_type` returns a structured `ReconError`
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Full reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the `get_transform_1d_type` derivation as supported
- **AND** broader reconstruction remains partial until the DPCM-direction
  selection, the runtime transform wiring, and the coefficient entropy decode are
  implemented and proven
