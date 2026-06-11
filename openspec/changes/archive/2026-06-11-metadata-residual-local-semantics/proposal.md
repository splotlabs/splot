# Proposal: metadata timecode and frame-hash residual local semantics

## Feature IDs

- `AV2-5.17.7-METADATA-TIMECODE` (§ 6.16.7 residuals)
- `AV2-5.17.12-METADATA-DECODED-FRAME-HASH` (reserved-field checks only)

## Why

The honesty sweep (PR #45) scoped `AV2-5.17.7-METADATA-TIMECODE`'s remaining
locally-decidable work to the § 6.16.7 inference-presence rules and the
timing-gated bounds (the range checks already landed). Those are stateful but
decidable from metadata alone. The decoded-frame-hash row's hash verification
is decoder-blocked, but its reserved fields are local facts.

## What Changes

Grounded in `06-syntax-structures-semantics.md#s-6-16-7` (mirror lines
~3840–3900) and `#s-6-16-13`:

1. `metadata/timecode-inferred-without-previous` (error, § 6.16.7): a
   timecode whose `seconds_value` / `minutes_value` / `hours_value` is
   absent (inferred from "the previous set of clock timestamp syntax
   elements in decoding order") when no previous set carried that value —
   the mirror states "it is required that such a previous … shall have been
   present" for each of the three. State scope per the existing metadata
   lifetime conventions (verify how the store scopes decoding-order state;
   document the chosen scope with the mirror's wording).
2. `metadata/timecode-n-frames-exceeds-rate` (error, § 6.16.7): when
   `ci_timing_info_present_flag == 1`, `n_frames < maxPicPerSecond =
   ceil(time_scale / TicksPerPicture)` — implement ONLY if the
   content-interpretation timing state needed for the bound is already
   parsed and trackable; otherwise record the blocker honestly in the
   matrix note (no guessing).
3. The § 6.16.7 `clockTimestamp` output-order monotonicity sentence stays
   blocked (needs frame output ordering) — matrix note names it.
4. Decoded-frame-hash reserved fields (§ 6.16.13): verify whether the
   parser/checks already reject nonzero reserved fields; add the missing
   check only if the mirror states one and it is absent (reuse the existing
   `metadata/*` reserved-field pattern); otherwise note-only.

## Non-goals

- § 6.16.13 hash verification (decoder-blocked; stays documented).
- § 7.3.3/§ 7.3.4 metadata placement (frame-unit-segmentation item).
- The output-order monotonicity check (blocked, named in the note).

## Acceptance criteria

- [ ] Inference-presence: violation per field, first-timecode-with-full-
  timestamp passes, inferred-after-present passes, scope-boundary behavior
  tested per the chosen scope.
- [ ] The n_frames bound lands only with real timing state (equality/
  boundary tested) or is honestly deferred.
- [ ] Matrix rows advance with proof; registry/feature-status/ci/coverage
  gates pass.
