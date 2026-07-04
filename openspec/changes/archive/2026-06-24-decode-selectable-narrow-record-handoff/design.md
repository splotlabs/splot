## Context

The previous local decoder mission luma transform-type handoff exposes the next
`TX_MODE_SELECT` selectable-record frontier:
`unsupported_wienerns_lr_selectable_transform_records_empty_transform` at byte
offset 110. Temporary local tracing showed the immediate source as a luma-only
`BLOCK_8X32` leaf at MI row 16, col 44 whose `Max_Tx_Size_Rect` maps to
`TX_8X32`; the partition syntax then selects `TX_PARTITION_VERT5`, whose
quarter-width subrecords collapse to zero 4x4 columns.

The selectable-record path already allows luma-only narrow leaves as a bounded
local decoder mission subcase, and `SelectableLumaTxGrid` can represent the actual 8x32
extent. The current max-rectangle partition geometry is therefore the wrong
representation for this supported handoff subset when the consumed partition
would create empty transform cells outside the actual luma leaf width. After
the 8x32 leaf is retained, the stream also reaches a luma-only chroma-offset
`BLOCK_4X32` leaf at MI row 16, col 46; that leaf has `has_chroma == false`, so
it can use the same luma-only actual-extent record path without deriving chroma
residual coordinates.

## Goals / Non-Goals

**Goals:**

- Admit supported luma-only narrow selectable leaves by consuming partition
  syntax and recording their actual 4x4-grid extents when the consumed
  partition would create empty geometry.
- Admit luma-only chroma-offset narrow leaves and keep chroma-bearing offset
  leaves fail-closed.
- Preserve zero-width or zero-height transform subrecords as invalid for all
  other paths.
- Carry skipped luma residuals into the LR tx-skip transform-record list as
  `skip_flag = true, eob = 0`.
- Advance the local decoder mission probe past the empty-transform frontier to the next
  structured active-MRL diagnostic.

**Non-Goals:**

- Active MRL prediction or `UsesMrls` propagation.
- Broad selectable transform partition support outside the observed luma-only
  narrow subset.
- Decoded samples, `FilterClass`, `SubclassLookup`, loop-restoration filtering
  or output, reference refresh, AVM/dav2d byte equality, or successful local decoder mission
  decode.

## Decisions

1. Consume partition syntax before actual-extent fallback.

   For observed luma-only leaves already admitted by
   `selectable_transform_leaf_shape_supported`, the handoff still consumes
   AV2 §5.20.6.3 partition symbols. If applying the consumed partition would
   create zero-width or zero-height subrecords, it writes one
   `SelectableLumaTxRecord` using the leaf's `Num_4x4_Blocks_*` dimensions
   instead. This keeps symbol order aligned while preventing impossible
   zero-width `VERT5` subrecords for the 8x32 leaf. Alternative considered:
   no-op zero-width subrecords. That only advanced the diagnostic to an
   incomplete-grid error and still left the leaf unrepresented.

2. Allow chroma-offset only for luma-only narrow leaves.

   The observed follow-on chroma-offset leaf is still a luma partition leaf and
   carries `has_chroma == false`. The handoff admits that subset because no
   chroma residual coordinates need to be derived. Chroma-bearing offset leaves
   remain rejected with the existing structured diagnostic.

3. Keep zero-geometry rejection for general selectable records.

   `SelectableLumaTxGrid::set_tx_size` continues to reject zero `h4` or `w4`
   outside the explicit luma-only narrow fallback. This preserves fail-closed
   behavior for invalid partition combinations and prevents fabricated cells.

4. Store the decoded all-zero state.

   `decode_general_intra_plane_coeffs` already consumes `all_zero` and updates
   coefficient contexts for skipped luma transforms. The LR tx-skip handoff will
   store that result as `skip_flag = luma.all_zero` with `eob = 0` for skipped
   records, while preserving nonzero `eob` and IST metadata for admitted
   nonzero records.

## Risks / Trade-offs

- Narrow-leaf fallback could be too broad if applied to chroma-bearing, shared,
  transposed, or unrelated narrow leaves. Mitigation: consume partition syntax
  first, gate fallback on `frontier.is_luma_part()`, `!frontier.has_chroma`,
  and the observed vertical 4x32/8x32 shape set; keep tests for rejected
  chroma-bearing, transposed, and unrelated narrow shapes.
- Chroma-offset admission could accidentally imply chroma residual coordinate
  support. Mitigation: admit only `frontier.is_luma_part() && !frontier.has_chroma`
  and keep chroma-bearing offset leaves on the existing fail-closed diagnostic.
- The next frontier is active MRL, not a successful decode. Mitigation: update
  diagnostics and docs to name the active-MRL stop and leave MRL prediction out
  of scope.
