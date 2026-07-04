# recon-wienerns-chroma-filter-primitive Specification

## Purpose
Define the scheduler-free `splot-recon` primitive for AV2 §7.20.3 chroma
non-separable Wiener filter arithmetic, without claiming runtime loop
restoration wiring or successful decode output.

## Requirements

### Requirement: Chroma Wiener NS filter primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
AV2 §7.20.3 chroma non-separable Wiener filter process, tracked by
`RECON-WIENERNS-CHROMA-FILTER-PRIMITIVE`. The primitive SHALL expose
`WIENER_NS_CHROMA_COEFFS = 18`, `WIENER_NS_CHROMA_TAPS = 12`, a caller-owned
`WienerNsChromaFilter` parameter type, and
`wiener_ns_filter_chroma_block(output, params, chroma_source_sample, luma_source_sample) -> Result<()>`.
For each output sample `(c, r)`, it SHALL read the center chroma sample `m`,
initialize `s = m << WIENER_NS_PREC_BITS` with `WIENER_NS_PREC_BITS = 7`, walk
the AV2 §7.20.3 `Wiener_Ns_Config_Uv` table, add chroma tap differences using
coefficient indexes `0..6`, derive `mLuma` through the §7.20.3
`get_luma_sample` process, add luma tap differences using coefficient indexes
`6..18`, round with §4.8 `Round2(s, 7)`, clamp with §4.8 `Clip1`, and write the
result into the caller output with the supplied stride. The caller SHALL resolve
source-frame selection, restoration-unit traversal, coefficient source
selection, and frame-coordinate offsets. The primitive SHALL NOT implement full
§7.20 traversal, §7.20.2 frame reads, runtime decode wiring, GDF/BRU, or local decoder mission
output.

#### Scenario: Chroma Wiener NS math is covered by focused tests

- **WHEN** `cargo test -p splot-recon wienerns_chroma_filter --locked` runs
- **THEN** the test suite covers zero-coefficient chroma source copying,
  hand-computed chroma tap accumulation, hand-computed luma tap contribution,
  4:2:0 luma downsampling for `cfl_ds_filter_index` values `0` and `1`,
  the `cfl_ds_filter_index == 3` remap, 4:2:2 direct luma reads,
  non-subsampled direct luma reads, 8-bit and 10-bit `Clip1` behavior, and
  source samples outside the active bit-depth range
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid input is rejected fail-atomically

- **WHEN** `wiener_ns_filter_chroma_block` is called with zero dimensions, an
  output stride smaller than the block width, an output buffer too short for the
  strided block, invalid chroma subsampling, inverted or unrepresentable luma
  bounds, invalid `cfl_ds_filter_index`, source samples outside the active
  bit-depth range, or unsupported sample storage for the active bit depth
- **THEN** it returns a typed `ReconError`
- **AND** the caller output remains unmodified
