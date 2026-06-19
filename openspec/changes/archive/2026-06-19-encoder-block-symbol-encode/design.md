## Context

The block-symbol trace work (modes + per-plane all-zero + the single-DC magnitude
vocabulary + eob=2 multi-coefficient + transform-type signaling) all routes through
one §8.2 coder, but only inside the `roundtrip_block_symbol_trace` TEST helper, which
both encodes and re-decodes to prove the bytes. To assemble a real packet, the encoder
needs a production `trace -> bytes` function whose output downstream tile-group
assembly can carry.

This brick extracts the encode half of the roundtrip helper into
`encode_block_symbol_trace(trace) -> Result<Vec<u8>>` and has the helper call it. The
function builds the scoped default CDF rows, drives the `SymbolEncoder` (one
`write_symbol` per CDF token, one `write_literal` per bypass token), and `finish()`es
the stream, returning the coded bytes. For a single-tile §5.20.1 `tile_group_payload()`
these bytes ARE the tile data (a single tile writes no `tile_size` field).

`splot-encode` cannot depend on `splot-decode` (the dependency graph), so the
cross-tool encode→decode hash check lives later at the CLI/integration level. Within
`splot-encode`, the function's output is proven decodable via `splot-core`'s
`SymbolDecoder` (the existing roundtrip).

## Goals / Non-Goals

**Goals:**

- A production §8.2 entropy-coding entry point returning coded bytes, with the test
  roundtrip refactored to use it.

**Non-Goals:**

- No tile-group payload framing, no tile-group OBU, no frame assembly, no
  `Context::receive_packet` wiring, no CLI, no cross-tool decode check (later bricks).

## Decisions

1. **Extract, don't duplicate.** The roundtrip helper calls the new function for its
   encode half, so there is one §8.2 encode path; the roundtrip's decode half is
   unchanged and still proves the bytes.

2. **Return raw coded bytes.** The function returns `Vec<u8>` (the tile's coded data),
   not a tile-group payload/OBU — those need framing/header inputs supplied by later
   bricks.

## Flight Manifest

- Change ID: `encoder-block-symbol-encode`
- Feature IDs: `ENC-BLOCK-SYMBOL-ENCODE`
- Base commit: `03799785`
- Depends on merged changes: the block-symbol trace series (through `encoder-block-trace-ist`).
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/block_symbol_trace_tests.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-symbol-encode/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-symbol-encode/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; other `crates/splot-encode/src/**` modules (coefficient
    tokenization, closed_loop, context, error, lib)
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-BLOCK-SYMBOL-ENCODE`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none at base.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR lands
  first, sync `main`, regenerate the tracking docs, and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The function looks like a pure extraction. -> Mitigation: it is the
  production entropy-coding boundary the tile-group assembly consumes; the test proves
  it emits the decodable §8.2 bytes, and the roundtrip now shares the one encode path.
