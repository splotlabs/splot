# Tasks: LCR PTL and rep-info agreement

## 1. Bookkeeping

- [x] 1.1 Matrix `openspec_change` on `AV2-5.8.4-LCR-SEQ-PTL-INFO` and
  `AV2-5.8.7-LCR-REP-INFO`; register in `openspec/changes/README.md`.
- [x] 1.2 Verify in the parsed model where `lcr_seq_profile_tier_level_info`
  and the rep-info fields live (local vs global records) and how the
  activation chain reaches them; re-read § 6.8.5/§ 6.8.8 mirror text.
  - PTL `LcrSeqProfileTierLevelInfo` lives on **both** records:
    `LcrLocalInfo.seq_ptl_info: Option<_>` (local, the § 6.8.5 keying record:
    the sentence says "associated with the local LCR ... indicated in an
    extended layer with obu_xlayer_id equal to i") and
    `LcrGlobalInfo.seq_ptl_infos: Vec<_>` (global, per xlayer in the map).
  - Rep-info `LcrRepInfo` lives inside `LcrXlayerInfo.rep_info` on
    `LcrLocalInfo.xlayer_info` (local, isGlobal=0) and on
    `LcrGlobalPayload.xlayer_info` (global, isGlobal=1).
  - Activation reaches them through the § 6.4.1 `LcrAssociation` snapshot
    (`snapshot_lcr_association`), consumed from `on_sequence_activation` via
    `agreement_activation_for` (frame-confirmed only) — same path as the
    § 6.8.9 `check_lcr_dependency_agreement`. The existing snapshot carried
    only `global_record` (PTL profile/level/tier, no mlayer-count, no
    rep-info) and `maps`; this change adds local PTL, global mlayer-count,
    and rep-info to the snapshot.

## 2. § 6.8.5 PTL ceiling checks

- [x] 2.1 `lcr/ptl-profile-exceeds-max`, `lcr/ptl-level-exceeds-max`,
  `lcr/ptl-tier-exceeds-max`, `lcr/ptl-mlayer-count-exceeds-max` (errors)
  at frame-confirmed activation vs the association-time LCR snapshot;
  presence-gated on `lcr_seq_profile_tier_level_info(i)`; equality passes.
  Implemented in `check_lcr_ptl_ceilings` (context.rs), called from
  `on_sequence_activation`. `<=` semantics, equality passes; absent PTL
  compares nothing.
- [x] 2.2 Annex A value-space reuse decision for the LCR-declared maxima.
  DECISION: the § 6.8.5 "shall not contain values of `lcr_*[i]` outside those
  specified in Annex A" sentences (lines 1772/1779-1780/1789-1791/1804-1806)
  constrain the LCR-*declared* values themselves, distinct from the four
  ceiling comparisons against the activated header. Only `lcr_max_level_idx[i]`
  cleanly maps onto Table A.7's level value space (the `annex-a/level-reserved`
  territory); `lcr_seq_profile_idc[i]`/`lcr_tier_flag[i]`/`lcr_max_mlayer_count[i]`
  ride the broader Annex A profile/tier/layer value spaces that are not modeled.
  Per the no-new-id rule, **no new diagnostic id is invented** for these
  value-space facts; wiring them stays on the Annex A profile/level/tier table
  backlog (the residual recorded in the `AV2-5.8.4-LCR-SEQ-PTL-INFO` matrix
  note), so `validate` for that row stays `partial`.

## 3. § 6.8.8 rep-info equality

- [x] 3.1 `lcr/rep-info-mismatch` (error, field named): dims, bit depth,
  chroma format, cropping flag/offsets vs the activated header; verify the
  exact § 6.8.8 sentences and presence gates in the mirror before each
  comparison. Implemented in `check_lcr_rep_info_agreement` (context.rs).
  Presence gates per the mirror: width/height always present (1925-1933);
  bit-depth/chroma gated on `lcr_format_info_present_flag` (1950-1958);
  cropping present-flag equality + offsets, with `seq_cropping_window_present_flag`
  now retained on `SequenceHeaderGeneral` for an exact flag comparison
  (1943-1968).

## 4. Redefinition and dedup

- [x] 4.1 Non-identical LCR redefinition re-checks affected layers; dedup
  keys carry content fingerprints (established mechanisms). The
  `LcrPtlFindingKey` / `LcrRepInfoFindingKey` dedup keys carry the LCR OBU
  offset (a redefinition is a new OBU) plus the LCR-declared and header
  compared values, so a non-identical redefinition (re-snapshotted at the next
  activation) re-emits while an identical re-evaluation is idempotent. The LCR
  observation clears its PTL/rep-info store before re-recording so a dropped
  PTL/rep-info cannot leave stale entries (`clear_local_lcr_extras` /
  `clear_global_lcr_extras`).

## 5. Docs, registry, artifacts

- [x] 5.1 Register ids; matrix rows advance with proof (resolve the
  `AV2-5.8.7-LCR-REP-INFO` blocker note from the honesty sweep); regenerate
  FEATURE-STATUS/SPEC-COVERAGE; roadmap mention updated.

## 6. Verification

- [x] 6.1 Tests per acceptance criteria (23 new tests in validator.rs: per
  rule violation, equality-passes boundary, absent-PTL/rep-info silence,
  unconfirmed-activation silence, redefinition recheck, both arrival orders
  reachable, dedup-across-reactivation, diagnostic-anchor, external-HLS
  suppression, global-record path).
- [x] 6.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 6.3 `cargo xtask ci` (bare, exit checked) passes.
