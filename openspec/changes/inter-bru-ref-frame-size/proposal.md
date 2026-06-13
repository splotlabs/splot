# Change: inter-bru-ref-frame-size

## Feature IDs

- `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS`
- `AV2-5.18.2-FRAME-HEADER-INFO`

## Why

A second §6.17.2 reference constraint decidable from the modeled inter frame-header + §7.23
state, the natural companion to `frame-header/ref-frame-scale-ratio`
(docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2 :4594-4595):

> If use_bru is equal to 1, it is a requirement of bitstream conformance that …
> RefFrameWidth[ ref_frame_idx[ bru_ref ] ] is equal to FrameWidth,
> RefFrameHeight[ ref_frame_idx[ bru_ref ] ] is equal to FrameHeight.

A backward-reference-update (BRU) frame writes into the reference it names via `bru_ref`, so
its dimensions must match that reference exactly. Both operands are already modeled: the
inter frame's resolved size (`core.frame_size`) and the §7.23 slot dims
(`SlotFacts.{width,height}`). The other `use_bru == 1` items are either already checked
(`immediate_output_frame == 1` → `bru-without-immediate-output`; `bru_ref < NumTotalRefs` →
`bru-ref-out-of-range`) or need the unmodeled `get_ref_frames()` `RefOrderHint` / inter
`refresh_frame_flags` derivation (`OrderHint >= RefOrderHint[i]`, the `RESTRICTED_OH` and
refresh-mask-bit items) and stay residual.

## Scope

- Spec section: § 6.17.2 (mirror :4594-4595).
- Crates/modules: `crates/splot-validate/src/context/reference_frames.rs`
  (`reference_state_checks`: one new block gated on `use_bru == Some(true)` + `Some(frame_size)`
  + `Some(bru_ref)` + a bounds-checked `ref_frame_idx[bru_ref]` slot that is `SlotState::Valid`).
- Diagnostics: new `frame-header/bru-ref-frame-size-mismatch` (error, § 6.17.2).
- Docs: matrix row notes/diagnostics/proof; `VALIDATOR-DIAGNOSTICS.md` registration.

## Non-goals

- The other `use_bru == 1` clauses (`OrderHint >= RefOrderHint[i]`, `RESTRICTED_OH`, the
  refresh-mask bit) and the `RefCounter` uniqueness rule — all need unmodeled reference-state
  derivation.
- Any reconstruction / entropy-decode work; the check reads no new bits.

## Acceptance criteria

- [ ] `frame-header/bru-ref-frame-size-mismatch` registered and emitted from
      `reference_state_checks` only for `use_bru == 1` frames with a resolved size and a
      proven-valid `bru_ref` slot.
- [ ] Negative: a BRU frame whose size differs from its `bru_ref` reference fires (and, when
      the dims are within the scale bounds, `ref-frame-scale-ratio` stays silent — the two are
      distinct).
- [ ] Positive: a matching BRU frame, an Unknown `bru_ref` slot, and a non-BRU frame all stay
      silent for this rule.
- [ ] `cargo xtask ci` passes.
