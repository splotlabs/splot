# Proposal: Metadata semantic / lifetime validation

## Summary

Track the AV2 v1.0.0 § 6.16 metadata *semantic* validation that the
`metadata-padding-foundation` change deliberately left deferred. That change landed the
full § 5.17 metadata parsers and every **locally-decidable** § 6.16 diagnostic, but the
remaining metadata conformance rules are **stateful** (cross-OBU / CVS-scoped) or depend
on capabilities `splot` does not have yet (a decoder, full frame/tile parsing). This
change is the home for that remaining work so `AV2-5.17-METADATA` can eventually move
from `validate = "partial"` to `validate = "done"`.

Items 1 and 2 (and their prerequisites — the sequence-header dependency maps and the
exact coded-video-sequence boundary) are **implemented**. Items 3 and 4 remain
blocked-tracking entries gated on other roadmap phases (see Blockers).

## Why

`metadata-padding-foundation` (now archived) parses `metadata_short_obu()`,
`metadata_group_obu()`, the bounded `metadata_unit()`, and the § 5.17.4-§ 5.17.13 child
payloads, and emits the local § 6.15 / § 6.16 diagnostics (layer-idc range, group
header/unit-count bounds, reserved/xlayer/mlayer-map rules, timecode and scan-type
ranges, temporal-point-info placement). The metadata umbrella is therefore honestly
`validate = "partial"`: the per-OBU rules are checked, but the rules that span OBUs or a
coded video sequence are not. This change collects exactly those.

## What changes

1. **Persistence / cancellation lifetime store (§ 6.16.3) — implemented.** A
   CVS-scoped, per-layer state machine that applies `muh_persistence_idc`
   (GLOBAL / BASIC / NO / ENHANCED) and `muh_cancel_flag`, propagating active metadata
   across layers via `TLayerDependencyMap` / `MLayerDependencyMap` (now exposed by the
   sequence-header model), plus the reserved-value warnings and the § 6.16.5/§ 6.16.6
   HDR CLL/MDCV repeat-content checks.
2. **Scan-type CVS-wide consistency (§ 6.16.10) — implemented.** Correlates
   `metadata_scan_type` (`mps_pic_struct_type`) with `ci_scan_type_idc` from the
   content-interpretation OBU per Table 6.18, and enforces the "for all pictures in the
   current CVS, only one `mps_pic_struct_type` group is used" constraint over the exact
   § 7.3.6 CVS boundary. (The local `mps_pic_struct_type <= 12` check already shipped;
   the mirror defines no `mps_source_scan_type_idc` ↔ `ci_scan_type_idc` consistency
   rule, so none is checked.)
3. **Decoded-frame-hash verification (§ 6.16.13) — future.** Verify
   `frame_hash` / `plane_hash` (MD5) against the decoded output samples. (The hash is
   already parsed and surfaced.)
4. **Frame-unit suffix/prefix placement (§ 7.3.3 / § 7.3.4) — future.** Validate the
   exact position of prefix vs. suffix metadata *inside* a coded frame unit. (The coarse
   temporal-unit prefix/suffix/coded-layer classification already shipped.)

## Blockers (status per item)

- **Item 1** — resolved. The sequence-header model now exposes the § 5.4.1 dependency
  maps, the exact § 7.3.6 CVS boundary replaced the temporal-unit-reset approximation,
  and the lifetime store landed.
- **Item 2** — resolved. CVS-scoped state and the content-interpretation ↔ scan-type
  cross-reference landed over the exact CVS boundary.
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
- `AV2-5.17.3-METADATA-GROUP`, `AV2-5.17.5-METADATA-HDR-CLL`,
  `AV2-5.17.6-METADATA-HDR-MDCV` (item 1)
- `AV2-5.17.10-METADATA-SCAN-TYPE` (item 2)
- `AV2-5.4.1-SEQUENCE-HEADER-GENERAL`, `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`
  (prerequisites: dependency maps, exact CVS boundary)
- `AV2-5.17.12-METADATA-DECODED-FRAME-HASH` (item 3, blocked)
- `AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT`, `AV2-7.3.4-CODED-NONOUTPUT-FRAME-UNIT`
  (item 4, blocked)

## Acceptance criteria

- Each item ships with stable diagnostics, spec citations, and positive/negative tests,
  and the corresponding matrix row's `validate` stage advances only with proof.
- `AV2-5.17-METADATA` reaches `validate = "done"` only when items 1-4 (or their
  documented blocked dependencies) are resolved.
