## 1. Residual Policy and Metadata

- [x] 1.1 Inspect the current transform-tool residual and Wiener NS LR record
  paths, then add an explicit LR tx-skip active-IST handoff policy.
- [x] 1.2 Return luma active-IST syntax metadata from the DCT-only residual
  handoff without changing coefficient reconstruction semantics.
- [x] 1.3 Keep general reconstruction/output residual policies fail-closed for
  active intra IST after consuming required syntax.

## 2. Tests and Live Probe

- [x] 2.1 Add focused positive tests proving active `sec_tx_type` is admitted
  only under the LR tx-skip handoff policy and metadata is surfaced.
- [x] 2.2 Add negative tests proving the existing reconstruction-safe policy
  still rejects active intra IST.
- [x] 2.3 Run the local decoder mission decode probe and update the ignored CLI runtime
  gate to the next real unsupported frontier if the stream advances.

## 3. Tracking and Verification

- [x] 3.1 Add `DECODE-ACTIVE-INTRA-IST-HANDOFF` to the implementation
  matrix, decoder-support matrix, spec mapping if needed, and generated status
  docs.
- [x] 3.2 Run focused tests, `openspec validate --all --no-interactive`,
  `cargo xtask feature-status`, `cargo xtask check-feature-status`,
  `cargo xtask check-decoder-support`, `cargo xtask conformance`, and
  `cargo xtask ci`.
- [x] 3.3 Sync and archive the OpenSpec change once all tasks are complete.
