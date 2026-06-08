# bitstream spec delta

## ADDED Requirements

### Requirement: SVLC descriptor parsing

`splot-core` SHALL parse AV2 `svlc()` descriptors using the AV2 v1.0.0 § 4.11.4 mapping
from `uvlc()` values to signed integers.

#### Scenario: zero value

- **WHEN** `uvlc()` returns `0`
- **THEN** `svlc()` SHALL return `0`.

#### Scenario: alternating signed values

- **WHEN** `uvlc()` returns `1`, `2`, `3`, or `4`
- **THEN** `svlc()` SHALL return `1`, `-1`, `2`, or `-2` respectively.

#### Scenario: truncated code

- **WHEN** the `uvlc()` prefix is truncated or has `leadingZeros >= 32`
- **THEN** `read_svlc()` SHALL return the typed parser error from `read_uvlc()` without
  panicking.

### Requirement: User-defined QM helper parsing

`splot-core` SHALL parse AV2 `user_defined_qm(level, t, plane)` (§ 5.4.11) as a shared
helper used by quantizer-matrix syntax, covering the three fundamental transform shapes
`Fundamental_Tx_Size[3] = { TX_8X8, TX_8X4, TX_4X8 }`.

#### Scenario: plane copy

- **WHEN** `plane > 0` and `qm_copy_from_previous_plane` is set
- **THEN** the parser SHALL copy the previously parsed plane matrix and return without
  reading new coefficient deltas.

#### Scenario: 4x8 transpose

- **WHEN** the `TX_4X8` matrix signals `qm_4x8_is_transpose_of_8x4`
- **THEN** the parser SHALL fill it as the transpose of the same plane's `TX_8X4` matrix.

#### Scenario: user-defined coefficients

- **WHEN** a matrix is neither copied nor transposed
- **THEN** the parser SHALL read coefficient deltas with `svlc()` in AV2 2D diagonal
  scan order and apply coefficient-repeat behavior when `quant2 == 0`.

### Requirement: Quantizer Matrix OBU parsing

`splot-core` SHALL parse `OBU_QUANTIZATION_MATRIX` payloads using AV2
`quantizer_matrix_obu()` syntax (§ 5.13) and dispatch them from `open_bitstream_unit()`.

#### Scenario: reset/default QM OBU

- **WHEN** `qm_bit_map == 0`
- **THEN** the parser SHALL record the reset/default path and SHALL NOT read per-level
  matrix payloads.

#### Scenario: user-defined QM level

- **WHEN** a level bit is set and `qm_is_default_flag == 0`
- **THEN** the parser SHALL read all `user_defined_qm(level, t, plane)` structures for
  the selected plane count.

### Requirement: Film Grain OBU parsing

`splot-core` SHALL parse `OBU_FILM_GRAIN` payloads using AV2 `film_grain_obu()` syntax
(§ 5.14) and dispatch them from `open_bitstream_unit()`.

#### Scenario: updated film-grain slot

- **WHEN** bit `i` of `fgm_update_flags` is set
- **THEN** the parser SHALL read one `film_grain_model(monochrome, subX, subY)` and
  associate it with slot `i`.

### Requirement: Film grain model parsing

`splot-core` SHALL parse `film_grain_model()` syntax (§ 5.18.10.2), preserving the
fields needed for inspection and future frame-reference checks.

#### Scenario: scaling points and AR coefficients

- **WHEN** a model has non-zero luma/chroma scaling points and a non-zero
  `ar_coeff_lag`
- **THEN** the parser SHALL read the cumulative scaling points and the de-biased AR
  coefficient arrays for the derived position counts.
