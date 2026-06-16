## 1. Recon Primitive

- [x] 1.1 Add `splot-recon` H/V cardinal directional prediction types and primitive with AV2 §7.13.2.8 / §9.2 citations.
- [x] 1.2 Add positive and negative unit tests for H/V output, edge lengths, sample range, sample type, stride, and output length.

## 2. Workspace And Runtime Handoff

- [x] 2.1 Add current-frame workspace helpers for in-storage H/V prediction and missing-edge errors.
- [x] 2.2 Update the minimal runtime reconstruction frontier to use explicit traced chroma `H_PRED` handling with spec-correct hash/Y4M output bytes.
- [x] 2.3 Add or update focused runtime tests proving the traced chroma handoff is used and corrected outputs remain deterministic.

## 3. Fuzz And Documentation

- [x] 3.1 Extend `recon_intra_prediction_bytes` to exercise direct and workspace H/V cardinal cases without new external dependencies.
- [x] 3.2 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated status/coverage docs, and roadmap/spec references with `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION`.
- [x] 3.3 Keep broad intra/reconstruction/conformance rows partial and explicitly document non-goals.

## 4. Verification And Review

- [x] 4.1 Run targeted tests: `cargo test -p splot-recon intra_directional workspace --locked`, `cargo test -p splot-decode runtime_minimal_recon runtime_hash runtime_y4m --locked`, and CLI decode tests.
- [x] 4.2 Run fuzz/build gates: `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`, `cargo xtask check-fuzz-targets`, and a short local `cargo +nightly fuzz run recon_intra_prediction_bytes`.
- [x] 4.3 Run status gates: `openspec validate recon-intra-cardinal-directional-prediction --strict`, `openspec validate --all --no-interactive`, `cargo xtask check-decoder-support`, `cargo xtask check-decoder-conformance-coverage`, `cargo xtask check-feature-status`, and `cargo xtask ci`.
- [x] 4.4 Obtain independent subagent correctness, security/resource, and documentation/status review decisions and address any findings.

## 5. Archive And PR

- [x] 5.1 Archive the OpenSpec change and commit the archive in the same branch.
- [x] 5.2 Re-run required gates after archive.
- [ ] 5.3 Open a ready pull request, wait for green CI and final-head clean review/approval, ensure zero unresolved live review threads, then squash merge.
