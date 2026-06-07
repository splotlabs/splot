# Tasks: sequence timing consistency and HLS availability

These tasks were deferred from the `sequence-hls-validator-coverage` change.

## 1. Cross-embedded-layer timing consistency (§6.4.12)

- [ ] Parse the content-interpretation OBU (`AV2-5.15-CONTENT-INTERPRETATION`) far
      enough to reach `ci_timing_info_present_flag` and call the existing
      `parse_timing_info()`.
- [ ] Store timing values per embedded layer in `ValidatorContext`.
- [ ] Emit `sequence-header/timing-display-tick-mismatch`,
      `…/timing-time-scale-mismatch`, `…/timing-equal-picture-interval-mismatch`,
      and `…/timing-num-ticks-mismatch` when present timing values differ across
      embedded layers in the same coded video sequence.
- [ ] Add `sequence-state/monotonic-output-order-mismatch` and
      `sequence-state/distinct-mlayer-count-exceeds-seq-max` (§6.4.1) once the
      multistream state needed to decide them is available.

## 2. Full HLS availability store (§7.3.8)

- [ ] Add an availability store for parsed MSDO / MFH (and later LCR / atlas / OPS)
      objects in `ValidatorContext`.
- [ ] Add `ValidationOptions { external_hls: ExternalHlsMode }`; default disabled.
- [ ] Emit `hls/unavailable-sequence-header` and `hls/external-hls-disabled`
      (§7.3.8) and `mfh/sequence-header-unavailable` when an MFH references a
      sequence header that is not available in-band or externally.
- [ ] Keep CLK/frame-header-dependent activation bounded
      (`AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`) until frame headers are parsed.

## 3. Matrix, docs, and proof

- [ ] Update `docs/IMPLEMENTATION-MATRIX.toml` statuses and proof.
- [ ] Regenerate `docs/FEATURE-STATUS.md`.
- [ ] Update `STATUS.md`.
- [ ] Run `cargo xtask check-feature-status` and `cargo xtask ci`.
