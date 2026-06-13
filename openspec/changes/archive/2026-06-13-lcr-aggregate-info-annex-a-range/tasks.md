# Tasks

## Matrix and docs

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` `AV2-5.8.3-LCR-AGGREGATE-INFO` notes; set
      `validate = "done"`; add proof tests/commands; point `openspec_change` at this change.
- [x] Register `lcr/config-idc-reserved`, `lcr/aggregate-level-idx-reserved`, and
      `lcr/max-interop-reserved` in `docs/VALIDATOR-DIAGNOSTICS.md` (lcr/ namespace).
- [x] Regenerate `docs/FEATURE-STATUS.md` / `docs/SPEC-COVERAGE.md` if affected.

## Implementation

- [x] Add `annex_a::is_defined_max_interop` (Table A.3: defined `{0,1,2,15}`; `3..=14`
      reserved), with a table-verified unit test.
- [x] In `check_layer_config_record_semantics` Global arm, gated on
      `global.aggregate_info.is_some()`, fire:
  - [x] `lcr/config-idc-reserved` when `!is_defined_config_idc(config_idc)`.
  - [x] `lcr/aggregate-level-idx-reserved` when `is_reserved_level(aggregate_level_idx)`.
  - [x] `lcr/max-interop-reserved` when `!is_defined_max_interop(max_interop)`.

## Tests and proof

- [x] Negative: reserved `lcr_config_idc` (3) -> `lcr/config-idc-reserved`.
- [x] Negative: reserved `lcr_aggregate_level_idx` (22) -> `lcr/aggregate-level-idx-reserved`.
- [x] Negative: reserved `lcr_max_interop` (3) -> `lcr/max-interop-reserved`.
- [x] Positive: defined values (config 2, level 31, interop 15) trip none of the three.
- [x] `is_defined_max_interop` unit test against Table A.3.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
