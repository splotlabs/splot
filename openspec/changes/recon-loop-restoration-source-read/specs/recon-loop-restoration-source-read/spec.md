## ADDED Requirements

### Requirement: Loop-restoration source-sample frame read

The repository SHALL provide a scheduler-free `splot-recon` helper for the AV2
section 7.20.2 source-sample frame read, tracked by
`RECON-LOOP-RESTORATION-SOURCE-READ`. The helper SHALL expose
`LoopRestorationSourceSampleValue<T>` and
`loop_restoration_source_sample_value(plane, x, y, bounds, curr_frame,
cdef_frame) -> Result<LoopRestorationSourceSampleValue<T>>`. It SHALL reuse the
existing section 7.20.2 coordinate/source selector, read from `CurrFrame` when
the resolved sample lies above or below the current stripe, read from
`CdefFrame` when the resolved sample lies inside the current stripe, and read
through immutable `FrameRef` / `PlaneRef` views without allocating or mutating
caller-owned frame data. It SHALL NOT derive restoration-unit bounds, traverse
loop restoration, apply Wiener NS/chroma Wiener NS/PC-Wiener/GDF/BRU filters,
wire runtime decode, or produce ac0ej3 output.

#### Scenario: Source reads follow section 7.20.2 frame selection

- **WHEN** `cargo test -p splot-recon loop_restoration --locked` runs
- **THEN** the test suite covers in-stripe samples reading from `CdefFrame`,
  out-of-stripe samples reading from `CurrFrame` after two-line clamping, and
  chroma-plane reads using subsampled bounds
- **AND** the helper returns both the resolved source sample metadata and the
  sample value read from the selected immutable frame view

#### Scenario: Invalid caller-provided frame views are rejected

- **WHEN** `loop_restoration_source_sample_value` is called with mismatched
  `CurrFrame` / `CdefFrame` metadata, a selected sample outside the visible
  plane, or an absent selected chroma plane
- **THEN** it returns a typed `ReconError`
- **AND** it does not allocate, mutate caller-owned frame data, or invoke a
  runtime decoder
