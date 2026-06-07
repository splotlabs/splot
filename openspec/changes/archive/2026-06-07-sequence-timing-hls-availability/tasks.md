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
- [x] Validate `sequence-state/monotonic-output-order-mismatch` and
      `sequence-state/distinct-mlayer-count-exceeds-seq-max` (§6.4.1) as out of
      scope for this change. They still require CMVS / coded-frame / random-access
      scoping state beyond the current frame-header prefix model; no new OpenSpec
      change is created here.

## 2. Full HLS availability store (§7.3.8) — PR B

- [x] Add an availability store (`HlsAvailabilityStore`) for in-band HLS objects in
      `ValidatorContext`. Sequence-header availability (the one implemented in-band
      reference path) is modeled; MSDO / MFH / LCR / atlas / OPS *records* are
      deferred because their consumers (frame-header `cur_mfh_id`, the RAP-identical
      MSDO rule, `seq_lcr_id` resolution) need frame-header parsing or RAP detection
      and would otherwise be unconsumed state.
- [x] Add `ValidationOptions { external_hls: ExternalHlsMode }`; default disabled.
      `Validator::validate_bytes` delegates to a new
      `validate_bytes_with_options(.., &ValidationOptions::default())`, so the
      existing API is unchanged.
- [x] Emit `mfh/sequence-header-unavailable` (and the advisory
      `hls/external-hls-disabled` under default options) when an MFH references a
      sequence header that is not available in-band or through supplied external HLS
      (§7.3.8.6/§7.3.8.7). `hls/unavailable-sequence-header` is reserved for the
      generic frame-header reference path (blocked on `AV2-5.18-FRAME-HEADER`).
- [x] Keep CLK/frame-header-dependent activation bounded
      (`AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`) until frame headers are parsed: the
      in-band store is monotonic and CVS resets stay approximated on the CLK.
- [x] Validate the per-xlayer active sequence header reset as superseded for the
      modeled paths by `frame-activation-hls-skeleton`: parsed frame-header
      references now update `active_sequence_by_xlayer`, and exact CVS resets remain
      intentionally bounded on random-access / long-term-reference state.

### CLK-driven activation (root cause of the activation approximations)

The following were re-audited after `frame-activation-hls-skeleton`, which added the
prefix-only frame-header activation path:

- [x] Activate a received sequence header when a parsed frame-header reference is
      seen (AV2 §7.3.8). The OBU-order fallback remains only for unparsed paths so
      the validator stays sound-over-complete until full frame/tile coverage lands.
- [x] Preserve the fingerprint of the sequence header that opens a CVS so a later
      non-identical repeat *within* the modeled temporal-unit/CVS scope is still
      caught. Exact cross-temporal-unit CVS scoping remains bounded on random-access
      / long-term-reference state.

## 3. Matrix, docs, and proof

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` statuses and proof (PR A:
      `AV2-5.15-CONTENT-INTERPRETATION`, `AV2-5.4.12-TIMING-INFO`,
      `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`; PR B: `AV2-7.3.8-HLS-AVAILABILITY`,
      `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`, `AV2-5.6-MSDO`,
      `AV2-5.7-MULTI-FRAME-HEADER`).
- [x] Regenerate `docs/FEATURE-STATUS.md`.
- [x] Update `STATUS.md`.
- [x] Run `cargo xtask check-feature-status` and `cargo xtask ci`.
