## Context

`DECODE-COEFF-NONZERO-CONTEXT-COMMIT` added a wrapper that runs the
derived-base/derived-sign ordinary non-FSC pass and commits the final
§5.20.7.27 `culLevel` and `dcCategory` values through
`TileCoeffContextState::update_after_coeffs`.

That wrapper still accepts `AboveDcContext` and `LeftDcContext` as explicit
caller slices through `CoeffOrdinaryDerivedSignPassConfig`. Runtime
`coeffs()` integration should instead read those slices from the same
`TileCoeffContextState` it later updates. This keeps the state owner singular
and makes the required read-before-write ordering explicit before broader block
syntax starts calling the ordinary pass.

## Goals / Non-Goals

**Goals:**

- Add a crate-private state-backed ordinary nonzero handoff that reads
  `AboveDcContext[plane]` and `LeftDcContext[plane]` from
  `TileCoeffContextState` before running sign-source derivation.
- Reuse the existing derived-base/derived-sign ordinary pass and context-commit
  wrapper rather than duplicating coefficient syntax logic.
- Preserve existing lower-level explicit-slice APIs for focused tests and later
  staged composition.
- Prove read-before-write behavior and failure-state preservation in focused
  tests.

**Non-Goals:**

- No runtime `coeffs()` invocation, no `get_scan` handoff from real transform
  syntax, no real block-syntax plumbing for scan/transform/plane/geometry/parity
  or TCQ facts, no dequantization, no inverse transform, no residual add, no
  reconstruction, no decoded-output change, no public API, no encoder work, and
  no dependency or crate graph changes.
- No new AV2 constants, tables, CDF contents, or copied third-party material.

## Decisions

1. **Add a higher-level state-backed wrapper.** The existing
   `apply_nonzero_coeff_ordinary_pass_with_context_commit` remains useful for
   tests that need explicit context slices. The new wrapper takes
   `&mut TileCoeffContextState`, reads DC slices immutably first, then delegates
   to the existing commit wrapper.

2. **Clone DC context lines before mutating state.** Borrowing slices from
   `TileCoeffContextState` while also passing `&mut TileCoeffContextState` to
   the commit wrapper would create an aliasing conflict. Copying the small
   `u8` context lines into local vectors makes the read-before-write ordering
   explicit and keeps the lower-level sign-source config unchanged.

3. **Use one compact context config for read and write geometry.** The new
   wrapper should use the same plane and 4x4 geometry to read sign DC context
   inputs and later commit level/DC outputs. This avoids a split-brain caller
   API where sign derivation reads one plane while the commit writes another.

## Risks / Trade-offs

- **Risk: Extra context-line copies** -> The current wrapper is crate-private,
  loaded-but-unwired, and copies only tile-local `u8` DC lines. Runtime
  optimization can replace this with a split-borrow helper once broader
  integration needs it.
- **Risk: Overclaiming runtime support** -> Matrix, decoder support, roadmap,
  and OpenSpec rows must state that runtime `coeffs()` still does not call this
  wrapper and output remains unchanged.
- **Risk: Context-update failure after symbol consumption** -> This remains the
  same staged failure mode as the existing commit wrapper. The state update
  helper preflights ranges before mutating lines, and tests must prove invalid
  update facts preserve pre-existing context state.
