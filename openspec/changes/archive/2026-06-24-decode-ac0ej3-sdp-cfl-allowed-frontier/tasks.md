## 1. SDP State And Mode-Info Handoff

- [x] 1.1 Retain AV2 §5.20.3.1 `CflAllowedInSdp` state during intra SDP partition traversal and expose it on chroma-part block frontiers.
- [x] 1.2 Thread the frontier state into §5.20.5.6 chroma mode-info decoding so disabled SDP CfL/MHCCP syntax is not read.
- [x] 1.3 Add focused traversal and mode-info tests that prove `CflAllowedInSdp == 0` skips `is_cfl` and keeps the following `uv_mode` read synchronized.

## 2. ac0ej3 Runtime Handoff

- [x] 2.1 Add `DECODE-AC0EJ3-INTRA-PRELUDE-TX-FRONTIER` for the luma/shared mode-info prelude and transform-record synchronization brick.
- [x] 2.2 Gate the LR transform-record handoff before tile decode for unsupported mode/coeff/filter tools, while allowing only the ac0ej3 syntax this path intentionally consumes.
- [x] 2.3 Consume the observed zero `use_intrabc`, CDEF strength-index, and delta-Q prelude syntax in AV2 §5.20.5.3 order before luma mode and §5.20.6 transform partition parsing.
- [x] 2.4 Reject chroma-offset leaves before deriving chroma residual coordinates from the wrong luma leaf.
- [x] 2.5 Verify the local ac0ej3 probe advances past `unsupported_wienerns_lr_live_transform_record_uv_mode`.
- [x] 2.6 Update the local ac0ej3 CLI regression to name the new live frontier without claiming output.

## 3. Tracking And Verification

- [x] 3.1 Add `DECODE-AC0EJ3-SDP-CFL-ALLOWED-FRONTIER` and `DECODE-AC0EJ3-INTRA-PRELUDE-TX-FRONTIER` to the implementation and decoder-support matrices, update spec mapping if needed, and regenerate generated status docs.
- [x] 3.2 Run focused tests, the local ac0ej3 decode probe, `openspec validate --all --no-interactive`, `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask ci`.
