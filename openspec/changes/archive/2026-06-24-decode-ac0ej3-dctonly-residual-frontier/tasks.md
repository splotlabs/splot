## 1. DCT-only Residual Admission

- [x] 1.1 Add a spec-derived helper for deciding when an ac0ej3 transform-record residual resolves to `DCT_DCT` without unsupported transform-type/CCTX/IST/FSC syntax.
- [x] 1.2 Wire selectable luma/chroma residual calls to admit DCT-only nonzero residuals and retain structured fail-closed diagnostics for non-DCT active transform syntax.
- [x] 1.3 Add focused positive and negative tests for DCT-only residual admission, active luma transform-type mapping, non-DCT rejection, and unchanged all-zero behavior.

## 2. ac0ej3 Probe and Tracking

- [x] 2.1 Update the local ignored ac0ej3 CLI probe to the new frontier.
- [x] 2.2 Add `DECODE-AC0EJ3-DCTONLY-RESIDUAL-FRONTIER` implementation and decoder-support matrix rows, then regenerate status docs.
- [x] 2.3 Sync OpenSpec main specs and archive the change after all tasks and gates are complete.

## 3. Verification

- [x] 3.1 Run focused unit/CLI tests plus the local ac0ej3 probe.
- [x] 3.2 Run `openspec validate --all --no-interactive`, `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask ci`.
