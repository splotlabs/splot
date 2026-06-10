# Tasks: Sequence-header multistream semantics

## 1. Pre-implementation bookkeeping

- [x] 1.1 Update `docs/SPEC-MAPPING.md` if the §6.4.1 / §7.3.2 / §7.3.6 rules used here
  are not yet represented (bitstream-affecting behavior rule). (No change needed:
  SPEC-MAPPING.md records spec *sources and citation rules*, not per-section rules or
  status; the committed mirror already represents §6.4.1 / §7.3.2 / §7.3.6, and the
  file deliberately carries no per-feature/per-rule status prose.)
- [x] 1.2 Update `docs/IMPLEMENTATION-MATRIX.toml`: set `openspec_change =
  "sequence-multistream-semantics"` on `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`,
  `AV2-7.3.8-HLS-AVAILABILITY`, `AV2-7.3.2-CMVS-BOUNDARIES`; fix the stale
  "blocked on CLK frame-header activation" note on the §6.4 row.
- [x] 1.3 Register the change in `openspec/changes/README.md` (Active changes table).

## 2. Supporting state (no diagnostics)

- [x] 2.1 Add the stateful §5.6 MSDO observer to `ValidatorContext` (last-seen MSDO,
  §7.3.2 condition-2 key-field change detection), with unit tests.
- [x] 2.2 Add the three-state §7.3.2 CMVS begin/end tracker
  (`Outside`/`Inside`/`Unknown`) fed by CLK observations, the MSDO observer, and
  global-LCR activation; inline spec quotes at each transition; unit tests for every
  begin/end condition and for `Unknown` conservatism.

## 3. Diagnostics

- [x] 3.1 Implement `frame-header/switch-or-ras-mlayer-dependency-not-self-contained`
  in `frame_header_core_checks` (§6.4.1), with positive/negative tests for
  `OBU_SWITCH` and `OBU_RAS_FRAME`.
- [x] 3.2 Implement `sequence-state/distinct-mlayer-count-exceeds-seq-max` (§6.4.1):
  per-xlayer distinct-`obu_mlayer_id` sets reset at CVS starts; document the
  global-OBU attribution reading with a mirror citation; emit once per CVS; tests
  including the CVS-boundary reset and pre-first-CLK edge cases.
- [x] 3.3 Implement `hls/multiple-active-sequence-headers` (§7.3.6) in the
  `load_sequence_header` activation path, gated on frame-confirmed prior activation,
  no intervening CVS start, and `ExternalHlsMode::Provided` suppression; tests for
  the firing case, CLK re-activation, fallback-guess non-firing, and unreferenced
  extra header non-firing.
- [x] 3.4 Implement `sequence-state/monotonic-output-order-mismatch` (§6.4.1) gated on
  CMVS `Inside`; tests for disagreement inside a CMVS (MSDO-begun), agreement inside a
  CMVS, and disagreement outside/`Unknown` not firing.

## 4. Registry, docs, and generated artifacts

- [x] 4.1 Add the four rule ids to `docs/VALIDATOR-DIAGNOSTICS.md` registry tables
  (CI `check-diagnostic-registry` requires exact set equality).
- [x] 4.2 Update `docs/VALIDATOR-ROADMAP.md`: move the three landed backlog rows out of
  the planned-diagnostics table, correct the §7.3.8 → §7.3.6 citation, drop the
  "warning until CLK parsing exists" hedge, refresh Current focus / Phase 4 / Phase 5
  status lines.
- [x] 4.3 Update matrix stages with proof (tests listed in `proof` fields):
  `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` validate stays `partial` (blocked residuals
  documented), `AV2-7.3.2-CMVS-BOUNDARIES` types/validate → `partial`,
  `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT` / `AV2-7.3.8-HLS-AVAILABILITY` notes updated.
- [x] 4.4 Regenerate `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`
  (`cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`;
  `cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md`).
- [x] 4.5 Check `README.md` capability claims; update only if test counts/claims shift.
  (No change needed: the README uses soft/round counts — "over 700 tests" (workspace
  now 832, still true), "128 tracked features" (unchanged), "628 indexed sections"
  (spec mirror untouched) — and states no exact diagnostic-registry count, so nothing
  drifted.)
- [x] 4.6 Re-record the audit ledger: `cargo xtask audit-scope --all --write-ledger`.

## 5. Verification

- [x] 5.1 `cargo xtask feature-status` and `cargo xtask check-feature-status` pass.
- [x] 5.2 `cargo xtask ci` passes (fmt, clippy, build, tests, doctests, typos,
  machete, deny, repo checks) with `RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin`.
- [x] 5.3 Fuzz smoke: `cargo xtask fuzz --time 30` (run-if-present) shows no panics
  (5,004,469 runs in 31s).

## 6. Review follow-ups (PR #38 Codex findings)

- [x] 6.1 (FIX 1) Defer the `sequence-state/monotonic-output-order-mismatch` check when
  the §7.3.2 `Inside` membership is only provisional at a sequence-header OBU (no CLK
  observed yet in the temporal unit), resolving the deferred verdict at temporal-unit
  completion (`CmvsTracker::monotonic_verdict` / `complete_temporal_unit`). Removes the
  false positive on a §7.3.6-permitted same-CVS redefinition preceding a CMVS-ending
  CLK. Tests: `monotonic_output_order_provisional_inside_clk_ending_tu_is_not_flagged`,
  `_mid_cmvs_redefinition_is_flagged`, `_flushes_at_end_of_bitstream`,
  `_unknown_clk_is_not_flagged`.
- [x] 6.2 (FIX 2 + 3) Narrow the `hls/multiple-active-sequence-headers` (Gate 4) and
  `sequence-state/monotonic-output-order-mismatch` external-HLS gates from
  `Provided(_)` to `declares_any_sequence_header()`, matching the sibling gates. Tests:
  `second_activation_under_empty_external_hls_is_flagged`,
  `_sequence_free_external_hls_is_flagged`, `_out_of_range_external_hls_id_is_flagged`,
  `monotonic_output_order_disagreement_under_empty_external_hls_is_flagged`; existing
  suppression tests stay green.
- [x] 6.3 (FIX 4) Upgrade `DistinctMlayerTracker::reset_cvs` from a whole-state drop to
  exact re-attribution of the boundary temporal unit's pre-CLK ids to the new CVS
  (per-temporal-unit seen set + immediate re-seed exceedance check). Tests:
  `distinct_mlayer_count_pre_clk_header_reattributed_to_new_cvs_is_flagged`,
  `_reattribution_excludes_pre_boundary_tu_ids`,
  `_reattribution_reports_once_across_clk_in_boundary_tu`, and the refreshed
  `_pre_clk_obu_in_boundary_tu_is_not_flagged`.
- [x] 6.4 Sync the validator spec delta, design decisions 4/5 and risks, and the
  `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT` / `AV2-7.3.8-HLS-AVAILABILITY` matrix-row notes
  to the narrowed gates and re-attribution.
