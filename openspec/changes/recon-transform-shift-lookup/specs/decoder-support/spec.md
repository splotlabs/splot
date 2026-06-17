## ADDED Requirements

### Requirement: Inverse transform Transform_Shift lookup

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.15.4 `Transform_Shift` row and column down-shift lookup, tracked by
`RECON-TRANSFORM-SHIFT-LOOKUP`. The `transform_shift` function SHALL return the
pair `(rowShift, colShift) = (Transform_Shift[txSz][0], Transform_Shift[txSz][1])`
for the `txSz` whose `(Tx_Width_Log2, Tx_Height_Log2)` equals the requested
`(log2_width, log2_height)` shape, transcribed verbatim from the § 7.15.4
constant table. Because `Transform_Shift` is a § 7.15.4 process-body constant
absent from the generated `all_tables.h` § 9 attachment, it SHALL be a
hand-written, spec-cited `splot-recon` constant rather than a `gen-tables`
output. The function SHALL return a typed `ReconError` for a `(log2_width,
log2_height)` pair that is not one of the 25 AV2 `TX_SIZES_ALL` transform shapes,
and SHALL be total and panic-free. The primitive SHALL read no frame, segment, or
tile state and SHALL NOT implement the `get_transform_1d_type` derivation, the
DPCM-direction selection, the wiring of `Transform_Shift` into the runtime decode
path, the § 7.15.3 secondary transform, or the coefficient entropy decode that
produces `Quant`.

#### Scenario: Transform_Shift lookup succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon transform_params --locked` runs
- **THEN** the test suite covers every `TX_SIZES_ALL` shape against the verbatim
  spec table, the `(log2W, log2H)` key uniqueness invariant, independently
  transcribed spec spot values, and transpose symmetry
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Non-AV2 transform shape is typed

- **WHEN** callers request a `(log2_width, log2_height)` pair that is not one of
  the 25 AV2 `TX_SIZES_ALL` transform shapes
- **THEN** `transform_shift` returns a structured `ReconError`
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Full reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the `Transform_Shift` lookup as supported
- **AND** broader reconstruction remains partial until the `get_transform_1d_type`
  derivation, the coefficient entropy decode, and the runtime transform wiring are
  implemented and proven
