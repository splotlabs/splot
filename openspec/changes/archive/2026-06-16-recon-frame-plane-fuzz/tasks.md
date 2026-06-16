## 1. Feature Tracking And OpenSpec

- [x] 1.1 Add `CONF-RECON-FRAME-PLANE-TYPES-FUZZ` to `docs/IMPLEMENTATION-MATRIX.toml` with scoped proof entries.
- [x] 1.2 Add `recon-frame-plane-types-fuzz` to `docs/DECODER-SUPPORT-MATRIX.toml` without broadening runtime decode, reconstruction, output scheduling, reference-refresh, or resource-diagnostic claims.
- [x] 1.3 Run `openspec validate recon-frame-plane-fuzz --strict`.

## 2. Fuzz Target

- [x] 2.1 Add the `recon_frame_plane_types_bytes` bin to `fuzz/Cargo.toml` without adding third-party dependencies.
- [x] 2.2 Implement `fuzz/fuzz_targets/recon_frame_plane_types_bytes.rs` with bounded frame/plane model generation and targeted invalid mutations.
- [x] 2.3 Exercise bit-depth/chroma-idc mapping, geometry/crop/stride/backing validation, plane presence and visible-size checks, sample storage/range checks, visible rows, borrowed views, and `SharedFrame` invariants.

## 3. Documentation And Generated Status

- [x] 3.1 Update `AGENTS.md`, `.github/workflows/ci.yml`, `docs/TESTING.md`, and related fuzz target lists for the frame/plane fuzz target.
- [x] 3.2 Update decoder conformance coverage metadata for frame/plane fuzz coverage.
- [x] 3.3 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Review And Gates

- [x] 4.1 Run targeted gates: `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`, `cargo xtask check-fuzz-targets`, `cargo test -p splot-recon frame --locked`, `cargo test -p splot-recon plane --locked`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.2 Run a short local `cargo +nightly fuzz run recon_frame_plane_types_bytes` smoke when cargo-fuzz is available.
- [x] 4.3 Run independent review for fuzz target scope, resource bounds, status honesty, and no-panic arbitrary-byte behavior.
- [x] 4.4 Run `openspec validate --all --no-interactive`, `cargo xtask ci`, and `git diff --check`.

## 5. Archive And PR

- [x] 5.1 Archive the OpenSpec change with `openspec archive recon-frame-plane-fuzz --yes` and commit the archive in this branch.
- [x] 5.2 Re-run relevant gates after archive.
- [ ] 5.3 Open a ready, non-draft PR with Feature ID, scoped fuzz behavior, tests, reviewer decisions, and known exclusions.
