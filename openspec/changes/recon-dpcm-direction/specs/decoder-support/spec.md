## ADDED Requirements

### Requirement: Inverse transform DPCM direction selection

The repository SHALL provide a scheduler-free `splot-recon` helper that selects
the AV2 § 7.15.4 DPCM cumulative-sum direction for a transform block, tracked by
`RECON-DPCM-DIRECTION`. The `dpcm_direction(use_dpcm, mode_is_v_pred) ->
Option<DpcmDirection>` function SHALL return `None` when `use_dpcm` is false,
`DpcmDirection::Vertical` when `use_dpcm` is true and `mode_is_v_pred` is true,
and `DpcmDirection::Horizontal` when `use_dpcm` is true and `mode_is_v_pred` is
false, where `use_dpcm` is the § 7.15.4 plane-selected `useDpcm` flag and
`mode_is_v_pred` is whether the plane-selected prediction `mode` equals `V_PRED`.
It SHALL be a total `const fn` with no error path, SHALL read no frame, segment,
or tile state, and the result SHALL be usable as the `dpcm` field of
`InverseTransform2dOuter`. The helper SHALL NOT implement the runtime resolution
of `use_dpcm_y`, `use_dpcm_uv`, or the prediction mode, any wiring into the
runtime decode path, the § 7.15.3 secondary transform, or the coefficient entropy
decode that produces `Quant`.

#### Scenario: DPCM direction maps the four spec cases

- **WHEN** `cargo test -p splot-recon transform_params --locked` runs
- **THEN** the test suite proves `dpcm_direction` returns `None` whenever
  `use_dpcm` is false, `Vertical` for `(true, true)`, and `Horizontal` for
  `(true, false)`
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Selected direction drives the outer transform

- **WHEN** an `InverseTransform2dOuter` resolved with `dpcm = dpcm_direction(true,
  true)` is applied to a lossless IDTX block
- **THEN** the § 7.15.4 column cumulative sum runs, turning a flat pre-DPCM
  residual into the per-column `1, 2, 3, 4` sequence, while `dpcm = dpcm_direction(false,
  _)` leaves the residual flat
