## 1. OpenSpec Planning

- [x] 1.1 Record planning subagent findings in the change agent log
- [x] 1.2 Validate the proposal, design, spec delta, and tasks with `openspec validate decoded-frame-plane-runtime-types --strict`
- [x] 1.3 Create the implementation branch after OpenSpec validation passes

## 2. Runtime Model Implementation

- [x] 2.1 Add `splot-recon` modules and reexports for errors, AV2-derived format data, geometry, planes, and frames
- [x] 2.2 Implement `BitDepth`, `PixelFormat`, `PlaneId`, `OutputIndex`, dimensions, and visible rectangle types with validating constructors
- [x] 2.3 Implement immutable owned `Plane<T>` with stride, storage size, visible rectangle, checked sample/byte accounting, and visible-row accessors
- [x] 2.4 Implement immutable `DecodedFrame<T>` validation for plane count, crop alignment, AV2-derived chroma dimensions, sample type, and sample range
- [x] 2.5 Keep `splot-recon` independent of other `splot-*` crates and avoid new third-party dependencies

## 3. Tests

- [x] 3.1 Add unit tests for AV2 bit-depth and chroma-format mappings, including reserved-value rejection
- [x] 3.2 Add unit tests for plane stride, buffer length, visible rectangle, checked arithmetic, and padding exclusion behavior
- [x] 3.3 Add unit tests for decoded-frame monochrome/non-monochrome plane presence and shape validation
- [x] 3.4 Add unit tests for crop alignment, sample-type compatibility, and sample-range rejection

## 4. Status and Documentation

- [x] 4.1 Update `docs/DECODER-ROADMAP.md` to reference the new source-backed runtime model without claiming decode support
- [x] 4.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` and regenerate `docs/DECODER-SUPPORT-STATUS.md`
- [x] 4.3 Update `docs/IMPLEMENTATION-MATRIX.toml` and regenerate feature/spec status docs
- [x] 4.4 Keep AVM/dav2d evidence non-executable and do not update local reference evidence unless portable metadata is actually created

## 5. Verification and Review

- [x] 5.1 Run `cargo test -p splot-recon --locked`
- [x] 5.2 Run `cargo xtask check-dependency-direction`
- [x] 5.3 Run `cargo xtask check-decoder-support`
- [x] 5.4 Run `cargo xtask feature-status` and `cargo xtask check-feature-status`
- [x] 5.5 Run `cargo xtask ci`
- [x] 5.6 Run final review subagents for code/tests, AV2/spec/status, and dependency/AVM boundary
- [x] 5.7 Archive the OpenSpec change after implementation and reviews pass
- [ ] 5.8 Open the PR and wait for Codex approval or review before merge
