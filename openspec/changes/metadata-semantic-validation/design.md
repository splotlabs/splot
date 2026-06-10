# Design: Metadata semantic / lifetime validation

## Context

`metadata-padding-foundation` delivered the metadata parsers and the locally-decidable
§ 6.16 diagnostics. The remaining § 6.16 rules are stateful or capability-blocked. This
change scopes them; items 1 and 2 (with their prerequisites) are **implemented**, while
items 3 and 4 remain tracking entries for when their dependencies land.

## Item 1 — persistence / cancellation lifetime (§ 6.16.3) — implemented

A per-`(obu_xlayer_id, metadata_type)` "active metadata" store
(`crates/splot-validate/src/metadata_lifetime.rs`), scoped to a coded video sequence,
updated as metadata OBUs are observed:

- `muh_persistence_idc`: GLOBAL (persists for the CVS; `muh_cancel_flag` is a no-op),
  BASIC (persists until same-type metadata for the layer or a cancel), NO (current frame
  only), ENHANCED (basic with partial update; modeled as BASIC with a marker — the spec
  gives no merge algorithm).
- `muh_cancel_flag`: cancels previously-signaled metadata of the type for the current
  extended layer (or all extended layers when the cancel OBU is global — group cancel
  units carry no layer maps per § 5.17.3).
- Cross-layer propagation via `TLayerDependencyMap` / `MLayerDependencyMap` as a pure,
  never-flagging applicability query (the § 6.16.3 propagation bullets are decoder
  applicability rules, not conformance requirements).

Alongside the store, the § 6.16.3 reserved-value warnings and the § 6.16.5/§ 6.16.6 HDR
CLL/MDCV repeat-content checks landed. The former blockers are resolved: the
sequence-header model now exposes the § 5.4.1 dependency maps, and the exact § 7.3.6
CVS boundary (CLK per extended layer per temporal unit, with cross-temporal-unit
comparisons deferred to the temporal-unit boundary) replaced the temporal-unit-reset
approximation.

## Item 2 — scan-type CVS consistency (§ 6.16.10) — implemented

Correlates `metadata_scan_type` with the content-interpretation `ci_scan_type_idc` per
Table 6.18, and enforces the CVS-wide constraint that `mps_pic_struct_type` stays within
one of the three permitted groups for all pictures of the CVS, using per-xlayer scopes
plus a global bucket over the exact CVS boundary. Reserved `mps_pic_struct_type` values
are excluded from the state; the mirror defines no `mps_source_scan_type_idc` ↔
`ci_scan_type_idc` consistency rule, so none is checked.

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

Items 1 and 2 (and their prerequisites — the dependency maps and the exact CVS
boundary) have landed. Items 3 and 4 are gated on the decoder and frame/tile parsing
phases respectively and may be split out to those phases' changes when they begin.
