# Tasks

## Matrix and docs

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` notes,
      diagnostics, and proof; point `openspec_change` at this change. (`validate` stays
      `partial` — implicit-map slots and the other reference-state checks remain residuals.)
- [x] Register `frame-header/ref-frame-scale-ratio` in `docs/VALIDATOR-DIAGNOSTICS.md`.
- [x] Regenerate `docs/FEATURE-STATUS.md` / `docs/SPEC-COVERAGE.md` if affected.

## Implementation

- [x] In `reference_state_checks` (`context/reference_frames.rs`), add a block gated on
      `core.inter` + `Some(core.frame_size)` that iterates `inter.ref_frame_idx`, matches
      `SlotState::Valid(facts)`, and fires `frame-header/ref-frame-scale-ratio` on the first
      violated §6.17.2 inequality (saturating products; one diagnostic per frame).

## Tests and proof

- [x] Positive: 1:1 ratio silent; 2x-upscale boundary (`2*FrameWidth == RefFrameWidth`)
      silent; Unknown slot silent; ProvenInvalid slot fires only `ref-frame-idx-invalid-slot`.
- [x] Negative: width too small, height too small, width too large, height too large each fire.
- [x] Add a `inter_frame_explicit_size` test builder (explicit `found_ref == 0` dims).

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
