## Context

The block-symbol context derivation has covered `y_mode_index` and `uv_mode`.
The remaining two trace literals are the `all_zero` contexts for the luma
`txb_skip` and the V-plane `v_txb_skip` symbols. This change implements the
§ 8.3.2 `all_zero` formula and uses it with the level-context contribution that
is genuinely derivable for the first transform block.

## Goals / Non-Goals

Goals:

- Implement the § 8.3.2 `all_zero` context formula (luma and V) as reusable,
  unit-tested functions.
- Use them in the trace with the first-block level context derived, replacing the
  bare literals, with no change to decoded output.

Non-Goals:

- The § 5.20 transform-block syntax, the level-context / DC-context buffers, the
  coefficient decode, `EobU` derivation, `fsc_mode` / `txSz` / residual geometry,
  and the U-plane `txb_skip` branch.

## Decisions

- **Formula now, full inputs later.** The § 8.3.2 `all_zero` context decomposes
  into a level-context contribution (an OR-reduction of the `Above`/`Left` level
  buffers, clamped) and a transform-block-geometry contribution
  (`tx_fills_block`, `chroma_block_larger_than_tx`) plus `fsc` / `EobU`. The
  formula is implemented as pure functions taking these as inputs; the buffers and
  geometry come from the § 5.20 transform-block syntax, which is deferred.
- **First-block level context is derived.** For the first transform block at the
  tile origin there are no prior decoded transform blocks and the above/left 4x4
  neighbours are out of frame, so every `Above`/`Left` level (and DC) value is 0
  — the same out-of-frame justification used for `y_mode_index`. The U plane is
  decoded all-zero immediately before the V symbol, so `EobU == 0`. These inputs
  are therefore supplied as derived values, not asserted.
- **Geometry is asserted (with `TODO(spec)`).** `tx_fills_block` (luma) and
  `chroma_block_larger_than_tx` (V) depend on the real `txSz` / residual block
  size, which the toy trace does not faithfully model. An empirical probe proved
  the conformant fixture *forces* the luma context to 0 (so the transform fills
  its block) and the V context to 3 (so the chroma block exceeds the transform):
  flipping either literal fails the no-output-change snapshot. So the geometry is
  asserted to those forced values with a `TODO(spec)` to derive it from the
  § 5.20 transform-block syntax.
- **No-output-change.** `txb_skip_ctx_luma(0, 0, true, false) == 0` and
  `v_txb_skip_ctx(false, false, true, false) == 3` match the previous literals,
  so the frozen-trace snapshot then named
  `block_symbol_frontier_accepts_minimal_fixture_trace` stayed green at the time of
  this change. (That snapshot test was later retired by
  `decode-minimal-fixture-avm-skip-polarity`, which replaced the committed fixture
  with a conformant general-path luma-skip stream; the frozen trace is now covered
  by `block_symbol_trace_rejects_legacy_inverted_skip`.) Unit tests pin the formula
  directly (filling/non-filling/fsc for luma; the neighbour/chroma/EobU
  contributions for V).

## Risks / Trade-offs

- The transform-block geometry remains caller-asserted rather than derived; this
  is explicitly marked `TODO(spec)` and bounded to the fixture-forced values.
  The durable value is the spec-exact `all_zero` formula, reused unchanged when
  the § 5.20 transform-block syntax supplies the real geometry and buffers.

## Migration Plan

Additive: two new functions plus a one-call-each change in `consume_trace`. No
public API change; the runtime and the decoded output are unaffected.

## Open Questions

None — the empirical probe resolved whether the literals are forced (they are).
