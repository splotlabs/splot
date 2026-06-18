## 1. Frame Input API

- [x] 1.1 Add encode-local frame metadata, borrowed plane input, `Frame<'a>`, and `RetainedFrame` types backed by validated `splot-recon` views.
- [x] 1.2 Extend encoder errors for unsupported input formats, missing or unexpected planes, plane geometry failures, and chroma-size mismatches.
- [x] 1.3 Update `Context::send_frame` and crate exports to use the real borrowed frame input while preserving unimplemented lifecycle behavior.

## 2. Tests and Fuzzing

- [x] 2.1 Add positive unit tests for valid 8-bit YUV420 input, odd visible luma dimensions, stride padding, typed identity, and timestamp metadata.
- [x] 2.2 Add negative/property-style tests for truncated buffers, too-small strides, missing chroma planes, unsupported formats, and derived chroma-size mismatches.
- [x] 2.3 Add a cargo-fuzz target that exercises frame-input construction over bounded dimensions, strides, and truncated buffers.

## 3. Tracking and Documentation

- [x] 3.1 Update `ENC-Y4M-INPUT` in `docs/IMPLEMENTATION-MATRIX.toml` with partial frame-input-view status and proof commands.
- [x] 3.2 Regenerate `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`.
- [x] 3.3 Validate OpenSpec artifacts for this change and all specs.

## 4. Verification and Review

- [x] 4.1 Run targeted tests and fuzz smoke for the new input-view target.
- [x] 4.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, and `cargo xtask ci`.
- [x] 4.3 Run local correctness, security/zero-copy, determinism, and tests/evidence reviewer passes and address findings.
- [ ] 4.4 Archive the OpenSpec change before merge, then complete final GitHub Claude/Codex review and green-check merge gates.
