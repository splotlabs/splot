# Change: bru-ref-refresh-flag

## Feature IDs

- `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS`

## Why

Lands the §6.17.2 `use_bru` refresh-mask-bit clause — the last header-decidable BRU conformance
requirement (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md :4596):

> If use_bru is equal to 1, … the value of refresh_frame_flags & (1 << ref_frame_idx[bru_ref])
> must be non-zero.

A backward-reference-update frame must refresh the very reference slot it updates. Every
operand is parsed header state — `core.inter.refresh_frame_flags` (read on the inter path,
inter.rs:477), `bru_ref`, and `ref_frame_idx[bru_ref]` — so the check needs no reference-state
lookup (unlike the dims / `RefOrderHint` clauses), and is zero-false-positive.

## Scope

- Spec section: §6.17.2 (mirror :4596).
- `crates/splot-validate/src/context/reference_frames.rs`: in `reference_state_checks`, a new
  block gated on `use_bru == Some(true)` that fires `frame-header/bru-ref-refresh-flag-unset`
  when `refresh_frame_flags & (1 << ref_frame_idx[bru_ref]) == 0`. `bru_ref` is bounds-checked
  against the recorded `ref_frame_idx`, and the shift is guarded against an out-of-range slot.
- New diagnostic `frame-header/bru-ref-refresh-flag-unset` registered in
  `docs/VALIDATOR-DIAGNOSTICS.md`.
- Matrix `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` notes/diagnostics/proof; the residual inter-reference clauses get a
  precise `BLOCKED:` note (need §7.7 get_ref_frames / output-order-dependent get_disp_order_hint).

## Non-goals

- The remaining §6.17.2 use_bru clauses (`OrderHint >= RefOrderHint[i]`, `RESTRICTED_OH`,
  `RefCounter`-uniqueness) — `BLOCKED:` on unmodeled `get_ref_frames()` / output-order state.

## Acceptance criteria

- [ ] `frame-header/bru-ref-refresh-flag-unset` registered and emitted only for `use_bru == 1`
      frames whose `refresh_frame_flags` bit for `ref_frame_idx[bru_ref]` is clear.
- [ ] Negative: a BRU frame with the slot bit clear fires; positive: with the bit set, silent.
- [ ] No panic on an out-of-range slot index (shift-guarded).
- [ ] `cargo xtask ci` passes.
