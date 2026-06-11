# Proposal: LCR PTL and rep-info agreement with activated sequence headers

## Feature IDs

- `AV2-5.8.4-LCR-SEQ-PTL-INFO` (§ 6.8.5)
- `AV2-5.8.7-LCR-REP-INFO` (§ 6.8.8)
- `AV2-5.8-LAYER-CONFIG-RECORD` (umbrella bookkeeping)

## Why

Two LCR child rows carry verified, unimplemented "requirement of bitstream
conformance" sentences comparing an activated LCR's declared parameters
against the sequence headers activated by the same extended layer — pure
cross-record comparisons over state the validator already models (the
association chain with association-time snapshots, frame-confirmed
activation). § 6.8.8's rep-info residual was surfaced by the honesty sweep
(PR #45) as an audit-claim correction; § 6.8.5's four ceiling rules have no
coverage.

## What Changes

Grounded in `06-syntax-structures-semantics.md#s-6-8-5` (mirror lines
1768–1815) and `#s-6-8-8` (lines 1918–1969):

1. **§ 6.8.5 PTL ceilings** — when `lcr_seq_profile_tier_level_info(i)` is
   present in the LCR activated by the extended layer `i`'s frame-confirmed
   sequence header (`<=` semantics, equality passes):
   - `lcr/ptl-profile-exceeds-max` (error): `seq_profile_idc >
     lcr_seq_profile_idc[i]`.
   - `lcr/ptl-level-exceeds-max` (error): `seq_level_idx >
     lcr_max_level_idx[i]`.
   - `lcr/ptl-tier-exceeds-max` (error): `seq_tier > lcr_tier_flag[i]`.
   - `lcr/ptl-mlayer-count-exceeds-max` (error):
     `seq_max_mlayer_cnt_minus_1 + 1 > lcr_max_mlayer_count[i]`.
2. **§ 6.8.8 rep-info equality** — `lcr/rep-info-mismatch` (error, the
   disagreeing field named in the message): the activated LCR's
   `lcr_max_pic_width` / `lcr_max_pic_height` vs
   `max_frame_width/height_minus_1 + 1`, `lcr_bit_depth_idc` vs
   `bit_depth_idc`, `lcr_chroma_format_idc` vs `chroma_format_idc`, and the
   cropping-window flag/offsets vs the `seq_cropping_*` values, per the
   § 6.8.8 equality sentences, for each header activated by the same
   extended layer. (Verify field presence gates in the mirror before
   comparing; absent rep-info compares nothing.)
3. **§ 6.8.5/§ 6.8.8 Annex A value spaces** (`lcr_max_level_idx[i]` reserved
   range etc.): reuse `annex-a/level-reserved` where the sentence maps onto
   the same Table A.7 value space; do not invent new ids for value-space
   facts already covered.

Comparisons run at frame-confirmed activation against the header's own
association-time LCR snapshot (the PR #48 snapshot field), and on LCR
non-identical redefinition re-check affected layers per the established
fingerprint mechanism. Both arrival orders covered by the association
semantics (unresolved references are already diagnosed by the § 7.3.8.3
check and compare nothing here).

## Non-goals

- § 6.8.2 (landed, PR #48), § 6.8.6 payload bounds (parse-enforced), the
  same-global-LCR-for-all-layers arbitration residual.
- `lcr_enforce_tile_alignment_flag` (frame/tile-blocked).
- Annex A profile/level constraint derivation from LCR values (the § 6.8.5
  NOTE explicitly says these maxima do not feed Annex A).

## Acceptance criteria

- [ ] Each rule: violation, equality-passes boundary, absent-PTL/rep-info
  silence, unconfirmed-activation silence, redefinition recheck, both
  arrival orders where reachable.
- [ ] Matrix rows advance with proof (`AV2-5.8.7-LCR-REP-INFO`'s blocker
  note from PR #45 resolves); registry/feature-status/ci/coverage gates
  pass.
