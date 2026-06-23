## ADDED Requirements

### Requirement: Loop-restoration source-sample selector

The repository SHALL provide a scheduler-free `splot-recon` selector for the AV2
section 7.20.2 get source sample process, tracked by
`RECON-LOOP-RESTORATION-SOURCE-SAMPLE`. The selector SHALL expose
`LoopRestorationSourceBounds`, `LoopRestorationSourceSample`,
`LoopRestorationSource`, and
`loop_restoration_source_sample(plane, x, y, bounds) -> Result<LoopRestorationSourceSample>`.
It SHALL derive `subX` and `subY` as zero for luma and from the supplied
sequence subsampling for chroma; clip `x` and `y` to the luma-derived allowed
plane extents; derive `stripeStartY` and `stripeEndY`; select `CurrFrame` for
samples above or below the current stripe with two-line clamping; and select
`CdefFrame` for samples inside the current stripe. The selector SHALL NOT read
frame storage, traverse loop-restoration units, apply Wiener NS or PC-Wiener
filters, implement GDF/BRU, wire runtime decode, or produce ac0ej3 output.

#### Scenario: Source selection and clipping match section 7.20.2

- **WHEN** `cargo test -p splot-recon loop_restoration --locked` runs
- **THEN** the test suite covers luma samples inside the stripe reading
  `CdefFrame`, samples above and below the stripe reading `CurrFrame`, two-line
  out-of-stripe clamping, chroma subsampled bounds, and luma ignoring sequence
  subsampling
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid caller-resolved facts are rejected

- **WHEN** `loop_restoration_source_sample` is called with subsampling outside
  the AV2 `0..=1` domain, inverted luma x/y ranges, an inverted luma stripe
  range, stripe bounds outside the luma y range, or luma bounds that cannot be
  represented for signed clipping
- **THEN** it returns a typed `ReconError`
- **AND** it does not read frame storage or mutate caller-owned frame data
