## 1. Recon Primitive

- [x] 1.1 Split shared DC prediction math/output helpers out of `intra.rs` so existing square/rectangular DC behavior remains unchanged and line budgets stay healthy.
- [x] 1.2 Add the §7.13.2.11 subsampled DC primitive and public exports with AV2 citations.
- [x] 1.3 Add positive and negative unit tests for no-edge midpoint, stepped large-edge averaging, approximate division, 8-bit/10-bit ranges, invalid edge lengths, invalid samples, stride, and output length.

## 2. Workspace And Fuzz Handoff

- [x] 2.1 Add current-frame workspace subsampled DC helper without deciding full §7.13.2.1 edge availability or runtime dispatch.
- [x] 2.2 Add workspace tests for in-storage edge prediction, no-edge midpoint, missing plane, and out-of-bounds geometry.
- [x] 2.3 Extend `recon_intra_prediction_bytes` to exercise direct and workspace subsampled DC cases without external dependencies.

## 3. Documentation And Status

- [x] 3.1 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, roadmap text, and conformance coverage metadata with `RECON-INTRA-DC-SUBSAMPLED-PREDICTION`.
- [x] 3.2 Regenerate generated status/coverage docs and keep broad intra/reconstruction/conformance rows partial.
- [x] 3.3 Validate the OpenSpec change and all repo specs.

## 4. Verification And Review

- [x] 4.1 Run targeted tests and checks for `splot-recon`, fuzz target compilation/enumeration, source-line budgets, dependency direction, concurrency policy, feature status, decoder support, and decoder conformance coverage.
- [x] 4.2 Run `cargo xtask ci`.
- [x] 4.3 Obtain independent subagent correctness, security/resource, performance, and documentation/status review decisions and address any findings.

## 5. Archive And PR

- [x] 5.1 Archive the OpenSpec change and commit the archive in the same branch.
- [x] 5.2 Re-run required gates after archive.
- [ ] 5.3 Open a ready pull request, wait for green CI and final-head clean review/approval, ensure zero unresolved live review threads, then squash merge.
