# Proposal: parse the § 5.18.2 inter/TIP/bridge/switch control region

## Feature IDs

- `AV2-5.18.2-FRAME-HEADER-INFO` (the non-intra control region)
- `AV2-5.18.3-FRAME-CONFIGURATION` (frame_opfl_refine_type § 5.18.3.2,
  get_relative_dist § 5.18.3.1 — confirmed row id)
- `AV2-5.18.4-FRAME-SIZE` (frame_size_with_refs § 5.18.4.3,
  frame_size_with_bridge § 5.18.4.2)
- `AV2-5.18.5-FILTERING` (read_interpolation_filter § 5.18.5.1)

## Why

`parse_frame_header_core` stops right after the frame-type field on any
non-intra frame — the largest single § 5 parsing gap. The PR #62
reference-state model supplies the per-slot facts the inter region
consumes. Unparsed elements (mirror `#s-5-18-2`): primary-ref signaling,
the inter refresh branches (incl. `bridge_frame_overwrite_flag`), the
explicit reference map (`num_total_refs`, `ref_frame_idx[i]`),
`use_bru`/`bru_ref`/`bru_inactive`, `use_ref_frame_mvs`/`tmvp_*`, the TIP
block, `change_drl`/`max_drl_bits_minus_1`, MV precision flags,
`frame_enabled_motion_modes`, `read_interpolation_filter()`,
`allow_df_sub_pu`/`apply_deblocking_filter_tip`, the with-refs/bridge
frame sizes, and the § 5.18.3 reference-distance derivations. Landing this
unblocks the inter arms of § 5.18.6/.7 tests and the BRU arms of § 5.19.

## What Changes

1. Parse the non-intra § 5.18.2 control region exactly per the mirror,
   structure by structure, gated on the parsed sequence config and the
   modeled reference state. Where a branch consumes reference-state facts
   the model has poisoned (mid-stream joins, prior unparsed frames), the
   parse stops honestly at that branch (facts preserved) — never guesses
   bit positions.
2. The inter tail joins the intra tail where the paths converge (the
   shared structure cluster — ground which structures the inter path
   shares and where it diverges; reuse the existing modules).
3. The § 5.19 BRU arms (PR #61 residual) become decidable where
   `use_bru`/`bru_inactive` parse; the deblocking MFH/`allow_df_sub_pu`
   and TIP-deblocking arms land in filtering.rs.
4. § 6 conformance clauses on the new fields that are locally decidable
   get diagnostics with citations (e.g. `ref_frame_idx` validity against
   the modeled RefValid, `num_total_refs` bounds, primary-ref constraints
   — ground each; under-report what needs unmodeled state).
5. EOF/truncation per the established partition; constructed-input audit;
   `inspect` surfaces the new region; fixtures extended per README
   recipes.

## Non-goals

- Reconstruction/MV semantics; the Wiener bank decode.
- § 5.20 payloads; global-motion subexp (item 22).
- Output-order/decoder-model semantics.

## Acceptance criteria

- [ ] Inter/TIP/bridge/switch headers parse through their control region
  on reference-state-grounded streams; honest stops on poisoned state;
  per-structure positive/negative/EOF tests, gates both ways;
  diagnostics with citations; matrix proof; `cargo xtask ci` green.
