## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for the directional UV handoff.
- [x] 1.3 Update `docs/DECODER-ROADMAP.md` and regenerate generated status/coverage docs.

## 2. Implementation

- [x] 2.1 Extend the ordinary branch `Mode_To_Txfm` config chain with caller-resolved `AngleDeltaUV`.
- [x] 2.2 Implement the AV2 section 5.20.7.29 directional `UVMode` `wide_angle_mapping` path before `Mode_To_Txfm` lookup.
- [x] 2.3 Preserve existing all-zero, non-directional, chroma-DCT-only, and unsupported-subset behavior.

## 3. Tests and Validation

- [x] 3.1 Add focused ordinary branch tests for directional no-remap, wide-angle remap, transform-set fallback, and fail-atomic invalid domains.
- [x] 3.2 Run focused coefficient-loop tests and OpenSpec/tracking checks.
- [x] 3.3 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
