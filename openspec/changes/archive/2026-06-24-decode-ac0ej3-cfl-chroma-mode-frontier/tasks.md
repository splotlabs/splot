## 1. CfL CDF Rows

- [x] 1.1 Add generated CfL index, sign, alpha, MHCCP, and MH direction rows to the tile CDF subset with checked selectors.
- [x] 1.2 Include the new rows in tile CDF defaults, mutable selection, copy/average lifecycle, frame-end scaling, and focused tests.

## 2. CfL Mode And Alpha Syntax

- [x] 2.1 Extend general intra chroma mode facts so active `UV_CFL_PRED` is represented as a typed mode value.
- [x] 2.2 Implement AV2 §5.20.7.32 `read_cfl_alphas()` symbol consumption with sign/alpha context tests and EOF/error coverage.

## 3. ac0ej3 Runtime Handoff

- [x] 3.1 Thread the typed CfL chroma mode through the ac0ej3 selectable-transform record path and coefficient residual handoff.
- [x] 3.2 Update structured diagnostics so the former active-CfL mode gate advances to the next honest unsupported frontier without output.

## 4. Tracking And Verification

- [x] 4.1 Add the `DECODE-AC0EJ3-CFL-CHROMA-MODE-FRONTIER` matrix/support rows and refresh generated status docs.
- [x] 4.2 Run focused tests, the local ac0ej3 decode probe, `openspec validate --all --no-interactive`, `cargo xtask feature-status`, `cargo xtask check-feature-status`, and `cargo xtask ci`.
