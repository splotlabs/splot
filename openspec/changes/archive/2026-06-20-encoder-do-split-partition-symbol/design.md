## Context

The encoder targets the AVM-validated general intra decode path (the prior `-minimal` frozen
tier uses an inverted `txb_skip` polarity and is abandoned as the encoder oracle). On that
path the partition tree reads `do_split` first (partition.rs:234). For the frozen single-block
64x64 tier `do_split == false` (`PARTITION_NONE`).

## Decision: pin the ctx empirically

The § 8.3.2 `do_split` context is `adj_size * 4 + ctx1 * 2 + ctx2` where
`adj_size = Partition_Size_Adjust[BLOCK_64X64] = 3` and the two out-of-frame neighbour bits
are 0 at the tile origin, giving `ctx == 12`. Rather than trust an analytic derivation, the
ctx was pinned **empirically**: a temporary debug print in the decoder's
`do_split_selector` call site, run over the AVM-validated `syn-flat-intra-64x64-q80` fixture,
printed `DoSplit { plane_start: 0, ctx: 12 }`. The CDF row is therefore
`DEFAULT_DO_SPLIT_CDF[0][12]`.

## Oracle

The token round-trips through one § 8.2 coder (`roundtrip_block_symbol_trace`) to symbol 0
with `symbol_count == 1` — the same machinery the existing mode/coefficient tokens use. The
ctx's cross-crate correctness (that splot-decode reads this exact byte as the root
`do_split=false`) is proven end-to-end when the full skip trace is muxed into an IVF and
decoded (a later brick).

## Non-Goals

A full block trace, a tile, a frame, a packet, `Context::receive_packet` output, or any
non-`PARTITION_NONE` partition.
