# Tasks: MSDO/global-LCR agreement, cmvs/* diagnostics, Table A.4 re-land

## 1. Pre-implementation bookkeeping

- [x] 1.1 Matrix: `openspec_change` on `AV2-5.8.1-LCR-GLOBAL-INFO` and
  `AV2-7.3.2-CMVS-BOUNDARIES`; register in `openspec/changes/README.md`.
- [x] 1.2 Re-read PR #46 codex threads' requirements (quoted in the
  VALIDATOR-ROADMAP planned-diagnostics rows for the three annex-a ids) and
  §6.8.2 / §7.3.2 mirror text before coding.

## 2. Activated-global-LCR resolution

- [x] 2.1 Expose "the activated global LCR of the CMVS" from the existing
  association chain (frame-confirmed activated header's `seq_lcr_id` →
  local LCR `lcr_global_id` → global record; or direct global reference),
  with Unknown when no frame-confirmed activation resolves one. Reusable by
  §6.8.2, the DOH requirement, and the Table A.4 global-LCR arms.

## 3. § 6.8.2 agreement checks (deferred to CMVS resolution)

- [x] 3.1 `lcr/msdo-stream-count-mismatch`, `lcr/msdo-sub-xlayer-not-in-lcr`.
- [x] 3.2 `lcr/msdo-aggregate-mismatch` (Table A.6 consistency for
  `lcr_config_idc`, Table A.1 IOP equality vs `lcr_max_interop`, level and
  tier equality), gated on `lcr_aggregate_info_present_flag`.
- [x] 3.3 `lcr/msdo-substream-ptl-mismatch` gated on
  `lcr_seq_profile_tier_level_info_present_flag` (equality per §6.8.2,
  exact-match semantics — unlike the §6.6 ceiling checks).
- [x] 3.4 `lcr/msdo-doh-flag-mismatch`.
- [x] 3.5 `lcr/doh-constraint-required` (§6.8.2 line 1619) via the same
  deferred-resolution mechanism as `msdo/doh-constraint-required`.

## 4. § 7.3.2 boundary identity

- [x] 4.1 `cmvs/boundary-set-mismatch` on decidable disagreement between the
  MSDO-derived and MSDO+LCR-derived boundary sets (07 mirror ~line 351);
  CmvsTracker Unknown states stay silent; `cmvs/` added to
  `DIAGNOSTIC_PREFIXES`.

## 5. Table A.4 re-land

- [x] 5.1 Restore `interoperability_point` (Table A.1) to `annex_a.rs` and
  transcribe Table A.6 (verbatim, cell-verified, mirror lines ~242-254).
- [x] 5.2 Re-land the IOP window machinery with the PR #46 requirements:
  MSDO aggregate-profile IOP precedence; activated-global-LCR-only arms;
  §7.3.6 per-TU attribution (pre-CLK HLS belongs to the new CVS); same-id
  CLK seeding; whole-CVS windows (flush at next CVS start / EOS); Table A.3
  definition-order layer counting; ExternalHlsMode suppression;
  frame-confirmed gating.
- [x] 5.3 `annex-a/msdo-required-for-iop`, `annex-a/lcr-required-for-iop`
  re-land as errors; `annex-a/msdo-prohibited-for-iop` as the documented
  defensive arm. Remove the three rows from the roadmap planned-backlog
  table (they land).

## 6. Docs, registry, generated artifacts

- [x] 6.1 Register all new ids in `docs/VALIDATOR-DIAGNOSTICS.md`; matrix
  rows advance with proof; regenerate FEATURE-STATUS/SPEC-COVERAGE;
  VALIDATOR-ROADMAP Phase 5/6 mentions updated (cmvs/* landed).

## 7. Verification

- [x] 7.1 Tests: every §6.8.2 group positive/negative/boundary in both
  arrival orders; unactivated-global-LCR null cases; Unknown-state silence;
  every PR #46 codex scenario for Table A.4; Table A.6/A.1 spot-value tests.
- [x] 7.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 7.3 `cargo xtask ci` passes (bare, exit checked).
