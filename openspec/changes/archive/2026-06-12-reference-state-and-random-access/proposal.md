# Proposal: reference-slot and random-access conformance (header-state-only)

## Feature IDs

- `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` (todo → the § 7.3.9.1
  rules)
- `AV2-7.4-RANDOM-ACCESS` umbrella rows for § 7.4.2/.4/.5 (confirm the
  row ids; create per schema if absent)
- `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` (the reference residuals)
- `AV2-5.18.6-QUANTIZATION` / `AV2-5.13-QUANTIZATION-MATRIX` (the
  § 7.3.8.9 QM availability half + QmProtected resets +
  § 6.17.6.2 layer-dependency TODO)
- `AV2-5.8.8-LCR-EMBEDDED-LAYER-INFO` (§ 6.8.9 expected-dims bounds)

## Why

The § 7.23 slot model (PR #62) and the inter control region (PR #63) make
the random-access conformance tranche header-decidable — no pixel decode
is needed. The audit enumerates the unblocked checks; several halves
already landed (the SEF slot check, the MFH stored-dims bound, the FGM
availability check, primary_ref_frame's range clause) — this change
completes the group without duplicating them.

## What Changes

Ground each in the mirror and implement what is decidable; name the rest:

1. § 7.3.9.1 long-term reference availability (07 mirror — read the whole
   section) and the RAP-CELU CLK-then-OLK first-frame-units rule with
   `immediate/implicit_output == 0` (07:953-968).
2. § 7.4.2 long-term-reference preconditions; § 7.4.4 OLK rules in their
   header-observable form (ref_long_term_id iff-conditions, the
   OrderHint < 1<<OrderHintBits bound when long_term_frame_id_bits > 0,
   the no-back-reference rules where header-decidable; 07:1168-1219);
   § 7.4.5 RAS rules (07:1276-1289).
3. § 7.3.8.9 using_qmatrix reference availability + the QmProtected
   CLK/OLK/SWITCH/RAS reset (the SWITCH arm consumes the parsed
   restricted_prediction_switch from PR #63); the § 6.17.6.2 QM
   layer-dependency checks (the explicit TODO in context.rs — the maps
   are exposed by the sequence model).
4. § 6.8.9 lcr_max_expected_width/height bounds against parsed frame
   dimensions.
5. Remaining decidable § 6.17.2 reference clauses (check what PRs
   #62/#63/#54 already landed; complete the rest, e.g. the
   primary_ref_frame validity beyond the range clause, 06:4500-4508).
6. Established invariants throughout: per-key external-HLS suppression
   where declarable kinds are involved, RAP-replay visibility care,
   zero false positives (poisoned/Unknown drops), dedup, both arrival
   orders, anchors at the offending OBU.

## Non-goals

- Output-order-dependent rules (deferred in celu-orderhint-constraints).
- Pixel/decode semantics; § 5.20 payloads.

## Acceptance criteria

- [ ] Each implemented rule: violation + boundary + Unknown-silence +
  suppression + both-order tests with citations; no duplicate of landed
  checks; matrix proof per row; `cargo xtask ci` green.
