## Context

`DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS` composes the ordinary non-FSC
nonzero coefficient path through state-derived base selectors, local `Level[]`
writes, derived sign sources, `read_quant`, and signed `Quant[]` writes. The
result already carries a `NonZeroCoeffQuantPass`, whose final quant-state
summary exposes the `culLevel` and `dcCategory` values needed by the
§5.20.7.27 end-of-`coeffs()` context-line update.

The all-zero branch already commits those context lines through
`TileCoeffContextState::update_after_coeffs`. The nonzero branch needs the same
handoff, but only after all coefficient syntax and local state writes succeed.
This keeps runtime `coeffs()` integration staged while closing another
fabricated state boundary.

## Goals / Non-Goals

**Goals:**

- Add a crate-private wrapper around the derived-base/derived-sign ordinary
  pass that commits nonzero coefficient context lines after successful pass
  completion.
- Derive `CoeffContextUpdate` from caller-resolved plane/4x4 geometry plus the
  pass result's final `culLevel` and `dcCategory`.
- Preserve the pass result for downstream dequant/reconstruction work.
- Preserve transactional context behavior: coefficient-pass failures do not
  mutate tile context state, and invalid context-update facts do not partially
  mutate the context lines.
- Cover successful context writes and failure cases in focused tests.

**Non-Goals:**

- No runtime `coeffs()` invocation, no `get_scan` handoff from real transform
  syntax, no real block-syntax plumbing for plane/type/geometry/parity/TCQ or
  lossless facts, no dequantization, no inverse transform, no residual add, no
  reconstruction, no decoded-output change, no public API, no encoder work, and
  no dependency or crate graph changes.
- No new AV2 constants, tables, CDF contents, or copied third-party material.

## Decisions

1. **Add a wrapper instead of mutating the existing derived-base function.**
   The current `apply_nonzero_coeff_ordinary_pass_with_derived_base` remains a
   useful pure local-block composer for tests and future dequant handoff. A
   wrapper that additionally takes `&mut TileCoeffContextState` makes the
   context mutation opt-in and keeps failure boundaries explicit.

2. **Use `TileCoeffContextState::update_after_coeffs` directly.**
   The tile context state already owns the checked §5.20.7.27 above/left
   level/DC update. Reusing it avoids duplicating range validation or
   introducing a second state model.

3. **Commit only after the ordinary pass succeeds.**
   A failed base/sign/quant pass may have consumed symbols and mutated local CDF
   rows, but it must not publish incomplete coefficient facts into tile
   context lines. The wrapper therefore runs the ordinary pass first, then
   applies `CoeffContextUpdate`.

4. **Return the ordinary pass result after commit.**
   The next decoder bricks need the final local `Quant[]` block for
   dequantization and reconstruction. The context-commit wrapper should not
   hide that output.

## Risks / Trade-offs

- **Risk: Context-update failure after symbol consumption** -> This is inherent
  because the final `culLevel`/`dcCategory` are unavailable until after
  coefficient syntax succeeds. The wrapper must guarantee no partial context
  mutation on invalid update facts by relying on the existing checked update
  preflight.
- **Risk: Overclaiming runtime support** -> Matrix, decoder support, roadmap,
  and OpenSpec rows must state that runtime `coeffs()` still does not call the
  wrapper and output remains unchanged.
- **Risk: Test fixtures become too synthetic** -> Keep tests focused on the
  boundary: reuse the existing ordinary-pass fixture helpers, assert concrete
  context-line values, and include failure cases that prove context state is
  unchanged.
