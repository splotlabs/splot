## 1. Reconstruction Primitive

- [x] 1.1 Add typed `splot-recon` angle/edge API and errors for one-sided directional-angle prediction.
- [x] 1.2 Implement pAngle `45`, `67`, and `203` non-IDIF interpolation with checked validation before output mutation.
- [x] 1.3 Export the new public API and keep crate dependency direction unchanged.

## 2. Tests And Fuzzing

- [x] 2.1 Add unit tests for exact D45, D67, and D203 interpolation and edge-end fallback behavior.
- [x] 2.2 Add negative tests for unsupported pAngles, edge length/range errors, output shape errors, unsupported sample type, and no-mutation guarantees.
- [x] 2.3 Extend `recon_intra_prediction_bytes` with bounded direct cases for supported and unsupported directional-angle inputs.

## 3. Documentation And Status

- [x] 3.1 Add implementation and decoder-support matrix rows with the narrow Feature ID, evidence, exclusions, and test commands.
- [x] 3.2 Regenerate generated decoder support, feature status, and decoder conformance coverage docs.
- [x] 3.3 Update roadmap/conformance notes so broad directional-angle, IDIF, MRL, IBP, edge preparation, and runtime dispatch remain partial.

## 4. Verification

- [x] 4.1 Run targeted `splot-recon` unit tests and fuzz target compile checks.
- [x] 4.2 Run `openspec validate recon-intra-directional-angle-prediction --strict`, `openspec validate --all --no-interactive`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.3 Run `cargo xtask ci` before archiving.

## 5. Archive And PR

- [x] 5.1 Archive the OpenSpec change and rerun the relevant post-archive gates.
- [ ] 5.2 Commit, push, open a ready PR, and wait for green CI plus current-head approval with no unresolved live review threads.
