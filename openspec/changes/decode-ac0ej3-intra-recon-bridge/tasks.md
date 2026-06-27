## 1. Reconstruction Bridge

- [x] 1.1 Add a `wienerns_lr/recon.rs` reconstruction sink that owns a `CurrentFrameWorkspace<u16>` sized to the ac0ej3 frame and reconstructs verified NON-IntrABC general-intra DC blocks in walk order.
- [x] 1.2 Thread the sink through the selectable residual-chunk decode so each decoded luma `LumaCoeffBlock` and chroma group is reconstructed where it is parsed, reusing the existing `runtime_minimal_recon` primitives.
- [x] 1.3 Gate reconstruction to the supported subset (DC luma + DC chroma, rectangular transforms); leave any other mode, IntrABC block, or unsupported transform geometry unreconstructed rather than emitting wrong samples.
- [x] 1.4 Keep the public `splot decode ac0ej3` path fail-closed at the first active IntrABC block (no partial frame emitted as success).

## 2. Tests

- [x] 2.1 Add an infrastructure test that drives the selectable walk with the sink and asserts the bridge reconstructs a workspace region in range, while the public decode stays fail-closed.
- [x] 2.2 Add the bit-exact region-verification test against a committed oracle assertion (FNV-1a-64 + sum + flat value of the frame-origin `DC_PRED` 16x16 luma leaf, not the 6 MB YUV); the test PASSES now that PR #497's CCSO read makes the first-superblock parse AVM-faithful (first bit-exact ac0ej3 reconstruction milestone). It is `#[ignore]`d only because it needs the local mission fixture.

## 3. Tracking and Proof

- [x] 3.1 Update `docs/IMPLEMENTATION-MATRIX.toml` and `docs/DECODER-SUPPORT-MATRIX.toml` for `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS` with the bit-exact frame-origin DC luma block evidence.
- [x] 3.2 Regenerate decoder support/status docs affected by the matrix updates.
- [x] 3.3 Run the focused Rust tests, the ignored local ac0ej3 probe, `openspec validate --all --no-interactive`, `cargo xtask conformance`, `cargo xtask ci`, and `dupehound check --diff $(git merge-base HEAD origin/main) .`.
