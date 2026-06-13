# Tasks

## Matrix and docs

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` `AV2-A-PROFILES` notes (Table A.3 budget
      landed; remaining residuals re-stated) and `openspec_change`.
- [x] Register `annex-a/layer-budget-exceeds-iop` in `docs/VALIDATOR-DIAGNOSTICS.md`.
- [x] Regenerate `docs/FEATURE-STATUS.md`.

## Implementation

- [x] Add `emit_iop_layer_budget` to `annex_a_iop.rs` and call it from
      `evaluate_annex_a_iop_window` (replacing the `TODO(spec: AV2-A-LEVELS-TIERS)`).
- [x] Per the window's table-determined IOP, check Number of Extended Layers (<= 4), Number
      of Embedded Layers (1 / 2 / 3 for IOP0 / IOP1 / IOP2), and the Extended-and-Embedded
      combination (forbidden for IOP0/IOP1, permitted for IOP2).
- [x] Reuse the conservative lower-bound counts already computed for Table A.4 (zero false
      positives); reuse the `annex_a_iop_error` builder (spec section A.2).

## Tests and proof

- [x] Negative: IOP0 with >1 embedded layer fires; IOP1 with E && M fires.
- [x] Positive: a within-budget IOP0 CVS stays silent; an IOP2 E && M CVS stays silent.
- [x] Suppression: a Provided external-HLS mode suppresses the check.
- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
