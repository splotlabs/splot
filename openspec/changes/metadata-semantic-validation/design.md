# Design: Metadata semantic / lifetime validation

## Context

`metadata-padding-foundation` delivered the metadata parsers and the locally-decidable
§ 6.16 diagnostics. The remaining § 6.16 rules are stateful or capability-blocked. This
change scopes them and records the intended approach so they can be implemented when
their dependencies land. It is a tracking design; no code ships with this change.

## Item 1 — persistence / cancellation lifetime (§ 6.16.3)

A per-`(obu_xlayer_id, metadata_type)` "active metadata" store, scoped to a coded video
sequence, updated as metadata OBUs are observed:

- `muh_persistence_idc`: GLOBAL (persists for the CVS; `muh_cancel_flag` is a no-op),
  BASIC (persists until same-type metadata for the layer or a cancel), NO (current frame
  only), ENHANCED (basic with partial update).
- `muh_cancel_flag`: cancels previously-signaled metadata of the type for the current
  extended layer (or a set of layers when global).
- Cross-layer propagation via `TLayerDependencyMap` / `MLayerDependencyMap`.

**Blocker:** the dependency maps are not exposed by the sequence-header model, and the
exact CVS boundary is only approximated today. Both are prerequisites.

## Item 2 — scan-type CVS consistency (§ 6.16.10)

Correlate `metadata_scan_type` with the content-interpretation `ci_scan_type_idc` per
Table 6.18, and enforce the CVS-wide constraint that `mps_pic_struct_type` stays within
one of the three permitted groups for all pictures of the CVS. Requires CVS-scoped state
and a content-interpretation ↔ scan-type cross-reference.

## Item 3 — decoded-frame-hash verification (§ 6.16.13)

Recompute the MD5 over the decoded output samples (per-plane or whole-frame, per
`per_plane` / `has_grain` / `is_monochrome`) and compare against the parsed
`plane_hash` / `frame_hash`.

**Blocker:** requires a decoder (entropy coding + reconstruction + the § 7.21 output
process), which `splot` does not have. This is Phase 9/10 conformance work.

## Item 4 — frame-unit suffix/prefix placement (§ 7.3.3 / § 7.3.4)

Validate that prefix metadata (`metadata_is_suffix == 0`) appears before the frame data
and suffix metadata after it, within a coded frame unit.

**Blocker:** requires full frame-header + tile-group parsing to locate the frame-data
boundary. The coarse temporal-unit prefix/suffix/coded-layer classification already
ships in `metadata-padding-foundation`; this is the precise in-frame-unit refinement.

## Sequencing

Items 2 and 1 are the validator-achievable next steps (in roughly that order, gated on
CVS-boundary modeling and the dependency maps). Items 3 and 4 are gated on the decoder
and frame/tile parsing phases respectively and may be split out to those phases' changes
when they begin.
