# Change: rap-replay-qm-references

## Feature IDs

- `AV2-7.3.8-HLS-AVAILABILITY`

## Why

Completes the § 7.3.8.1 random-access-point (RAP) HLS-availability replay by wiring
quantizer-matrix level references, the last residual on `AV2-7.3.8-HLS-AVAILABILITY` after
`rap-replay-film-grain-references`. The linear `frame-header/qm-level-unavailable`
(§ 7.3.8.9) reads `QuantizerMatrixState.available[]`, which the reset/poison discipline
governs for sequential decode — but it under-reports the random-access direction: a custom
QM level made available before a random access point and not resent is unavailable when a
decoder starts there, yet the linear test (when the level survives the random access point's
reset, e.g. a RAS that clears only the `QmMLayerId == -1` arm) sees it present.

This is sound to wire: the replay records the QM OBU *send* (its temporal facts) and is
disjoint from the linear availability/reset/poison state. No QM level is available from a
decode start without a quantizer-matrix OBU send at or after it (§ 6.12 makes even a default
matrix an in-OBU field), so "no qualifying resend visible from the random access point" ⇔
"unavailable from that start", regardless of reset_qm() (which only governs the linear
check). A `qm_bit_map == 0` reset-to-defaults makes EVERY level available, so it records a
(re)send for all levels.

## Scope

- Spec sections: § 7.3.8.1 (random-access-point availability), § 7.3.8.9 (quantizer matrix
  availability).
- Crates/modules: `crates/splot-validate/src/context/rap_replay.rs` (new
  `RapHlsKey::QmLevel` variant + family/section/describe + external-HLS suppression arm),
  `crates/splot-validate/src/context/quantizer_matrix.rs` (`note_resend` per available level
  in `check_quantizer_matrix`, incl. all levels on `qm_bit_map == 0`;
  `frame_qm_reference_checks` returns the linearly-available referenced levels),
  `crates/splot-validate/src/context/frame_header_checks.rs` (`FrameRapReferences` carries
  the QM levels) / `frame_headers.rs` (buffer the QM references).
- CLI/docs/tests: matrix notes (the `AV2-7.3.8-HLS-AVAILABILITY` QM residual closed); no new `rule_id` (feeds the
  existing `hls/unavailable-at-random-access-point`).

## Non-goals

- The § 6.17.6.2 QM layer-dependency constraints (`MLayerDependencyMap`/`TLayerDependencyMap`
  for QM) — still a separate residual (the QM-side check needs the defining OBU's layer
  identity threaded; the linear `frame_qm_reference_checks` TODO stays).
- The `MLayerPresenceMap` arm of the SWITCH/RAS `reset_qm()` (a named residual on the linear
  side) — unchanged; the replay does not depend on it (it reads sends, not resets).

## Acceptance criteria

- [ ] `AV2-7.3.8-HLS-AVAILABILITY` notes updated (QM RAP-replay wired; the umbrella RAP
      residual is closed).
- [ ] `RapHlsKey::QmLevel` participates in family/section/describe + external-HLS suppression
      (inexpressible kind → blanket-suppress under any Provided mode).
- [ ] Negative: a level sent before a (RAS) random access point, surviving its reset and not
      resent, referenced by an INTRA_ONLY frame, fires `hls/unavailable-at-random-access-point`.
- [ ] Positive: a level resent after the random access point (incl. via a `qm_bit_map == 0`
      reset-to-defaults) stays silent.
- [ ] Disjointness: a never-sent level fires only the linear `frame-header/qm-level-unavailable`.
- [ ] Suppression: a Provided external-HLS mode suppresses the QM replay.
- [ ] `cargo xtask ci` passes.
