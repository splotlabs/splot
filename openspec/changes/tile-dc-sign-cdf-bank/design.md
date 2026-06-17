## Context

The `splot-decode` block CDF subset now carries the `eob_extra` bank and the
`eob_pt` family. `dc_sign` is the next coefficient bank toward the §5.20.7.27
`coeffs()` decode loop.

## Decisions

- **Bank-load is self-contained; `ctx` derivation is deferred.** §8.3.2 reads
  `TileDcSignCdf[ptype][isHidden][ctx]` (plus the leading `coeff_cdf_q_ctx`),
  where `ctx` (0/1/2) is computed by scanning `AboveDcContext[plane]` /
  `LeftDcContext[plane]` (08-parsing-process.md lines 1448-1487). Those frame
  buffers do not exist yet, so the bank is loaded and the selector takes `ctx` as
  a caller-resolved index; the actual `ctx` derivation lands with the coeffs()
  loop. Like the other coefficient banks, the bank is loaded-but-unread, so this
  change is additive and no-output-change.

- **Four-axis selector, structurally like `txb_skip`.** `DcSignCdfRows` is
  `[COEFF_CDF_Q_CONTEXTS][PLANE_TYPES][DC_SIGN_GROUPS][DC_SIGN_CONTEXTS][CDF_ROW_LEN]`
  (`[4][2][2][3][3]`), matching `DEFAULT_DC_SIGN_CDF` exactly. The
  `BlockCdfSelector::DcSign` arm bounds-checks all four indices
  (`checked_coeff_cdf_q_context`, `checked_plane_type`, the new
  `checked_dc_sign_group`, and `.get(ctx)` against `DC_SIGN_CONTEXTS`) before
  indexing, so it is total. The `group` field is the spec `isHidden` flag.

- **Parameterize `checked_plane_type`.** It previously hard-coded
  `TileCdfArray::TxbSkip` in its error; it now takes the owning array so a
  `dc_sign` `plane_type` overflow reports `DcSign` (and `txb_skip` still reports
  `TxbSkip`). Keeps the typed error accurate per bank.

- **Flattened-iterator lifecycle.** `dc_sign` joins `avg_from_tile` /
  `scale_counts_for_frame_end_update` via `iter_mut().flatten().flatten()
  .flatten()` over the four index dims (avoiding `needless_range_loop`), using the
  width-generic `avg_cdf_row` / `scale_cdf_count`.

## Risks / Trade-offs

- **Shape fidelity** — covered by the full-index default-load test comparing every
  `[q][plane][group][ctx]` cell against `DEFAULT_DC_SIGN_CDF`.
- **Error-array accuracy** — the parameterized `checked_plane_type` is verified by
  the four-axis bounds test, which asserts each axis names `DcSign`.
