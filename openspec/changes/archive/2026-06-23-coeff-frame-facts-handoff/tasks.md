## 1. Frame-Facts Handoff

- [x] 1.1 Add crate-private coefficient frame-facts input types for `DECODE-COEFF-FRAME-FACTS-HANDOFF`.
- [x] 1.2 Implement all-zero routing through the existing base-q/shared-facts path without requiring frame facts.
- [x] 1.3 Implement nonzero frame-fact derivation for `enable_fsc`, `enable_chroma_dctonly`, `reduced_tx_set`, `LosslessArray[segmentId]`, and `base_q_idx`.
- [x] 1.4 Return a typed fail-atomic error for invalid segment ids before delegating to lower branch wrappers.

## 2. Parser-Fact Propagation

- [x] 2.1 Thread parsed sequence/frame coefficient facts through crate-private `FrameCandidateTileFacts`, `TileFrameFacts`, and `DecodeTileWorkUnit`.
- [x] 2.2 Add focused derivation tests proving `FrameHeaderCore` facts and active sequence facts reach the tile work unit unchanged.

## 3. Tests

- [x] 3.1 Add focused tests proving all-zero frame-facts input matches the existing all-zero branch behavior.
- [x] 3.2 Add focused tests proving ordinary selected-branch behavior matches explicit base-q delegation.
- [x] 3.3 Add focused tests proving FSC selected-branch behavior matches explicit base-q delegation.
- [x] 3.4 Add focused tests proving invalid segment ids preserve state, CDFs, and symbol state.
- [x] 3.5 Run focused `splot-decode` coefficient-loop and tile-payload derivation tests.

## 4. Tracking and Documentation

- [x] 4.1 Add `DECODE-COEFF-FRAME-FACTS-HANDOFF` to `docs/IMPLEMENTATION-MATRIX.toml` with proof tests and commands.
- [x] 4.2 Add the decoder-support row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 4.3 Update `docs/DECODER-ROADMAP.md` and decoder conformance coverage metadata for the new partial handoff.
- [x] 4.4 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 5. Validation

- [x] 5.1 Run `openspec validate coeff-frame-facts-handoff --strict`.
- [x] 5.2 Run `openspec validate --all --no-interactive`.
- [x] 5.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 5.4 Run `git diff --check`.
- [x] 5.5 Run `cargo xtask audit-scope --all --write-ledger`.
- [x] 5.6 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
