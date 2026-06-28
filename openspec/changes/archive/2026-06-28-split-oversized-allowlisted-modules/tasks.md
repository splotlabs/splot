## 1. Planning And Feature Tracking

- [x] 1.1 Validate this OpenSpec change (`openspec validate split-oversized-allowlisted-modules --strict`).
- [x] 1.2 Add `INFRA-MODULE-SIZE-REFACTOR` to `docs/IMPLEMENTATION-MATRIX.toml` (category/kind `infrastructure`, `openspec_change = "split-oversized-allowlisted-modules"`, risk `low`).

## 2. celu.rs split (this change's implementation)

- [x] 2.1 Confirm the in-file `#[test]` module touches only `pub(crate)`/`pub` surface (no private-field reach) before moving.
- [x] 2.2 Move the in-file test module to `crates/splot-validate/src/celu/tests.rs` (declared `#[cfg(test)] mod tests;`); keep shared synthetic-OBU builders with the tests.
- [x] 2.3 Remove the `crates/splot-validate/src/celu.rs` entry from `HARD_LINE_ALLOWANCES` in `xtask/src/source_lines.rs`.
- [x] 2.4 `cargo fmt -p splot-validate` to canonicalize the relocated tests; confirm all 71 celu tests still pass (`cargo test -p splot-validate --locked`).
- [x] 2.5 `cargo xtask check-source-lines` passes with `celu.rs` under the hard cap and no allowance problem.

## 3. Gates, Review, And PR Discipline

- [x] 3.1 Run `cargo xtask check-feature-status` and `cargo xtask ci` bare (exit 0) before committing.
- [x] 3.2 Keep this PR scoped to the celu split (test relocation, allowance edit, matrix row, OpenSpec artifacts); no opportunistic restructuring.
- [x] 3.3 Sync the `module-size-discipline` spec to `openspec/specs/` and archive this change.
- [ ] 3.4 Create a ready (non-draft) PR; request review and wait for the completed latest-head review (both AI reviewers) before merge.

## Follow-up (tracked as separate changes under the same Feature ID + `module-size-discipline` capability)

These were analyzed and planned in this change's `design.md` but are intentionally
out of scope for this PR (one allowlisted file per PR; actively-developed files
deferred):

- `sequence.rs` → `sequence/` directory: extract `profile.rs`, `layer_dependency.rs`
  (all three maps together), and `child_configs.rs` (~860-line §5.4.x mass); re-export
  every moved public item (~70 importers via `crate::headers::sequence::…`).
- `frame/info.rs` (DEFERRED — active decoder frontier): split `status` + `show_existing`
  + `seq_view` once stable. Not the proposal's `activation`/`inter_control`/`shared_tail`.
- `wienerns_lr/tx_records.rs` (DEFERRED — active ac0ej3 frontier): split `per_block_syntax`
  (DeltaQ/CDEF) + `tx_partition` + `error` once stable. Not chroma-first.
