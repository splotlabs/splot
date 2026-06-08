# Proposal: Metadata semantic / lifetime validation

## Summary

Track the AV2 v1.0.0 § 6.16 metadata *semantic* validation that the
`metadata-padding-foundation` change deliberately left deferred. That change landed the
full § 5.17 metadata parsers and every **locally-decidable** § 6.16 diagnostic, but the
remaining metadata conformance rules are **stateful** (cross-OBU / CVS-scoped) or depend
on capabilities `splot` does not have yet (a decoder, full frame/tile parsing). This
change is the home for that remaining work so `AV2-5.17-METADATA` can eventually move
from `validate = "partial"` to `validate = "done"`.

This is a tracking proposal: it is **not implemented yet**, and several of its items are
blocked on other roadmap phases (see Blockers).

## Why

`metadata-padding-foundation` (now archived) parses `metadata_short_obu()`,
`metadata_group_obu()`, the bounded `metadata_unit()`, and the § 5.17.4-§ 5.17.13 child
payloads, and emits the local § 6.15 / § 6.16 diagnostics (layer-idc range, group
header/unit-count bounds, reserved/xlayer/mlayer-map rules, timecode and scan-type
ranges, temporal-point-info placement). The metadata umbrella is therefore honestly
`validate = "partial"`: the per-OBU rules are checked, but the rules that span OBUs or a
coded video sequence are not. This change collects exactly those.

## What changes (future)

1. **Persistence / cancellation lifetime store (§ 6.16.3).** A CVS-scoped, per-layer
   state machine that applies `muh_persistence_idc` (GLOBAL / BASIC / NO / ENHANCED) and
   `muh_cancel_flag`, propagating active metadata across layers via
   `TLayerDependencyMap` / `MLayerDependencyMap`.
2. **Scan-type CVS-wide consistency (§ 6.16.10).** Correlate `metadata_scan_type`
   (`mps_pic_struct_type`, `mps_source_scan_type_idc`) with `ci_scan_type_idc` from the
   content-interpretation OBU, and enforce the "for all pictures in the current CVS,
   only one `mps_pic_struct_type` group is used" constraint. (The local
   `mps_pic_struct_type <= 12` check already shipped.)
3. **Decoded-frame-hash verification (§ 6.16.13).** Verify `frame_hash` / `plane_hash`
   (MD5) against the decoded output samples. (The hash is already parsed and surfaced.)
4. **Frame-unit suffix/prefix placement (§ 7.3.3 / § 7.3.4).** Validate the exact
   position of prefix vs. suffix metadata *inside* a coded frame unit. (The coarse
   temporal-unit prefix/suffix/coded-layer classification already shipped.)

## Blockers (why each is not done yet)

- **Item 1** needs a CVS-scoped metadata state machine plus the sequence-header
  layer-dependency maps, which the sequence-header model does not yet expose.
- **Item 2** needs CVS-scoped state and cross-OBU correlation over a precisely modeled
  CVS boundary (the validator currently approximates the boundary at temporal-unit
  resets, a sound-over-complete choice).
- **Item 3** needs a **decoder**: `splot` has no entropy coder or reconstruction
  (`RangeDecoder` is a stub), so there are no decoded samples to hash. Blocked on the
  Phase 9/10 decoder/conformance work in `docs/VALIDATOR-ROADMAP.md`.
- **Item 4** needs **full frame-header + tile-group parsing** to locate the frame-data
  boundary within a coded frame unit. Blocked on Phase 8/9 (`AV2-5.18-FRAME-HEADER`,
  `AV2-5.19-TILE-GROUP`).

## Non-goals

- Re-implementing the parsing or local diagnostics already delivered by
  `metadata-padding-foundation`.

## Feature IDs

- `AV2-5.17-METADATA` (umbrella; `validate` advances toward `done` as items land)
- `AV2-5.17.10-METADATA-SCAN-TYPE`, `AV2-5.17.12-METADATA-DECODED-FRAME-HASH`
- `AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT`, `AV2-7.3.4-CODED-NONOUTPUT-FRAME-UNIT`

## Acceptance criteria

- Each item ships with stable diagnostics, spec citations, and positive/negative tests,
  and the corresponding matrix row's `validate` stage advances only with proof.
- `AV2-5.17-METADATA` reaches `validate = "done"` only when items 1-4 (or their
  documented blocked dependencies) are resolved.
