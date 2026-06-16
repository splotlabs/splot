## 1. Reconstruction Primitive

- [x] 1.1 Add typed `splot-recon` middle-angle/edge API and errors for logical prepared edges.
- [x] 1.2 Implement pAngles `113`, `135`, and `157` non-IDIF interpolation with checked signed arithmetic and validation before output mutation.
- [x] 1.3 Export the new public API and keep crate dependency direction unchanged.

## 2. Tests And Fuzzing

- [x] 2.1 Add unit tests for exact pAngle `113`, `135`, and `157` interpolation, including above-branch and left-branch negative logical indices.
- [x] 2.2 Add negative tests for unsupported pAngles, missing/short logical edge coverage, edge sample range errors, sample type mismatch, output shape errors, and no-mutation guarantees.
- [x] 2.3 Extend `recon_intra_prediction_bytes` with bounded direct cases for supported and unsupported middle-angle inputs.

## 3. Documentation And Status

- [x] 3.1 Add implementation and decoder-support matrix rows with the narrow Feature ID, evidence, exclusions, and test commands.
- [x] 3.2 Regenerate generated decoder support, feature status, and decoder conformance coverage docs.
- [x] 3.3 Update roadmap/conformance notes so broad directional-angle, edge preparation, IDIF, MRL, IBP, and runtime dispatch remain partial.

## 4. Review

- [x] 4.1 Complete spec-mapper, decoder-architect, and security planning reviews before implementation.
- [x] 4.2 Complete independent correctness, security, performance, and docs/tests reviews after implementation.
- [x] 4.3 Fix or explicitly document the disposition of every actionable review finding.

## 5. Verification

- [x] 5.1 Run targeted `splot-recon` unit tests and fuzz target compile checks.
- [x] 5.2 Run `openspec validate recon-intra-middle-directional-angle-prediction --strict`, `openspec validate --all --no-interactive`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 5.3 Run `cargo xtask ci` before archiving.

## 6. Archive And PR

- [x] 6.1 Archive the OpenSpec change and rerun the relevant post-archive gates.
- [ ] 6.2 Commit, push, open a ready PR, and wait for green CI plus current-head approval with no unresolved live review threads.
