## ADDED Requirements

### Requirement: Secondary inverse transform process

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.15.3 secondary inverse transform, tracked by
`RECON-SECONDARY-INVERSE-TRANSFORM`. The `secondary_inverse_transform(dequant,
params)` function SHALL, over a `w * h` row-major `dequant` block, gather the
first `params.n` coefficients in § 5.20.7.30 2D scan order (zeroing those
positions), multiply the gathered vector by the § 9.7 IST kernel
(`IST_4X4_KERNEL` or `IST_8X8_KERNEL` selected by `large = w >= 8 && h >= 8`),
apply § 4.8 `Round2Signed(t, 7)` and the `Clip3(±(1 << (BitDepth + 7)))` bound,
and scatter the results into the top-left scan sub-block via `Stx_Scan_Order_4x4`
or `Stx_Scan_Order_8x8` (and `Stx_Scan_Map` for the large case), honoring
`transpose`. It SHALL take `w`, `h`, `n`, `kernel`, `sec_tx_type` (`1..=3`),
`transpose`, and `bit_depth` as caller-resolved facts and SHALL NOT parse
`sec_tx_type`, derive the kernel / transpose / `n`, read frame or block state, or
wire into the runtime decode path. It SHALL be total and panic-free for valid
inputs and SHALL reject an invalid shape, buffer length, or parameter with a
typed `ReconError` before modifying any coefficient.

#### Scenario: Secondary transform succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon secondary_transform --locked` runs
- **THEN** the test suite covers `Round2Signed` for both signs, a hand-computed
  single-DC block matching literal IST kernel values, the small-4x4 and large-8x8
  paths (including the reduced 8x8 height and the transpose layout), and an
  i32-extreme totality sweep
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid input is rejected fail-atomically

- **WHEN** `secondary_inverse_transform` is called with a non-power-of-two side,
  a `dequant` length other than `w * h`, or an `n` / `kernel` / `sec_tx_type`
  outside the selected kernel set's range
- **THEN** it returns `ReconError::SecondaryTransformInvalidShape`,
  `SecondaryTransformBufferMismatch`, or `SecondaryTransformInvalidParams`
  respectively and leaves the `dequant` block unmodified
