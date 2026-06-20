## Context

Brick 4's `encode_minimal_intra_clk_tile_group_obu` returns the § 5.19 `tile_group_obu()`
payload bytes — the OBU body, with no length framing. `write_annexb_obu(writer, header,
payload: &[u8])` (§ B.2) takes a § 5.2.2 `ObuHeader` and raw payload bytes and writes
`leb128(header_len + payload_len)` + header + payload. This brick composes them.

## Goals / Non-Goals

- Goal: emit a complete, self-delimiting OBU unit from coded tile bytes.
- Non-Goals: a temporal delimiter, a sequence-header OBU, a multi-OBU temporal unit, an IVF
  stream, a complete spec-conformant coded tile, a packet, or `receive_packet` output.

## Decision: a fixed CLK header literal, verified by the round-trip

The § 5.2.2 header is fixed for the frozen tier: no extension, `OBU_CLOSED_LOOP_KEY`,
`obu_tlayer_id == 0`. The no-extension layer-id inference is `obu_mlayer_id == 0` and
`obu_xlayer_id == 0` (CLK is not `OBU_MSDO` / `OBU_TEMPORAL_DELIMITER`, the only types that
infer the global xlayer — `read_obu_header_from_slice`, § 5.2.2). The `ObuHeader` is built as
a literal (the inference is a documented constant, not a parameter), and the round-trip test
verifies it: a wrong field would reparse to a different header or fail.

## Oracle

`parse_annex_b_obus_partial` of the result must be exactly one OBU with no error, header
type `OBU_CLOSED_LOOP_KEY`, no extension, and a payload equal to the brick-4 payload (which
in turn ends in the coded tile bytes). The reject test pins the empty-`tile_data` path.

## Error model

Reuses `MinimalIntraTileGroupError`: the inner assembler already returns it, and
`write_annexb_obu`'s `WriteError` maps to its `Write` arm via `#[from]` — both serialization
steps are `WriteError`, so no new error type is warranted.
