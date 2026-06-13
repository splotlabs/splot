# Change: annex-a-profile-newtype

## Feature IDs

- `AV2-A-PROFILES`

## Why

`AV2-A-PROFILES` carries `types=todo`: the profile identifier was modeled as an opaque
`ProfileIdc(u8)` newtype, so the named-profile semantics (which `seq_profile_idc` /
`multistream_profile_idc` values are defined Main profiles, which are reserved, which is the
Configurable profile) lived only as raw-`u8` comparisons in the validator. The MSDO, OPS,
and LCR profile fields were even barer — plain `u8`. This closes the types stage with a
strong `ProfileIdc` enum over the Annex A.2 Table A.1 value space, used at every profile
field's public boundary.

## Scope

- Spec sections: Annex A.2 Table A.1 (the shared `seq_profile_idc` /
  `multistream_profile_idc` value space, mirror lines 59-90).
- Crates/modules: `crates/splot-core/src/headers/sequence.rs` (`ProfileIdc` enum),
  `crates/splot-core/src/hls.rs` (MSDO `multistream_profile_idc`),
  `crates/splot-core/src/headers/operating_point_set.rs` (`ops_seq_profile_idc`),
  `crates/splot-core/src/headers/layer_config_record.rs` (`lcr_seq_profile_idc`). Validator
  consumers extract `.get()` at the boundary (the `ChromaFormatIdc` pattern).

## Non-goals

- No new or changed validation behavior — `from_bits`/`get` round-trip and the `Ord`
  ordering are preserved (variants declared in `seq_profile_idc` order), so this is purely a
  type refactor.
- The other `AV2-A-PROFILES` residuals (Configurable-profile derivation, Table A.5
  multi-sequence configuration, the Table A.3 Number-of-Layers sum bound) — separate changes.

## Acceptance criteria

- [ ] `ProfileIdc` is an enum over Table A.1 (`Main420Ip0` .. `Main444Ip1`, `Reserved(u8)`,
      `Configurable`) with `from_bits`/`get`/`is_reserved`/`is_configurable`.
- [ ] MSDO / OPS / LCR profile fields are `ProfileIdc`.
- [ ] `from_bits`/`get` round-trip every 5-bit value; `Ord` matches raw-value order.
- [ ] `AV2-A-PROFILES` `types` stage is `done`.
- [ ] No behavior change: the full validator/core suites pass unchanged.
- [ ] `cargo xtask ci` passes.
