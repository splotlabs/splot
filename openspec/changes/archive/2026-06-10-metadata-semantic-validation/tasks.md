# Tasks: Metadata semantic / lifetime validation

This is deferred work split out of `metadata-padding-foundation` so that change could be
completed and archived. Sections 1 and 2 are implemented; sections 3 and 4 remain
blocked on other roadmap phases.

## 1. Persistence / cancellation lifetime (§ 6.16.3)

- [x] Expose `TLayerDependencyMap` / `MLayerDependencyMap` from the sequence-header model.
- [x] Model the exact coded-video-sequence boundary (replace the temporal-unit-reset
      approximation).
- [x] Add a per-`(obu_xlayer_id, metadata_type)` active-metadata store applying
      `muh_persistence_idc` and `muh_cancel_flag` with cross-layer propagation.
- [x] Diagnostics + positive/negative tests; advance `AV2-5.17.3-METADATA-GROUP` /
      `AV2-5.17-METADATA` `validate` (the group child row reached `done`; the umbrella
      stays `partial` until sections 3-4 resolve).

## 2. Scan-type CVS consistency (§ 6.16.10)

- [x] Cross-reference `metadata_scan_type` with content-interpretation `ci_scan_type_idc`
      (Table 6.18).
- [x] Enforce the CVS-wide single-`mps_pic_struct_type`-group constraint.
- [x] Diagnostics + tests; advance `AV2-5.17.10-METADATA-SCAN-TYPE`.

## 3. Decoded-frame-hash verification (§ 6.16.13) — blocked on a decoder

- [ ] (Blocked) Recompute MD5 over decoded output samples and compare to
      `plane_hash` / `frame_hash`. Requires the Phase 9/10 decoder.

## 4. Frame-unit suffix/prefix placement (§ 7.3.3 / § 7.3.4) — blocked on frame parsing

- [ ] (Blocked) Validate prefix-before-frame-data / suffix-after-frame-data placement
      inside coded frame units. Requires full frame-header + tile-group parsing
      (`AV2-5.18-FRAME-HEADER`, `AV2-5.19-TILE-GROUP`).
