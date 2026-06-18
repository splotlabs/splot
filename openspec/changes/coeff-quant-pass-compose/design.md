## Context

The ordinary non-FSC coefficient-loop frontier has been split into small
crate-private boundaries: nonzero EOB reading, checked scan walking,
base/base-range symbol reads, local `Level[]` writes, sign reads, `read_quant`
literal parsing, and quantized-state writes. The existing
`apply_nonzero_coeff_quant_state` helper still requires caller-provided
`CoeffQuantReadInput` records, while `read_nonzero_coeff_quants` produces those
records from the literal syntax in AV2 § 5.20.7.28.

This change composes those two helpers for the ordinary non-FSC second pass in
AV2 § 5.20.7.27. It remains caller-fact driven: the caller still provides the
checked scan walk, sign summaries, `maxLevel` values, hidden-parity state,
`sumAbs1`, TCQ enablement, lossless status, and initial `hrLevelAvg`.

## Goals / Non-Goals

**Goals:**

- Add Feature ID `DECODE-COEFF-QUANT-PASS-COMPOSE`.
- Add a crate-private helper that preflights sign, local-level, `maxLevel`, and
  target-`Quant[]` facts before consuming `read_quant` literal bits.
- Compose `read_nonzero_coeff_quants` and `apply_nonzero_coeff_quant_state` so
  tests can prove decoded `read_quant` values flow into signed `Quant[]` writes.
- Preserve the existing no-panic, typed-error discipline.

**Non-Goals:**

- Do not wire the helper into runtime `coeffs()` yet.
- Do not derive `maxLevel`, `isHidden`, `sumAbs1`, `useTcq`, lossless, sign
  sources, CDF selectors, scan tables, transform class, or transform type from
  real block syntax.
- Do not mutate `QuantSign[]` in this helper.
- Do not update tile context lines, dequantize, run inverse transforms, add
  residuals, reconstruct pixels, update references, or invoke AVM/dav2d.

## Decisions

1. **Keep one composition boundary for the second pass.**

   The new helper starts after local `Level[]` values and sign summaries exist.
   It owns the ordering between `read_quant` parsing and `Quant[]` writes, which
   is the next useful invariant to test before broader runtime wiring.

2. **Preflight before literal consumption.**

   The helper validates input counts, scan-entry identity, local level
   agreement, hidden-parity sign presence for `c == 0`, hidden-parity
   consistency with TCQ/lossless facts, `maxLevel - useTcq` validity, and target
   `Quant[]` addresses before calling `read_quant`. This prevents malformed
   caller facts from consuming bits before a later state writer rejection.

3. **Make `useTcq` the composed `read_quant` TCQ flag.**

   In the ordinary non-FSC branch, AV2 § 5.20.7.27 calls `read_quant(...,
   useTcq)`. The composed config therefore passes the same `use_tcq` flag to
   the `read_quant` parser's `allow_tcq` input and to the quant-state writer.

4. **Keep runtime output unchanged.**

   This change exposes the composition as a crate-private helper only. The
   runtime tile path still reaches only the existing supported all-zero
   coefficient frontier.

## Risks / Trade-offs

- **Double validation with lower-level helpers** -> The bridge duplicates a
  small amount of preflight logic so it can fail before consuming bits. The
  lower-level validators remain the final backstop.
- **Overclaiming runtime progress** -> The matrix and decoder-support rows stay
  explicit that this is loaded-but-unwired composition, not broad `coeffs()` or
  reconstruction support.
- **Configuration drift between `read_quant` and quant-state writes** -> The
  bridge owns a single `use_tcq` input and passes it consistently to both lower
  layers.
