# Change: lcr-aggregate-info-annex-a-range

## Feature IDs

- `AV2-5.8.3-LCR-AGGREGATE-INFO`

## Why

Closes the last `validate` residual on `AV2-5.8.3-LCR-AGGREGATE-INFO`. § 6.8.4
(docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-4, lines 1737-1760) states no
reconstruction requirement; its only normative conformance clauses are three Annex-A
value-space constraints on the global LCR's `lcr_aggregate_info()`:

- `lcr_config_idc` shall not take values outside Annex A (lines 1744-1747) — Annex A.3
  Table A.5 defines multi-sequence configurations `0..=2`; `3..=63` are reserved.
- `lcr_aggregate_level_idx` shall not take values outside Annex A (lines 1749-1752) —
  Annex A.4 Table A.7 reserves level indices `22..=30`.
- `lcr_max_interop` shall not take values outside Annex A (lines 1757-1759) — Annex A.3
  Table A.3 defines interoperability points `0`, `1`, `2`, and `15` ("max"); `3..=14` are
  reserved.

`lcr_max_tier_flag` (line 1754) is a 1-bit field with no "shall not contain values outside
Annex A" clause, so it has no value-space check.

Each clause is decidable from the parsed global LCR's `lcr_aggregate_info()` alone — the
requirement is on the bitstream *containing* the value, not on any activation — so these are
local value-space checks in the same family as `annex-a/profile-reserved`. The Annex A
profile/level/tier tables that previously blocked this residual are now modeled
(`annex_a.rs`), so the enforcement is sound and zero-false-positive.

## Scope

- Spec sections: § 6.8.4 (LCR aggregate info semantics).
- Crates/modules: `crates/splot-validate/src/annex_a.rs` (new `is_defined_max_interop`
  helper, Table A.3-verified; existing `is_defined_config_idc` / `is_reserved_level` reused),
  `crates/splot-validate/src/checks/mod.rs` (`check_layer_config_record_semantics` Global
  arm: three value-space diagnostics gated on `lcr_aggregate_info_present_flag == 1`).
- Diagnostics: new `lcr/config-idc-reserved`, `lcr/aggregate-level-idx-reserved`,
  `lcr/max-interop-reserved` (all error, § 6.8.4) registered in
  `docs/VALIDATOR-DIAGNOSTICS.md`.
- Docs: matrix row notes + proof; `validate` advances `partial` -> `done`.

## Non-goals

- The § 6.8.2 MSDO <-> activated-global-LCR aggregate agreement (`lcr/msdo-aggregate-mismatch`
  and siblings) is a separate, already-landed cross-OBU check; this change is purely the
  local § 6.8.4 value-space of the global LCR's own aggregate fields.
- No new value-space modeling of `lcr_max_tier_flag` (no normative clause; 1-bit field).

## Acceptance criteria

- [ ] `AV2-5.8.3-LCR-AGGREGATE-INFO` notes updated and `validate` set to `done` with proof.
- [ ] `lcr/config-idc-reserved`, `lcr/aggregate-level-idx-reserved`, and
      `lcr/max-interop-reserved` registered in `docs/VALIDATOR-DIAGNOSTICS.md`.
- [ ] Negative: a global LCR with a reserved `lcr_config_idc` (3), `lcr_aggregate_level_idx`
      (22), or `lcr_max_interop` (3) fires the corresponding diagnostic.
- [ ] Positive: a global LCR with defined values (config 2, level 31, interop 15) trips none
      of the three.
- [ ] `is_defined_max_interop` is unit-tested against Table A.3.
- [ ] `cargo xtask ci` passes.
