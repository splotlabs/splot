# Proposal: model the reference-frame buffer state from parsed intra headers

## Feature IDs

- `AV2-7.23-REFERENCE-FRAME-UPDATE` (the per-slot update process — confirm
  the row id; create per the schema if absent)
- `AV2-5.18.2-FRAME-HEADER-INFO` (the reference-state view stops being an
  always-unknown placeholder)

## Why

`FrameReferenceStateView` exists but the validator always passes
`unknown()` and the parser never branches on it. The state is derivable
today for intra streams: `refresh_frame_flags`, `order_hint_lsb`, and
`frame_size` are parsed on every completed intra header, and the § 7.23
reference frame update process defines exactly how slots update. With a
real model, every reference-state-gated § 5.18 branch and § 6/§ 7 check
that needs per-slot `RefValid`/`RefOrderHint`/`RefFrameWidth/Height`
becomes reachable; the inter syntax change (next) consumes it.

## What Changes

1. A per-extended-layer per-slot reference-state tracker in the validator,
   updated per § 7.23 (read it verbatim) from each frame's parsed
   `refresh_frame_flags`/`OrderHint`/dimensions at the frame's decode
   point (segmenter-authoritative frame boundaries; the SEF/no-refresh and
   CLK/CVS-reset semantics per the spec — ground each).
2. Honest poisoning: a frame whose refresh mask is NOT parsed (inter/TIP/
   bridge stops, truncations, ambiguous boundaries) poisons the affected
   slots (all slots when the mask is unknown) until the next grounded
   reset; per-slot Unknown is the resting state, never a guess.
3. Thread the derived state into `FrameHeaderParseInput.reference_state`
   where the parse consumes it (the parser-side branches stay for the
   inter change unless a § 5.18 intra branch consumes it today — check).
4. Any § 6/§ 7 check that becomes locally decidable with the modeled state
   and is unambiguous gets a diagnostic with citation (e.g. SEF
   `frame_to_show_map_idx` referencing an invalid slot — find the exact
   clause); otherwise named residuals.
5. `inspect` is unchanged unless a natural surface exists (validator-state
   work, not new syntax).

## Non-goals

- Inter-path syntax parsing (frame-header-inter-reference-paths).
- §7.3.9 long-term reference availability (item 23's scope).
- Output-order/decoder-model semantics.

## Acceptance criteria

- [ ] Slot state tracks § 7.23 for intra streams (update/reset/SEF
  semantics each tested); poisoning on unparsed masks tested; any new
  diagnostic has violation + boundary + Unknown-silence + both-order
  tests; matrix proof; `cargo xtask ci` green.
