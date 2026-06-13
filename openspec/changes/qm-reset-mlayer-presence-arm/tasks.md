# Tasks

## Matrix and docs

- [x] Update `AV2-5.4.1-SEQUENCE-HEADER-GENERAL` (MLayerPresenceMap now derived/exposed +
      consumed), `AV2-5.18.6-QUANTIZATION` and `AV2-5.13-QUANTIZATION-MATRIX` (the SWITCH/RAS
      reset_qm() MLayerPresenceMap arm now modeled); add proof tests.
- [x] No `VALIDATOR-DIAGNOSTICS.md` change (reuses `frame-header/qm-level-unavailable`).
- [x] Regenerate `docs/FEATURE-STATUS.md` / `docs/SPEC-COVERAGE.md` if affected.

## Implementation

- [x] `MLayerDependencyMap::presence_map() -> MLayerPresenceMap` (§5.4.1 :583-601 closure, with
      a temp to avoid aliasing) + `MLayerPresenceMap::is_present()`; closure unit test.
- [x] `reset_qm_availability_for_switch_or_ras(obu_mlayer_id, presence)` adds the
      `MLayerPresenceMap[m][obu_mlayer_id]` arm for `QmMLayerId == Some(m)` levels (unresolved
      presence -> leave available).
- [x] `apply_qm_reset_for_frame` derives the presence map from the resolved sequence header
      (owned, so the immutable borrow ends before the mutable `self.qm` reset).

## Tests and proof

- [x] splot-core: `mlayer_presence_map_closes_transitive_dependency` (reflexive + transitive).
- [x] Single-layer reflexive reset fires: `validator_qm_confirmed_ras_reset_clears_same_layer_level_via_presence_map`.
- [x] Cross-layer survival silent: `validator_qm_confirmed_ras_reset_preserves_cross_layer_level_via_presence_map`.
- [x] Reconstruct the QM RAP-replay disjointness test multi-layer
      (`rap_replay_qm_level_only_before_rap_is_flagged`): layer-0 level survives a layer-1 RAS.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
