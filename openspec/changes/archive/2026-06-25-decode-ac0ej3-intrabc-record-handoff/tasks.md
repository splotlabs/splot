## 1. IntrABC Mode-Info Handoff

- [x] 1.1 Add a bounded IntrABC mode-info helper for the ac0ej3 selectable-transform path that consumes observed AV2 §5.20.5.3 `use_intrabc` and §5.20.5.4 `read_intrabc_info()` syntax in order.
- [x] 1.2 Thread retained IntrABC facts into selectable transform-size and residual context selection without widening decoded sample support.
- [x] 1.3 Preserve the existing `use_intrabc = 0` luma/shared prelude, mode, transform partition, and residual handoff behavior.
- [x] 1.4 Keep unsupported IntrABC prediction/reconstruction branches fail-closed with structured diagnostics.

## 2. Tests

- [x] 2.1 Add focused symbol-order tests for active IntrABC mode-info metadata retention and unsupported-branch diagnostics.
- [x] 2.2 Add focused regression tests proving non-IntrABC selectable-transform records still follow the previous ordinary intra path.
- [x] 2.3 Update the ignored local ac0ej3 CLI probe expectation to the next structured unsupported-feature frontier reached after IntrABC syntax consumption.

## 3. Tracking and Proof

- [x] 3.1 Update `docs/IMPLEMENTATION-MATRIX.toml` and `docs/DECODER-SUPPORT-MATRIX.toml` for `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`.
- [x] 3.2 Regenerate decoder support/status docs affected by the matrix updates.
- [x] 3.3 Run the focused Rust tests, the ignored local ac0ej3 probe, `openspec validate --all --no-interactive`, `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-fixtures`, `cargo xtask conformance`, and `cargo xtask ci`.
