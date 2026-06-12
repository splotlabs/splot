# Proposal: parse deblocking, GDF, and CDEF frame-header params

## Feature IDs

- `AV2-5.18.5-FILTERING` (deblocking_filter_params, § 5.18.5.2)
- `AV2-5.18.7-SEGMENTATION-TILING` (gdf_params § 5.18.7.9, cdef_params
  § 5.18.7.10 — confirm the rows; if GDF/CDEF live on a different matrix
  row, target that row instead)
- `AV2-5.18.2-FRAME-HEADER-INFO` (the intra-path stop point advances)

## Why

The intra core parse ends at `StoppedBeforeDeblockingFilterParams`;
§ 5.18.2's tail continues with exactly `deblocking_filter_params()`
(mirror `05-syntax-structures.md`:5923), `gdf_params()` (:6887), and
`cdef_params()` (:6983) — see the call sites at :5297-5301. All three are
fully determined by the already-parsed § 5.4.10 sequence filter config plus
frame state; no new dependency blocks them. PR #54 threaded
`mfh_deblocking_filter_update` / `mfh_apply_deblocking_filter` precisely
for this change's `cur_mfh_id > 0` arms.

## What Changes

1. Parse `deblocking_filter_params()` per § 5.18.5.2, including the
   `cur_mfh_id > 0` arms consulting the resolved MFH's
   `mfh_deblocking_filter_update` / `mfh_apply_deblocking_filter` (mirror
   :5949) and the inference rules around them.
2. Parse `gdf_params()` per § 5.18.7.9 and `cdef_params()` per § 5.18.7.10,
   gated on the parsed sequence filter config exactly as the mirror
   prescribes.
3. Advance the intra-path stop point past the three structures; the new
   stop status names the next unparsed structure honestly.
4. `inspect` surfaces the newly parsed fields; validator facts get richer
   on the intra path (no new diagnostics unless a § 6 bound on these
   fields is locally decidable and unambiguous — if one is, add it with
   its citation; otherwise name it as residual).
5. § 5.18.5.1 `read_interpolation_filter()` is inter-only and stays with
   the inter-paths change (named residual).

## Non-goals

- Inter-path parsing (§ 5.18.5.1 and the inter tail).
- Loop-restoration / CCSO params (§ 5.18.7.11-.12 — next change).
- Reconstruction semantics of the filters.

## Acceptance criteria

- [ ] The three structures parse on the intra path (incl. MFH-backed
  frames); positive/negative/EOF tests per structure, each gating flag
  both ways; unresolvable-MFH frames keep Unknown routing.
- [ ] The stop point's new location is tested; matrix proof recorded;
  `cargo xtask ci` green.
