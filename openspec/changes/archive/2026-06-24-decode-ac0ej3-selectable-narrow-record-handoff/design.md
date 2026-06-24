## Context

The previous ac0ej3 luma transform-type handoff exposes the next
`TX_MODE_SELECT` selectable-record frontier:
`unsupported_wienerns_lr_selectable_transform_records_empty_transform` at byte
offset 110. Temporary local tracing showed the immediate source as a luma-only
`BLOCK_8X32` leaf at MI row 16, col 44 whose `Max_Tx_Size_Rect` maps to
`TX_8X32`; the partition syntax then selects `TX_PARTITION_VERT5`, whose
quarter-width subrecords collapse to zero 4x4 columns.

The selectable-record path already allows luma-only narrow leaves as a bounded
ac0ej3 subcase, and `SelectableLumaTxGrid` can represent the actual 8x32
extent. The current max-rectangle partition path is therefore the wrong
representation for this supported handoff subset: it attempts transform cells
outside the actual luma leaf width and fails before residual syntax can prove
the next frontier. After the 8x32 leaf is retained, the stream also reaches a
luma-only chroma-offset `BLOCK_4X32` leaf at MI row 16, col 46; that leaf has
`has_chroma == false`, so it can use the same luma-only actual-extent record
path without deriving chroma residual coordinates.

## Goals / Non-Goals

**Goals:**

- Admit supported luma-only narrow selectable leaves by recording their actual
  4x4-grid extents directly.
- Admit luma-only chroma-offset narrow leaves and keep chroma-bearing offset
  leaves fail-closed.
- Preserve zero-width or zero-height transform subrecords as invalid for all
  other paths.
- Carry skipped luma residuals into the LR tx-skip transform-record list as
  `skip_flag = true, eob = 0`.
- Advance the local ac0ej3 probe past the empty-transform frontier to the next
  structured active-MRL diagnostic.

**Non-Goals:**

- Active MRL prediction or `UsesMrls` propagation.
- Broad selectable transform partition support outside the observed luma-only
  narrow subset.
- Decoded samples, `FilterClass`, `SubclassLookup`, loop-restoration filtering
  or output, reference refresh, AVM/dav2d byte equality, or successful ac0ej3
  decode.

## Decisions

1. Use actual narrow leaf extents before max-rectangle partitioning.

   For luma-only leaves already admitted by
   `selectable_transform_leaf_shape_supported`, the handoff writes one
   `SelectableLumaTxRecord` using the leaf's `Num_4x4_Blocks_*` dimensions and
   returns without reading §5.20.6.3 partition symbols. This keeps symbol order
   aligned with the observed stream and prevents impossible zero-width `VERT5`
   subrecords for the 8x32 leaf. Alternative considered: no-op zero-width
   subrecords. That only advanced the diagnostic to an incomplete-grid error and
   still left the leaf unrepresented.

2. Allow chroma-offset only for luma-only narrow leaves.

   The observed follow-on chroma-offset leaf is still a luma partition leaf and
   carries `has_chroma == false`. The handoff admits that subset because no
   chroma residual coordinates need to be derived. Chroma-bearing offset leaves
   remain rejected with the existing structured diagnostic.

3. Keep zero-geometry rejection for general selectable records.

   `SelectableLumaTxGrid::set_tx_size` continues to reject zero `h4` or `w4`
   outside the explicit luma-only narrow bypass. This preserves fail-closed
   behavior for invalid partition combinations and prevents fabricated cells.

4. Store the decoded all-zero state.

   `decode_general_intra_plane_coeffs` already consumes `all_zero` and updates
   coefficient contexts for skipped luma transforms. The LR tx-skip handoff will
   store that result as `skip_flag = luma.all_zero` with `eob = 0` for skipped
   records, while preserving nonzero `eob` and IST metadata for admitted
   nonzero records.

## Risks / Trade-offs

- Narrow-leaf bypass could be too broad if applied to chroma-bearing, shared, or
  unrelated narrow leaves. Mitigation: gate it on `frontier.is_luma_part()`,
  `!frontier.has_chroma`, and the observed 4x32/8x32 shape set; keep tests for
  rejected chroma-bearing and unrelated narrow shapes.
- Chroma-offset admission could accidentally imply chroma residual coordinate
  support. Mitigation: admit only `frontier.is_luma_part() && !frontier.has_chroma`
  and keep chroma-bearing offset leaves on the existing fail-closed diagnostic.
- The next frontier is active MRL, not a successful decode. Mitigation: update
  diagnostics and docs to name the active-MRL stop and leave MRL prediction out
  of scope.
