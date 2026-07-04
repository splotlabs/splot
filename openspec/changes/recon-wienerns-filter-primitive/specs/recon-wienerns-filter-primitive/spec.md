## ADDED Requirements

### Requirement: Luma Wiener NS filter primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
AV2 §7.20.3 luma non-separable Wiener filter process, tracked by
`RECON-WIENERNS-FILTER-PRIMITIVE`. The primitive SHALL expose
`WIENER_NS_LUMA_COEFFS = 16`, `WIENER_NS_LUMA_TAPS = 32`, a caller-owned
`WienerNsLumaFilter` parameter type, and
`wiener_ns_filter_luma_block(output, params, source_sample) -> Result<()>`.
For each output sample `(c, r)`, it SHALL read the center sample `m` from
`source_sample(c, r)`, initialize `s = m << WIENER_NS_PREC_BITS` with
`WIENER_NS_PREC_BITS = 7`, walk the AV2 §7.20.3 `Wiener_Ns_Config_Y` table, add
`(source_sample(c + dx, r + dy) - m) * coeffs_by_class[subclass][idx]` for each
tap, round with §4.8 `Round2(s, 7)`, clamp with §4.8 `Clip1`, and write into the
caller output with the supplied stride. The caller SHALL resolve source-sample
coordinates, §7.20.2 clipping/stripe behavior, PC-Wiener class/subclass mapping,
and coefficient selection. The primitive SHALL NOT implement chroma Wiener NS,
§7.20 traversal, restoration-unit syntax, temporal/reference Wiener state,
runtime decode wiring, or local decoder mission output.

#### Scenario: Luma Wiener NS math is covered by focused tests

- **WHEN** `cargo test -p splot-recon wienerns_filter --locked` runs
- **THEN** the test suite covers zero-coefficient source copying, a hand-computed
  luma tap accumulation, per-sample subclass selection, 8-bit and 10-bit `Clip1`
  behavior, and source samples outside the active bit-depth range
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid input is rejected fail-atomically

- **WHEN** `wiener_ns_filter_luma_block` is called with zero dimensions, an output
  stride smaller than the block width, an output buffer too short for the strided
  block, no coefficient classes, a subclass map shorter than `width * height`, a
  subclass index outside `coeffs_by_class`, or source samples outside `bit_depth`
- **THEN** it returns a typed `ReconError`
- **AND** the caller output remains unmodified
