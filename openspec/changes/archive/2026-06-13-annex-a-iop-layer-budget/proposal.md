# Change: annex-a-iop-layer-budget

## Feature IDs

- `AV2-A-PROFILES`

## Why

The Annex A IOP-window machinery already determines each coded (multistream) video
sequence's interoperability point (from the profile / `multistream_profile_idc`) and counts
its extended and embedded layers, and enforces the Table A.4 OBU-presence rules. But it does
not enforce the **Table A.3 layer budget** for that IOP — a documented `TODO(spec:
AV2-A-LEVELS-TIERS)` in `evaluate_annex_a_iop_window`. Table A.3 caps, per IOP, the Number of
Extended Layers, the Number of Embedded Layers, and whether the Extended-and-Embedded
combination is permitted. An IOP1 window with both more than one extended layer and more than
one embedded layer is the clearest gap: it exceeds the budget yet has no Table A.4 row, so
nothing currently flags it.

This closes one of the four `AV2-A-PROFILES` validate residuals (the Table A.3 layer-budget
bound for IOP 0/1, extended to IOP2 for completeness).

## Scope

- Spec sections: Annex A Table A.3 (interoperability-point layer budgets, mirror lines
  125-170).
- Crates/modules: `crates/splot-validate/src/context/annex_a_iop.rs`
  (`evaluate_annex_a_iop_window` + a new `emit_iop_layer_budget` helper). New diagnostic
  `annex-a/layer-budget-exceeds-iop`.

## Non-goals

- The Table A.3 "Number of Layers" (sum of embedded counts across singlestreams) bound — the
  per-singlestream sum is not tracked; a named residual.
- The other three `AV2-A-PROFILES` residuals (Configurable-profile derivation, Table A.5
  implicit multi-sequence configuration, the profile newtype enum) — separate changes.

## Acceptance criteria

- [ ] `AV2-A-PROFILES` notes updated (Table A.3 budget landed; remaining residuals re-stated).
- [ ] `annex-a/layer-budget-exceeds-iop` registered in `docs/VALIDATOR-DIAGNOSTICS.md`.
- [ ] Negative: an IOP0 CVS with >1 embedded layer, and an IOP1 CVS with the E && M
      combination, each fire the diagnostic.
- [ ] Positive: a within-budget IOP0 CVS, and an IOP2 E && M CVS (combination permitted),
      stay silent.
- [ ] Suppression: a Provided external-HLS mode suppresses the check.
- [ ] Zero false positives: the counts are conservative lower bounds; the IOP is
      table-determined (reserved / Configurable / disagreeing profiles are skipped).
- [ ] `cargo xtask ci` passes.
