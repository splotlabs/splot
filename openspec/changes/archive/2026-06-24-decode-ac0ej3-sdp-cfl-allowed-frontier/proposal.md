## Why

The live ac0ej3 decode probe now reaches AV2 §5.20.5 intra mode-info inside the
Wiener NS LR selectable transform-record path, but the runtime can desynchronize
before the first luma transform block. Two state gaps are involved:

- it can read `is_cfl` in SDP `CHROMA_PART` leaves where §5.20.3.1
  `CflAllowedInSdp` disables CfL/MHCCP syntax; and
- for luma/shared leaves it skips the §5.20.5.3 prelude syntax (`use_intrabc`,
  CDEF strength, and delta-Q) that precedes luma mode and §5.20.6 transform
  partition syntax in the ac0ej3 stream.

Those gaps shift later `uv_mode` and partition reads, stopping at
`unsupported_wienerns_lr_live_transform_record_uv_mode` instead of the next real
runtime frontier.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-SDP-CFL-ALLOWED-FRONTIER` for the ac0ej3 SDP
  chroma mode-info prerequisite.
- Add Feature ID `DECODE-AC0EJ3-INTRA-PRELUDE-TX-FRONTIER` for the ac0ej3
  luma/shared mode-info prelude and transform-record synchronization
  prerequisite.
- Retain the AV2 §5.20.3.1 `CflAllowedInSdp` state during intra SDP partition
  traversal and expose it on chroma-part block frontiers.
- Use that state in §5.20.5.6 chroma mode decoding so `is_cfl` and MHCCP syntax
  are skipped for `CHROMA_PART` leaves when `CflAllowedInSdp == 0`.
- Gate the LR transform-record handoff with the same explicit minimal-tool
  admission contract used by the normal general-intra route, except for the
  ac0ej3 tools intentionally consumed by this path.
- Consume the observed zero `use_intrabc`, CDEF strength-index, and delta-Q
  prelude syntax in §5.20.5.3 order before luma mode and §5.20.6 transform
  partition parsing.
- Reject chroma-offset leaves before deriving chroma residual coordinates from
  the wrong luma leaf.
- Add focused tests for the `CflAllowedInSdp` derivation and a live ac0ej3 probe
  expectation that advances past `unsupported_wienerns_lr_live_transform_record_uv_mode`.
- Update implementation and decoder-support tracking without claiming decoded
  samples, loop-restoration filtering, output, reference refresh, or successful
  ac0ej3 decode.

## Capabilities

### New Capabilities

- `ac0ej3-sdp-cfl-allowed-frontier`: SDP `CflAllowedInSdp` retention and
  chroma mode-info synchronization for the ac0ej3 Wiener NS LR transform-record
  path.
- `ac0ej3-intra-prelude-tx-frontier`: luma/shared intra mode-info prelude,
  tool-admission, CDEF/delta-Q synchronization, and chroma-offset safety for
  the ac0ej3 Wiener NS LR transform-record path.

### Modified Capabilities

- `ac0ej3-selectable-transform-records`: records the new SDP `CflAllowedInSdp`
  prerequisite before selectable transform records can continue to decoded
  sample population.
- `decoder-support`: adds the decoder support row for the new partial ac0ej3
  frontier.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/partition_traversal.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_block.rs`,
  `crates/splot-decode/src/tile_payload/cdf.rs`,
  `crates/splot-decode/src/runtime_minimal/wienerns_lr/tx_records.rs`,
  `crates/splot-decode/src/runtime_minimal/wienerns_lr.rs`, and the ac0ej3 CLI
  regression.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status Markdown, and OpenSpec
  main specs.
- No new dependencies, public API commitments, encoder work, broad CfL
  prediction support, decoded frame sample population, or output claim.
