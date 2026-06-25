## 1. Current-Frame Copy Primitive

- [x] 1.1 Add a `RECON-INTRABC-CURRENT-FRAME-COPY` matrix row and implement a checked same-plane current-frame rectangle copy helper on `splot-recon::CurrentFrameWorkspace`.
- [x] 1.2 Make the copy helper validate source/target rectangles and equal shapes before mutation, use bounded scratch storage for overlap safety, and return typed `ReconError` failures.
- [x] 1.3 Add focused `splot-recon` tests for successful luma copy, overlapping source/target copy, out-of-bounds source/target rejection, shape mismatch rejection, missing plane rejection, and no partial mutation on invalid input.

## 2. ac0ej3 IntrABC Prediction Geometry Handoff

- [x] 2.1 Add an ac0ej3 IntrABC luma prediction-geometry helper that converts retained eighth-pel block vectors into checked target rectangles, source sample envelopes, and subpel scaling phase for the current block, rejecting overflowing or out-of-frame geometry with structured diagnostics.
- [x] 2.2 Wire the selectable-transform IntrABC path to derive prediction geometry after NEARMV/NEWMV block-vector syntax and advance the live local probe to a precise missing-populated-`CurrFrame` frontier instead of the generic IntrABC prediction stop.
- [x] 2.3 Preserve existing syntax ordering, CDF updates, transform-record contexts, non-IntrABC behavior, skip/max-rect behavior, and no-output guarantees.
- [x] 2.4 Add focused `splot-decode` tests for NEWMV geometry, NEARMV geometry, fractional luma source-envelope/phase derivation, out-of-frame rejection, non-IntrABC regression behavior, and the live-path missing-`CurrFrame` diagnostic.

## 3. Tracking and Proof

- [x] 3.1 Update `docs/IMPLEMENTATION-MATRIX.toml` and `docs/DECODER-SUPPORT-MATRIX.toml` for `RECON-INTRABC-CURRENT-FRAME-COPY` and `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`.
- [x] 3.2 Regenerate affected decoder support/status and spec coverage documents, and confirm `docs/SPEC-MAPPING.md` already covers or is updated for the cited AV2 sections.
- [x] 3.3 Update the ignored local ac0ej3 CLI probe expectation to the next structured unsupported-feature frontier reached after IntrABC prediction-geometry handoff.
- [x] 3.4 Run focused Rust tests, the ignored local ac0ej3 probe, `openspec validate --all --no-interactive`, `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-fixtures`, `cargo xtask conformance`, and `cargo xtask ci`.
