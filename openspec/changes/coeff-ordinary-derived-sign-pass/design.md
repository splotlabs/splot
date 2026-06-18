## Context

`DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS` composes the state-derived
base/level first pass into the ordinary non-FSC coefficient pass, but it still
accepts caller-fabricated `CoeffSignReadInput` records. The newly landed
`DECODE-COEFF-SIGN-SOURCE-DERIVE` helper can derive those sign inputs from the
post-first-pass `Level[]` state, first-pass `isHidden` / `sumAbs1`, plane,
transform class, and DC context-line facts.

The AV2 §5.20.7.27 sign branch belongs between the first-pass `Level[]` writes
and the interleaved sign/`read_quant` stage. Integrating the derivation at that
point removes one fabricated caller input from the derived-base composer while
keeping runtime `coeffs()` integration and tile context commits staged.

## Goals / Non-Goals

**Goals:**

- Replace caller-supplied sign inputs in the derived-base ordinary pass with
  sign-source derivation from the first-pass local block state.
- Carry only the required caller-resolved facts for sign-source derivation:
  coefficient CDF q-context, plane type, DC context slices, and transform-block
  4x4 coordinates/extent.
- Preserve the existing interleaved sign, `maxLevel`, §5.20.7.28
  `read_quant`, and signed `Quant[]` write sequencing.
- Expose derived sign inputs for tests/audit, matching the existing derived
  base input exposure.
- Cover explicit-vs-derived equivalence, hidden-parity sign derivation, and
  no-consumption failure before sign/quant syntax.

**Non-Goals:**

- No runtime `coeffs()` integration, no scan derivation from `get_scan`, no real
  syntax-to-plane/type/geometry plumbing, no nonzero tile context commits, no
  dequantization, no reconstruction, no decoded-output changes, no public API,
  no encoder work, and no dependency or crate graph changes.
- No new AV2 constants, tables, CDF contents, or copied third-party material.

## Decisions

1. **Change the derived-base input shape instead of adding a parallel helper.**
   The existing derived-base entry point is already the staged composer for
   removing fabricated first-pass facts. Extending its input with a
   compact derived-sign config keeps the surface narrow and avoids a second
   nearly identical ordinary-pass API. The wrapper then builds the lower-level
   `CoeffSignSourceDeriveConfig` from first-pass facts plus those caller
   context slices.

2. **Derive signs after the base/level first pass succeeds.**
   The helper needs the first pass's final local `Level[]` and hidden-parity
   summary. Running it after `apply_nonzero_coeff_base_derived_level_pass`
   matches the AV2 §5.20.7.27 ordering and keeps first-pass errors before any
   sign or quant consumption.

3. **Preflight derived sign inputs before interleaving.**
   The current interleaved pass expects preflighted sign levels so a sign-count
   or state mismatch stops before sign/quant syntax. The derived path should
   call `derive_nonzero_coeff_sign_inputs`, then
   `preflight_nonzero_coeff_signs`, then the existing interleaved loop.

4. **Keep the explicit ordinary pass unchanged.**
   `apply_nonzero_coeff_ordinary_pass` remains useful for staged tests and for
   validating explicit-vs-derived equivalence. This change only removes the
   caller-supplied sign inputs from the derived-base wrapper.

## Risks / Trade-offs

- **Risk: Overclaiming runtime support** -> Matrix, roadmap, and OpenSpec rows
  must state that runtime `coeffs()` still does not call the derived-base
  composer and output is unchanged.
- **Risk: Hidden-parity sign behavior diverges from the quant preflight** ->
  Tests must cover `isHidden && c == 0 && sumAbs1 > 0` so the derived sign
  source is not skipped for a zero-level DC carrier.
- **Risk: Invalid derived sign selectors occur after first-pass symbol
  consumption** -> That ordering is inherent because `Level[]` is unavailable
  until after the first pass. Tests should assert that a selector failure
  preserves the post-first-pass CDF and symbol position, and does not consume
  sign or quant syntax.
- **Risk: Existing tests fabricate all-sign-bit inputs** -> Update derived-base
  tests to compare against explicit ordinary passes fed the same derived sign
  inputs, rather than retaining caller-fabricated sign inputs in the derived
  path.
