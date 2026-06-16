## 1. Feature Tracking And OpenSpec

- [x] 1.1 Add `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` to `docs/IMPLEMENTATION-MATRIX.toml` with scoped spec sections and proof placeholders.
- [x] 1.2 Add `tile-cdf-save-lifecycle-boundary` to `docs/DECODER-SUPPORT-MATRIX.toml` and keep adjacent CDF rows honest about partial full-lifecycle coverage.
- [x] 1.3 Run `openspec validate tile-cdf-save-lifecycle-boundary --strict`.

## 2. CDF Lifecycle Implementation

- [x] 2.1 Add supported-subset row-walk helpers for copying/averaging saved rows and scaling frame-end CDF counts.
- [x] 2.2 Add crate-private APIs that apply a completed tile CDF subset to saved state only after successful `exit_symbol()`.
- [x] 2.3 Add subset frame-end update behavior that copies saved rows into frame rows and scales each row count with `(3 * count) >> 2`.
- [x] 2.4 Wire the minimal runtime frontier through the lifecycle boundary without changing output bytes.

## 3. Tests

- [x] 3.1 Add CDF unit tests for copy policy, average policy, disabled update behavior, and frame-end count scaling across partition and block rows.
- [x] 3.2 Add rollback tests showing symbol mismatch or `exit_symbol()` failure leaves saved/frame CDF state unchanged.
- [x] 3.3 Re-run minimal runtime hash/Y4M tests to prove output identity is unchanged.

## 4. Documentation And Generated Status

- [x] 4.1 Update decoder roadmap/support notes for the lifecycle boundary and non-goals.
- [x] 4.2 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, and `docs/DECODER-SUPPORT-STATUS.md`.

## 5. Review And Gates

- [x] 5.1 Run targeted gates: `cargo test -p splot-decode tile_payload::cdf --locked`, `cargo test -p splot-decode runtime_hash --locked`, `cargo test -p splot-decode runtime_y4m --locked`, `cargo xtask check-decoder-support`, and `cargo xtask check-feature-status`.
- [x] 5.2 Run independent subagent reviews for spec exactness, security/transactionality, and performance/data layout.
- [x] 5.3 Run `openspec validate --all --no-interactive` and `cargo xtask ci`.

## 6. Archive And PR

- [x] 6.1 Archive the OpenSpec change with `openspec archive tile-cdf-save-lifecycle-boundary --yes` and commit the archive in this branch.
- [x] 6.2 Re-run relevant gates after archive.
- [ ] 6.3 Open a ready, non-draft PR with spec sections, matrix rows, tests, reviewer decisions, and known exclusions.
