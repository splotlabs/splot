# Proposal: coded-extended-layer-unit and OrderHint constraints (§ 7.3.6, § 7.3.7 DOH)

## Feature IDs

- `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT` (the note's enumerated residual)
- `AV2-7.3.7-TEMPORAL-UNIT-ORDER` (the DOH-constraint checks)
- `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` (documented-blocked split of the
  § 6.4.1 residuals)

## Why

The frame-unit segmentation (PR #52) unblocked the
`AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT` note's whole
residual list and the § 7.3.7 DOH checks: the segmenter knows each layer's
frame units, their output classification, and their boundaries; `order_hint`
is parsed on the supported core paths. The § 6.4.1 output-timing residuals
remain genuinely decoder-blocked and must move from bare-partial wording to
documented-blocked.

## What Changes

Grounded in `07-decoding-process.md#s-7-3-6` (lines 517–617), `#s-7-3-7`
(lines 650–657), `#s-7-4-6` (lines 1316–1320), `06-…#s-6-4-1`:

1. **§ 7.3.6 in-unit OBU ordering** (`celu/` namespace, errors): the
   LCR → OPS → Atlas → SeqHdr → per-mlayer frame-unit order with ascending
   `obu_mlayer_id`, PADDING position-free.
2. **§ 7.3.6 constraint family** (errors; Unknown units never fire):
   - at least one coded output frame unit per CELU;
   - non-output-implies-output per embedded layer;
   - all output units share one OrderHint (order_hint from supported parse
     paths; Unknown drops the check for that CELU);
   - CLK/OLK only-first-frame-unit per layer + lowest-layer-first rules;
   - no CLK+OLK mix in one CELU;
   - all-leading-or-none;
   - CI only in the first frame unit of each embedded layer (the CELU-scoped
     § 7.3.6 form, complementing the § 7.3.8.10 TU-scoped check).
3. **§ 7.3.7/§ 7.4.6 DOH constraints** (errors, gated on the recorded
   `multistream_doh_constraint_flag` / `lcr_doh_constraint_flag` being 1):
   same OrderHintBits for all frame units in the temporal unit; same
   OrderHint across coded output frame units of multiple CELUs in the TU.
4. **§ 6.4.1 documented-blocked split**: the same-output-time and
   `get_disp_order_hint`/`explicit_ref_frame_map` operating-point
   consistency residuals get explicit decoder/output-timing-blocked notes
   (the OrderHint-agreement parts land via items 2–3); the monotonic
   OrderHint regression rule's full form stays deferred with its blocker
   named.

## Non-goals

- Output-order/timing modeling (the (c) residuals stay blocked, named).
- The § 7.3.6 bit-identity fingerprint cross-layer baseline (already
  retargeted at its row by PR #51).
- Frame-header parsing extensions (Unknown stays Unknown).

## Acceptance criteria

- [ ] Every rule: violation + boundary + Unknown-silence + PADDING
  transparency tests; the DOH checks gated both ways (flag 0 → silent);
  multi-CELU TU OrderHint agreement both directions.
- [ ] All established invariants applied (per-TU attribution, deferral
  where CVS membership matters, dedup, anchors).
- [ ] Matrix rows advance with proof; the § 6.4.1 split leaves no bare
  partial; registry/feature-status/ci/coverage gates pass.
