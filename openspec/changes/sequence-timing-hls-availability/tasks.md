# Tasks: sequence timing consistency and HLS availability

These tasks were deferred from the `sequence-hls-validator-coverage` change.

## 1. Cross-embedded-layer timing consistency (§6.4.12) — PR A

- [x] Parse the content-interpretation OBU (`AV2-5.15-CONTENT-INTERPRETATION`) far
      enough to reach `ci_timing_info_present_flag` and call the existing
      `parse_timing_info()`. (Full §5.15 parser incl. `rg(2)`; dispatched by
      `open_bitstream_unit` and surfaced in `inspect --json`.)
- [x] Store timing values per embedded layer in `ValidatorContext`
      (`content_interpretations` keyed by `(obu_xlayer_id, obu_mlayer_id)`).
- [x] Emit `sequence-header/timing-display-tick-mismatch`,
      `…/timing-time-scale-mismatch`, `…/timing-equal-picture-interval-mismatch`,
      and `…/timing-num-ticks-mismatch` when present timing values differ across
      embedded layers in the same coded video sequence.
- [x] Flag a repeated, non-identical content-interpretation OBU for the same
      embedded layer (`content-interpretation/repeated-ci-not-identical`, §6.14)
      and surface a non-zero `ci_reserved_2bit`
      (`content-interpretation/reserved-bits-nonzero`, §6.14).
- [ ] Add `sequence-state/monotonic-output-order-mismatch` and
      `sequence-state/distinct-mlayer-count-exceeds-seq-max` (§6.4.1) once the
      multistream state needed to decide them is available. (Blocked on CLK
      frame-header activation, `AV2-5.18-FRAME-HEADER`.)

## 2. Full HLS availability store (§7.3.8)

- [ ] Add an availability store for parsed MSDO / MFH (and later LCR / atlas / OPS)
      objects in `ValidatorContext`.
- [ ] Add `ValidationOptions { external_hls: ExternalHlsMode }`; default disabled.
- [ ] Emit `hls/unavailable-sequence-header` and `hls/external-hls-disabled`
      (§7.3.8) and `mfh/sequence-header-unavailable` when an MFH references a
      sequence header that is not available in-band or externally.
- [ ] Keep CLK/frame-header-dependent activation bounded
      (`AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`) until frame headers are parsed.
- [ ] Reset the per-xlayer active sequence header (`active_sequence_by_xlayer` /
      `sequence_headers`) at CVS boundaries so a CLK-driven reconfiguration that
      reuses a `seq_header_id` with new layer limits is checked against the new
      header, not the stale one. Requires CLK frame-header activation (the CLK
      follows its sequence header in OBU order), so it cannot be done by resetting
      purely on the CLK without breaking intra-CVS frames.

### CLK-driven activation (root cause of the activation approximations)

All of the following need the CLK/OLK frame header (which references and activates a
`seq_header_id`) to be parsed. Because the activating CLK *follows* its sequence
header in OBU order, none can be done correctly at the OBU level alone; the current
validator uses sound-over-complete approximations and these tasks make them exact:

- [ ] Activate a received sequence header only when the first CLK/OLK that
      references it is seen (AV2 §7.3.8), instead of on the base-layer
      `OBU_SEQUENCE_HEADER` itself. Until then, coded OBUs between a sequence header
      and its activating key frame are checked against a still-pending header.
- [ ] Preserve the fingerprint of the sequence header that opens a CVS so a later
      non-identical repeat *within* that CVS is still caught (the current
      `maybe_reset_coded_video_sequence` clears it on the CLK, which trails the
      header in the CVS-start temporal unit, yielding a false negative — the
      conservative inverse of the earlier cross-CVS false positive).

## 3. Matrix, docs, and proof

- [ ] Update `docs/IMPLEMENTATION-MATRIX.toml` statuses and proof.
- [ ] Regenerate `docs/FEATURE-STATUS.md`.
- [ ] Update `STATUS.md`.
- [ ] Run `cargo xtask check-feature-status` and `cargo xtask ci`.
