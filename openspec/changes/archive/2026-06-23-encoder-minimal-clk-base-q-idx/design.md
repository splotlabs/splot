## Context

The minimal CLK container assembler was frozen at `base_q_idx == 255` (an arbitrary nonzero
choice that keeps `CodedLossless == 0`). The decodable-tile arc needs `base_q_idx <= 90` so
the decoder derives coefficient CDF q-context `0` — the q-context the brick-2 skip trace's
`txb_skip` symbols are coded under.

## Decision: parameterize base_q_idx, keep the frozen public API

`base_q_idx` is threaded through private impls; the existing frozen no-arg public functions
(`build_minimal_intra_clk_core`, `encode_minimal_intra_clk_tile_group_obu`,
`encode_minimal_intra_clk_annexb_obu`, `encode_minimal_intra_clk_ivf`) keep their signatures
and delegate at `FROZEN_TIER_BASE_Q_IDX == 255`, so the frozen tier and its tests are
untouched. One public entry point is added — `encode_minimal_intra_clk_ivf_with_base_q_idx` —
the only surface a later decode brick needs.

`base_q_idx` is the **only** variable field: a measurement of the existing assembler against
the q80 fixture showed they differ in exactly two bytes, both inside the CLK frame header's
`base_q_idx` field. So at `base_q_idx == 80` with the fixture's own `tile_data` the assembler
reproduces the fixture byte-for-byte; that is the headline test.

## Decision: reject base_q_idx == 0

With the canonical body's zero quantizer deltas and disabled segmentation, `base_q_idx == 0`
makes `CodedLossless == 1`, which changes the § 5.18.2 body bit layout (lossless frames read
different downstream fields). The fixed canonical writer does not model that, so `base_q_idx
== 0` is rejected up front with a typed `MinimalIntraCoreError::LosslessBaseQIdx` rather than
silently emitting a mis-parsing body.

## The decodability contract this satisfies

A prior multi-agent verification established that for the q_ctx-0 skip `tile_data` to decode,
the frame header must set `base_q_idx <= 90` (q-context 0) and `disable_cdf_update == 0`
(`CdfUpdateMode::Enabled`). The canonical body already writes `disable_cdf_update == 0`; this
brick supplies the `base_q_idx`. The cross-crate decode that proves an emitted skip frame
decodes lives in splot-cli (a later brick).

## Scope discipline

This brick parameterizes the container only. It does not mux the skip `tile_data`, decode
anything, or claim a coded skip frame — those are later bricks with distinct blast radii.
