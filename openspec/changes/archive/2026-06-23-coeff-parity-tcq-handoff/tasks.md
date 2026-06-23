## 1. Frame-Fact Propagation

- [x] 1.1 Add parsed `allow_tcq` and `allow_parity_hiding` to the crate-private coefficient frame-facts packet for `DECODE-COEFF-PARITY-TCQ-HANDOFF`.
- [x] 1.2 Thread the parsed flags through `FrameCandidateTileFacts`, `TileFrameFacts`, and `DecodeTileWorkUnit`.
- [x] 1.3 Add focused derivation tests proving `FrameHeaderCore` lossless-tail flags reach the tile work unit unchanged.

## 2. Coefficient Branch Derivation

- [x] 2.1 Derive ordinary lower-branch `parity_hiding` from frame `allow_parity_hiding`, lossless, plane, and `PlaneTxType`.
- [x] 2.2 Derive ordinary lower-branch `use_tcq` from frame `allow_tcq`, lossless, plane, transform class, and derived `useFsc`.
- [x] 2.3 Preserve all-zero routing without requiring frame, ordinary, or FSC facts.
- [x] 2.4 Preserve invalid-segment fail-atomic behavior before lower branch delegation.

## 3. Tests

- [x] 3.1 Add focused tests proving parity-hiding derivation matches explicit base-q delegation.
- [x] 3.2 Add focused tests proving TCQ derivation matches explicit base-q delegation.
- [x] 3.3 Add focused tests proving lossless, chroma, IDTX/FSC, and non-2D transform conditions suppress the derived flags.
- [x] 3.4 Run focused `splot-decode` coefficient-loop and tile-payload derivation tests.

## 4. Tracking and Documentation

- [x] 4.1 Add `DECODE-COEFF-PARITY-TCQ-HANDOFF` to `docs/IMPLEMENTATION-MATRIX.toml` with proof tests and commands.
- [x] 4.2 Add the decoder-support row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 4.3 Update `docs/DECODER-ROADMAP.md` and decoder conformance coverage metadata for the new partial handoff.
- [x] 4.4 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 5. Validation

- [x] 5.1 Run `openspec validate coeff-parity-tcq-handoff --strict`.
- [x] 5.2 Run `openspec validate --all --no-interactive`.
- [x] 5.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 5.4 Run `git diff --check`.
- [x] 5.5 Run `cargo xtask audit-scope --all --write-ledger`.
- [x] 5.6 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
