## ADDED Requirements

### Requirement: 2D inverse transform parameter resolve helper

The repository SHALL provide a scheduler-free `splot-recon` helper that resolves
the AV2 § 7.15.4 2D inverse transform parameter set from a single transform-block
fact source, tracked by `RECON-RESOLVE-2D-TRANSFORM-PARAMS`. The
`InverseTransform2dOuter::resolve(plane_tx_type, log2_width, log2_height,
use_ddt, lossless, bit_depth, dpcm)` associated function SHALL derive `row_shift`
and `col_shift` from `Transform_Shift[txSz]` via `transform_shift(log2_width,
log2_height)`, SHALL derive `row_type` and `col_type` via `get_transform_1d_type`
over the adjusted per-pass sample sizes `1 << Min(log2_width, 5)` and
`1 << Min(log2_height, 5)` (the § 7.15.4.1 `rowType = get_transform_1d_type(0, w)`
and `colType = get_transform_1d_type(1, h)`), and SHALL set
`plane_tx_type_is_idtx` from `PlaneTxType == IDTX (9)`, storing the same
`log2_width`, `log2_height`, `lossless`, `bit_depth`, and `dpcm` it was given. It
SHALL validate the shape via `transform_shift` before any adjusted-size
arithmetic and the type via `get_transform_1d_type`, returning
`ReconError::InvalidTransformShiftShape` or `ReconError::InvalidPlaneTxType`
without resolving partial parameters. The helper SHALL be a total, panic-free
`const fn`, SHALL read no frame, segment, or tile state, and SHALL NOT implement
the § 7.15.4 DPCM-direction selection from the prediction mode, any wiring into
the runtime decode path, the § 7.15.3 secondary transform, the § 5.20.7.29
`compute_tx_type` that produces `PlaneTxType`, or the coefficient entropy decode
that produces `Quant`.

#### Scenario: Resolve composes the shift and type derivations

- **WHEN** `cargo test -p splot-recon inverse_transform_2d_outer --locked` runs
- **THEN** the test suite proves `resolve` threads the original log2 dims into
  `transform_shift` and the adjusted per-pass sizes into `get_transform_1d_type`
  for several `PlaneTxType` / shape / `use_ddt` combinations, and that the
  resolved params drive `inverse_transform_2d_outer` identically to a hand-built
  params struct
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: DDT substitution keys off the per-pass adjusted size

- **WHEN** a TX_8X4 block with an `ADST_ADST` `PlaneTxType` resolves with
  `use_ddt` set
- **THEN** the row pass (adjusted size 8) substitutes `ADST` with `DDTX` while
  the column pass (adjusted size 4) keeps `ADST`, because the `sz != 4` guard
  uses each pass's adjusted size

#### Scenario: Invalid resolve input is typed and fail-atomic

- **WHEN** `resolve` is called with a non-`TX_SIZES_ALL` log2 shape or an
  out-of-range `PlaneTxType`
- **THEN** it returns `ReconError::InvalidTransformShiftShape` or
  `ReconError::InvalidPlaneTxType` respectively and resolves no partial
  parameters
- **AND** a sweep over every log2 dimension in `0..=8` and every `PlaneTxType` in
  `0..18` either resolves cleanly or returns one of those typed errors, never
  panicking
