# Tasks

## Matrix and docs

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` notes,
      diagnostics, and proof; point `openspec_change` at this change. (`validate` stays
      `partial` — the other use_bru clauses and implicit-map slots remain residuals.)
- [x] Register `frame-header/bru-ref-frame-size-mismatch` in `docs/VALIDATOR-DIAGNOSTICS.md`.
- [x] Regenerate `docs/FEATURE-STATUS.md` / `docs/SPEC-COVERAGE.md` if affected.

## Implementation

- [x] In `reference_state_checks` (`context/reference_frames.rs`), add a block gated on
      `core.inter.use_bru == Some(true)` + `Some(core.frame_size)` + `Some(bru_ref)` + a
      bounds-checked `ref_frame_idx.get(bru_ref)` slot that is `SlotState::Valid(facts)`, firing
      `frame-header/bru-ref-frame-size-mismatch` when `facts.{width,height} != frame size`.

## Tests and proof

- [x] Positive: matching BRU ref dims silent; Unknown `bru_ref` slot silent; non-BRU frame
      (no `use_bru`) silent.
- [x] Negative: a BRU dim mismatch fires, and `ref-frame-scale-ratio` stays silent for dims
      within the scale bounds (the two checks are distinct).
- [x] Add a `clk_override_size_small` test builder (small dims, no tile_info increment bits).

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
