# Proposal: parse the § 5.18.9 inter global-motion arm

## Feature IDs

- `AV2-5.18.9-GLOBAL-MOTION` (the inter arm: use_global_motion, the
  per-reference warp loop, global_param § 5.18.9.2, the subexp chain
  § 5.18.9.3-.6)

## Why

`global_motion_params()` (mirror `05-syntax-structures.md`:7776) is a
no-bit return on intra (landed with the intra tail); the inter arm reads
`use_global_motion`, the `our_ref ns(NumTotalRefs+1)` base selection, and
per-reference warp parameters through `read_global_param()` and the
`decode_signed/unsigned_subexp_with_ref` / `decode_subexp` /
`inverse_recenter` chain. PR #63 supplies the parsed reference list
(`num_total_refs`/`ref_frame_idx`) the loop consumes. This is the last
unparsed structure of the inter control region before the shared tail.

## What Changes

1. Parse the inter arm exactly per the mirror: `use_global_motion`,
   the base-params selection (incl. the SWITCH_FRAME `our_ref`
   inference and the `RefNumTotalRefs` arm — ground what reference-state
   facts it needs; stop honestly where unmodeled), the per-ref
   `GmType`/warp-parameter loop with the full subexp decode chain
   (§ 5.18.9.3-.6 transcribed exactly, constructed-input audited).
2. Honest stops where the loop consumes facts the model lacks
   (RefNumTotalRefs or similar cross-frame state — ground each).
3. § 6 conformance clauses on the new fields that are locally decidable
   get diagnostics with citations; otherwise named residuals.
4. EOF inside the modeled arm = facts-preserving truncation per the
   established partition; `inspect` surfaces the parsed warp state.

## Non-goals

- Warp-model reconstruction semantics; motion-vector projection.
- The shared-tail inter inputs (separate residual).

## Acceptance criteria

- [ ] The inter arm parses on grounded streams; per-element
  positive/negative/EOF tests, gates both ways; subexp chain unit-tested
  against hand-computed values; honest stops tested; matrix proof;
  `cargo xtask ci` green.
