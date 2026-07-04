# Decode deblock geometry and LR reference filter resolution

## Why

Coded frame 2 of the mission stream reconstructed a byte-exact pre-filter
luma plane but diverged after the § 7.2 filter chain on 316,616 luma
samples: intra transform units inside inter frames recorded one
block-level deblock entry (hiding every interior transform edge from the
§ 7.17.2 walk), skipped inter blocks' Max_Tx_Size_Rect tile seams passed
as coding-block edges, and the § 5.18 loop-restoration filter-match
dictionary resolved match indices without the reference-frame filter
group — every frame-bank tap that should copy `RefFrameLrWienerNs`
resolved to a wrong source, and the PC-Wiener translation offset ignored
the reference group entirely.

## What Changes

- Deblock records are per executed luma transform unit and carry the
  § 7.17 coding-block origin (`MiRowBase`/`MiColBase`) separately from
  the transform origin; `isBlockEdge` compares against the coding block,
  and a per-plane chroma base keeps the block's single chroma transform
  geometry under per-unit luma records.
- The reference buffer retains each stored frame's parsed frame-level
  Wiener-NS bank taps; the header parse builds the § 5.18
  `search_frame_filters` ordered entry list per plane from the retained
  taps (the same reference walk as the existing match-index count), and
  `fill_first_slot_of_bank_with_filter_match` resolves reference hits
  from it, offsets the PC-Wiener translation by both group sizes, and
  resolves chroma reference hits (05:17763-17817).

## Impact

- Affected specs: decoder-support (DECODE-FIRST-INTER-FRAME-FRONTIER)
- Affected code: `splot-decode` deblock records/walk, residual pipeline,
  reference buffer; `splot-core` lr_params filter-match resolution and
  the reference-state view
- Result: the mission stream's first three coded frames are POST-FILTER
  byte-exact on luma; remaining divergence is the disclosed pre-filter
  chroma batch
