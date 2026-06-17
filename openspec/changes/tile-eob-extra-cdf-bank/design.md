## Context

The block CDF subset in `splot-decode` already carries the partition-entry banks
plus the minimal flat-intra block-mode banks (`txb_skip`, `v_txb_skip`, …),
selected by `BlockCdfSelector` / `TileCdfSelector` and copied from `splot-core`'s
generated §9.3 defaults. The coefficient-decode loop (§5.20.7.27) that will
produce `Quant` needs the coefficient CDF banks wired in. They are added one at a
time as additive, verifiable plumbing.

## Decisions

- **`eob_extra` first.** It is the unique coefficient CDF whose §8.3.2 selection
  is context-free: the spec text is literally "the cdf is given by
  `TileEobExtraCdf`" (08-parsing-process.md s-8-3-2 line 1293) — no `Level[]`
  sum, scan position, eob index, neighbour scan, or transform geometry. Its only
  selection dimension is the frame `coeff_cdf_q_ctx`, already carried by the
  `txb_skip` / `v_txb_skip` banks, so the selector is a one-field clone of the
  smallest existing bank, and its default table (`Default_Eob_Extra_Cdf`,
  `[4][3]`) is the smallest coefficient family and already generated in
  `splot-core`. Every other coefficient CDF is strictly larger or blocked
  (`eob_pt` carries `eobCtx`; `coeff_base` / `coeff_br` need `Level[]`; `dc_sign`
  scans `AboveDcContext` / `LeftDcContext` frame buffers).

- **One-index bank.** `EobExtraCdfRows = [[i32; CDF_ROW_LEN];
  COEFF_CDF_Q_CONTEXTS]` (`[4][3]`) mirrors `DEFAULT_EOB_EXTRA_CDF`
  (`[[i32; 3]; 4]`) exactly (`CDF_ROW_LEN == 3`), so `from_defaults` assigns the
  generated table directly. The selector resolves to `eob_extra[coeff_cdf_q_ctx]`
  after the existing `checked_coeff_cdf_q_context` bound check — no inner `ctx`
  index, unlike `v_txb_skip`.

- **Additive / no-output-change.** The bank is loaded into the subset and joins
  the copy / average / count-scale lifecycle, but the §5.20.7.27 `coeffs()`
  decode loop that reads it is not wired, so no decode path consumes it and the
  minimal-fixture hash / Y4M / raw output is byte-identical (the existing CLI
  decode tests prove this). The matrix row stays partial and the notes state the
  bank is loaded-but-unread.

## Risks / Trade-offs

- **Shape transposition** — `DEFAULT_EOB_EXTRA_CDF` is `[q_ctx][3]`, so the alias
  must be `[[i32; CDF_ROW_LEN]; COEFF_CDF_Q_CONTEXTS]`; covered by the
  default-load assertion test.
- **Missing error array variant** — `TileCdfArray::EobExtra` must exist so the
  `SelectorOutOfRange` error can name the array; covered by the bounds-error test.
- **Honesty** — the matrix/support-matrix notes explicitly mark the bank
  loaded-but-unread (not "used in decode") so the partial row stays truthful.
