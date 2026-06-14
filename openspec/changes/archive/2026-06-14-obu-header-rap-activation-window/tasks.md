# Tasks

## Matrix and docs

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml`
      `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` notes (§ 6.2.2 NOTE residual closed);
      flip `validate` `partial -> done`; add the new proof tests.
- [x] Regenerate `docs/FEATURE-STATUS.md`.

## Implementation

- [x] Add `frame_confirmed_activated_limits: BTreeMap<ExtendedLayerId,
      (TemporalLayerId, EmbeddedLayerId)>` to `ValidatorContext`.
- [x] Snapshot the activated header's `(max_tlayer_id, max_mlayer_id)` on the § 5.18.2
      frame-confirmed activation path in `observe_frame_bearing_obu`.
- [x] `validate_active_sequence_limits` reads the snapshot for frame-confirmed layers, falling
      back to the live store for fallback-only layers.

## Tests and proof

- [x] Negative: an OBU between a tightening § 7.3.6 redefinition and its re-confirming CLK frame,
      within the prior activated `max_tlayer_id`, does not fire `sequence-state/tlayer-exceeds-max`.
- [x] Positive: an OBU in the same window exceeding even the prior activated limit still fires.
- [x] Positive: after the CLK re-confirms the tightened header, a later OBU exceeding the new
      limit fires.
- [x] Add proof tests to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
