## Context

The frozen single-block decoder tier and the AVM-validated general intra decode path differ
in their `txb_skip` polarity and context derivation. Brick 1 established that the encoder
targets the general path (which reads `do_split` first). Brick 2 composes the full ordered
skip-block symbol stream the general path reads for one undivided 64x64 4:2:0 superblock.

## Goals

- Compose the exact ordered symbol stream the general intra decode path reads for an all-zero
  (skip) DC block, so a later brick can finalize it into a `tile_data` payload and decode it.
- Pin the general-path `txb_skip` transform contexts (`TX_64X64` luma, `TX_32X32` chroma)
  against the decoder, not against memory.

## Decisions

### The ordered trace and its contexts

The trace is `[do_split=0, y_mode_set=0, y_mode_index=0, uv_mode=0, luma all_zero=1,
U all_zero=1, V all_zero=1]`. The contexts were taken from the decoder's own general-intra
`txb_skip` selector derivation (`general_intra_residual.rs`):

- luma `txb_skip`: `coeff_cdf_q_ctx`, `plane_type 0`, `tx_size 4` (`TX_64X64`), `ctx 0`
  (`txb_skip_ctx_luma` returns `0` when the transform fills the block).
- U `txb_skip`: `tx_size 3` (`TX_32X32`), `ctx 6` (`0 + 0 + 6` for the U plane).
- V `txb_skip`: the dedicated `TileVTxbSkipCdf`, `ctx 0` (`v_txb_skip_ctx(0, 0, false, false)`
  — the chroma block equals its transform and U is all-zero, so neither the `+3` nor `+6`
  term applies).

The `TX_64X64` / `TX_32X32` `txSzCtx` values (`4`, `3`) were confirmed empirically: a
temporary debug print on the decoder's general `txb_skip` selector, run over the
AVM-validated `syn-flat-intra-64x64-q80` fixture, read `tx_size: 4` for the 64x64 luma
transform and `tx_size: 3` for the 32x32 chroma U transform. Those contexts are
geometry-derived and independent of the all-zero/coded outcome, so a coded fixture pins the
same contexts a skip block uses.

### Coefficient CDF q-context 0 (`base_q_idx <= 90`)

A skip frame's residual is all-zero, so its decoded pixels (DC prediction only) are
independent of `base_q_idx`; any value is valid. Choosing `base_q_idx <= 90` puts the
`txb_skip` symbols in `coeff_cdf_q_ctx` bank `0` — the same bank the q80 fixture's
`base_q_idx == 80` selects — so the luma/U rows are exactly the rows that fixture's decode
exercises, and the V `txb_skip` reuses the existing neutral `[0][0]` row.

### A dedicated module

The general-path composers are a distinct family from the minimal-tier composers. The
composer lives in a new `general_intra_trace` module (keeping `block_symbol_trace.rs` under
the 1000-line budget); the CDF rows and routing stay in `block_symbol_trace.rs` because they
are part of the shared § 8.2 coder driver.

## Risks / Trade-offs

- The contexts are verified only through the entropy-layer roundtrip and the empirical decoder
  pin; cross-crate bit-exact decode of an emitted skip frame is a later brick. The honest
  scope (one in-memory trace, no tile/packet) is recorded in the matrix and the spec.
