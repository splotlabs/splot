# Tasks

## Matrix and docs

- [x] Update `AV2-5.18.6-QUANTIZATION` notes (the §6.17.6.2 QM layer-dependency constraints now
      land; no §6.17.6.2 QM residual remains), diagnostics list, and proof.
- [x] Register `frame-header/qm-mlayer-dependency-missing` /
      `frame-header/qm-tlayer-dependency-missing` in `docs/VALIDATOR-DIAGNOSTICS.md`; refresh
      the now-stale residual clauses in the `qm-level-unavailable` description.
- [x] Regenerate `docs/FEATURE-STATUS.md` / `docs/SPEC-COVERAGE.md` if affected.

## Implementation

- [x] In `frame_qm_reference_checks` (`context/quantizer_matrix.rs`), within the per-level
      `record` scope, gated on `record.mlayer_id == Some(m)`, fire the two §6.17.6.2 diagnostics
      against `MLayerDependencyMap` / `TLayerDependencyMap`. Remove the deferral TODO.

## Tests and proof

- [x] Negative: `validator_flags_qm_mlayer_dependency_missing` (level at undepended mlayer).
- [x] Negative: `validator_flags_qm_tlayer_dependency_missing` (level at undepended tlayer; the
      mlayer dependency is satisfied so only the tlayer rule fires).
- [x] Positive: `validator_qm_layer_dependency_satisfied_is_silent` (base-layer level).
- [x] Add a `qm_default_level_obu_chroma_at_layer` test builder (QM OBU at a chosen layer).

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
