## Context

The `splot-decode` block CDF subset already carries the partition-entry banks, the
minimal block-mode banks, and (from the prior brick) the `eob_extra` coefficient
bank. The coefficient banks are added one family at a time as additive, verifiable
plumbing toward the §5.20.7.27 `coeffs()` decode loop.

## Decisions

- **`eob_pt` next; closed-form context.** §8.3.2 selects
  `TileEobPt<size>Cdf[eobCtx]` with `eobCtx = (plane > 0) ? 2 : is_inter`
  (05-syntax line 15362) — a closed-form value needing no `Level[]`, scan order,
  or eob position. The only other input is the transform-size class
  (`eobMultisize`), modelled as the `EobPtSize` enum. So, like `eob_extra`, this
  is a self-contained bank-load + selector brick (the size and `eobCtx`
  derivations from real block state are read-time, deferred with the `coeffs()`
  loop).

- **Seven distinct-width banks.** The §9.3 defaults are
  `Default_Eob_Pt_<size>_Cdf[COEFF_CDF_Q_CTXS][EOB_PLANE_CTXS][N]` with class
  widths N = 6 (16), 7 (32), 8 (64), and 9 (128/256/512/1024). Each maps to its
  own type alias and `BlockCdfRows` field; the `row` / `row_mut` arms dispatch on
  `EobPtSize` and return the width-agnostic `&[i32]` slice after bounds-checking
  `coeff_cdf_q_ctx` (`< COEFF_CDF_Q_CONTEXTS`) and `eob_ctx`
  (`< EOB_PLANE_CTXS`). Generic `avg_eob_pt_bank<N>` / `scale_eob_pt_bank<N>`
  helpers fold the seven banks into the existing lifecycle using the
  width-generic `avg_cdf_row` / `scale_cdf_count`.

- **Consistent store-all design.** The banks store all `COEFF_CDF_Q_CONTEXTS`
  rows and resolve the q-context at read time, matching the merged
  `txb_skip` / `v_txb_skip` / `eob_extra` banks. The AV2 §6.19.1
  `init_coeff_cdfs()` single-row representation is a cross-cutting follow-up
  (collapsing all coefficient banks uniformly + threading `base_q_idx` into
  `from_defaults()`), tracked to land with the `coeffs()` decode-loop wiring,
  which is when the wrong-q-context misuse becomes reachable.

- **Additive / no-output-change.** The family is loaded but not read by any
  decode path, so the minimal-fixture hash / Y4M / raw output is byte-identical
  (the decode CLI tests prove it). The matrix row stays partial.

## Risks / Trade-offs

- **Width transposition** across the seven banks — covered by the per-size
  default-load + selection test, which compares each bank against its generated
  `Default_Eob_Pt_<size>_Cdf` for every `coeff_cdf_q_ctx` and `eobCtx`.
- **Missing error array variant** — `TileCdfArray::EobPt` must exist so the
  `SelectorOutOfRange` error can name the family; covered by the bounds test.
