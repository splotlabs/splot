## Context

`FrameHeaderCore` is the § 5.18.2 model both the parser produces and
`write_frame_header_core` consumes. It is `#[non_exhaustive]` with crate-private fields,
so an external crate cannot build it by literal; within `splot-core` a sibling module can,
but a field-by-field literal of a frame header is large, fragile, and generalizes poorly
(each hardcoded field is a "doesn't generalize" hazard). The frozen decoder minimal tier
accepts exactly one shape: a 64x64, `base_q_idx == 255`, single-picture
`OBU_CLOSED_LOOP_KEY` intra frame.

## Goals / Non-Goals

- Goal: produce a conformant `FrameHeaderCore` for that frozen tier, with the strongest
  possible correctness oracle.
- Non-Goal: a general frame-header builder, inter frames, non-64x64 sizes, a tile-group
  OBU, a frame, a packet, or any `receive_packet` output. Those are later bricks.

## Decision: parse-backed assembly

`build_minimal_intra_clk_core` serializes the canonical § 5.18.2 body with `BitWriter`
(one write per syntax element, each spec-cited) and then **parses** it with the real core
parser. The returned core is therefore conformant by construction — it is exactly what the
decoder would yield for that byte stream. This avoids the literal-ctor generalization
hazards: there is one byte sequence, validated by the same parser the decoder runs.

The body's bit layout depends on the activated sequence shape, so the assembler is paired
with `new_minimal_intra_single_picture`, whose eight § 5.4.x inferences make the body
spec-real (notably `OrderHintBits == 0`, so `order_hint` is `f(0)` — omitted — and SCC
`SELECT`, so an explicit `allow_screen_content_tools` bit is read). The existing test
fixture `single_picture_clk_bits` is **not** a spec-real oracle for this — it is
self-consistent with a non-degenerate `order_hint_bits == 4` / SCC `== 0` test sequence —
so this brick adds its own round-trip.

## Oracle

The round-trip is the proof: parse the canonical body against the single-picture view,
assert the derived facts (`IntraHeaderComplete`, Key, 64x64, `order_hint_lsb == 0`,
`refresh_frame_flags == 3`, immediate-output), then `write_frame_header_core` the core and
assert the re-emitted stream reparses to an equal core. The single-picture constructor's
eight inferences are thus verified end-to-end, not just by field assertion.

## Error model

`build_minimal_intra_clk_core` returns `Result<FrameHeaderCore, MinimalIntraCoreError>`
with `Body(WriteError)` / `Parse(crate::error::Error)` arms (`#[from]`). Both are
unreachable for the fixed canonical input; they exist only so the function honors the
no-panic library policy without `unwrap`/`expect` on the internal `BitWriter` / parser
results.

## Risks

- The canonical body must match the single-picture view's bit layout exactly; a wrong bit
  or a mismatched seq would mis-parse. Mitigated by the round-trip oracle (a mismatch
  fails the test) and by deriving the layout from the adversarially-verified blueprint.
