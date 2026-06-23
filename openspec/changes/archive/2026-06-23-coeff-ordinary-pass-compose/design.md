## Context

AV2 § 5.20.7.27 performs the ordinary non-FSC nonzero coefficient path as a
sequence: start from nonzero EOB, walk `scan[c]`, read base/base-range symbols,
write local `Level[]`, then for each checked coefficient read its sign, derive
`maxLevel`, invoke § 5.20.7.28 `read_quant`, and write signed `Quant[pos]`.
The coefficient block also starts that quant pass with `hrLevelAvg` reset to 0.

The repository already has each of those steps as separate crate-private
helpers. They are intentionally caller-fact-driven because runtime transform
syntax, evolving coefficient context selector derivation, scan derivation,
post-level sign-source selection, hidden parity, sumAbs1, TCQ, and lossless facts
are not yet wired into broad `coeffs()`.

## Goals / Non-Goals

**Goals:**

- Add one crate-private helper that composes the existing ordinary non-FSC
  helpers from `NonZeroCoeffBlockStart` through signed `Quant[]` writes.
- Preserve transactional preflight where possible: scan-walk and caller-fact
  mismatches fail before later symbol reads or block mutation.
- Preserve the spec-order shared symbol stream by interleaving each sign read
  with that coefficient's `read_quant` and signed `Quant[pos]` write.
- Reset `hrLevelAvg` to 0 at coefficient-block entry before the first
  `read_quant` call.
- Return the checked scan walk, base reads, sign reads, and quant-pass summary so
  later runtime wiring can inspect and reuse the intermediate facts.

**Non-Goals:**

- Do not derive scan tables from `get_scan`, transform class from real
  `PlaneTxType`, evolving base CDF selectors, or post-level sign sources from
  § 8.3.2 runtime state.
- Do not commit above/left tile coefficient context lines for nonzero blocks.
- Do not wire the helper into the minimal runtime, widen accepted streams,
  dequantize, inverse-transform, add residuals, reconstruct pixels, or compare
  against AVM/dav2d.

## Decisions

1. **Add a separate composition module.**

   `ordinary_pass.rs` will orchestrate existing helpers instead of growing
   `quant_pass.rs` or `coeff_loop.rs`. This keeps each lower-level primitive
   focused and avoids pushing large files toward the source-line budget.

2. **Consume and return owned state.**

   The helper consumes `NonZeroCoeffBlockStart`, then returns a final
   `TransformCoeffBlockState` reference through the existing quant-pass summary.
   This matches the existing state handoff pattern and avoids sharing partially
   mutated local block state with callers.

3. **Keep caller-resolved inputs explicit.**

   The wrapper accepts the scan slice, base read inputs, sign read inputs,
   `CoeffQuantPassMaxLevelConfig`, and hidden/sumAbs1/TCQ/lossless facts through
   `CoeffQuantPassConfig`, while normalizing `hrLevelAvg` to the block-entry
   value required by the spec. This is verbose, but it keeps all unimplemented
   runtime derivation honest and prevents the helper from inventing AV2 selector
   semantics.

## Risks / Trade-offs

- Composition can make symbol consumption harder to reason about ->
  focused tests will cover success ordering and bad-fact no-consumption for
  errors that can be rejected before any new symbol read.
- The helper still leaves many caller facts unresolved -> matrix and support rows
  remain `partial`, and notes will explicitly list runtime base-selector,
  sign-source, scan, transform, context-line, and reconstruction gaps.
