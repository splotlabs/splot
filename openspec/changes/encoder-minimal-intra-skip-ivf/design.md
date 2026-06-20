## Context

Bricks 1-4 proved each layer of a decodable minimal intra skip frame in isolation (symbol
roundtrips, byte-exact container vs the AVM-validated q80 fixture). What remained was to
assemble them and prove the assembled stream decodes — a cross-crate fact, since
`splot-encode` cannot depend on `splot-decode`.

## Decision: a public emit function in splot-encode, the oracle in splot-cli

`emit_minimal_intra_skip_ivf()` lives in `splot-encode` (it composes the skip `tile_data` and
calls the `splot-core` container assembler — both are within `splot-encode`'s allowed
dependencies). It is the crate's first public "emit a decodable stream" surface.

The decode proof lives in `splot-cli`, the only crate that depends on both `splot-encode` and
`splot-decode`. The oracle runs the real `splot decode` binary on the emitted IVF (matching
the existing `decode_raw_cli.rs` end-to-end pattern) rather than calling a decode API
directly, so it exercises the same path a user would.

## The expected output

The single block is a DC skip: `do_split == false`, `DC_PRED` luma and chroma, all-zero
residual. With no decoded neighbours the § 7.13.2 DC predictor is `1 << (BitDepth - 1)`, i.e.
`128` for 8-bit, and the all-zero residual leaves it unchanged. So every one of the 6144
samples (Y `64*64` + U `32*32` + V `32*32`) is `128` — a flat frame. The oracle asserts
exactly that.

## base_q_idx 80

`emit_minimal_intra_skip_ivf` muxes at `base_q_idx == 80` (the q80 fixture's value, `<= 90` so
the decoder derives coefficient CDF q-context `0`, matching the `tile_data`'s coding). The
container is therefore byte-identical to the AVM-validated `syn-flat-intra-64x64-q80` fixture
apart from the `tile_data`.

## Honesty

The decode is verified against `splot-decode` (itself AVM/dav2d-validated on the coded q80/q180
fixtures), not by decoding this exact stream with AVM — so the matrix records `decode_check`,
not `avm_diff`. This is the first decodable stream, not a general encoder, a `receive_packet`
packet, or Baseline Encoder Profile v1.
