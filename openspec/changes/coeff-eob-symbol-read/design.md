## Context

The decoder already has the pieces needed for the first nonzero `coeffs()` EOB
step: `TileCdfSelector::EobPt`, `TileCdfSelector::EobExtra`, mutable tile CDF
row handoff to `SymbolDecoder::read_symbol`, and the checked
`nonzero_coeff_eob` value helper. What is missing is the crate-private glue that
performs the § 5.20.7.27 read sequence for a caller-resolved transform-size EOB
class and produces the checked EOB value.

## Goals / Non-Goals

**Goals:**

- Read the caller-selected `eob_pt_*` CDF row and convert its symbol to `eobPt`.
- Read the `eob_pt_256_extra`, `eob_pt_512_extra`, or `eob_pt_1024_extra`
  literal bits when the size class and symbol require them.
- Read `eob_extra` and packed `eob_extra_bit` literal refinements for
  `eobPt >= 3`, then call `nonzero_coeff_eob`.
- Preserve CDF update behavior through the existing symbol decoder policy.

**Non-Goals:**

- No automatic transform-size-to-`EobPtSize` mapping, since broader transform
  syntax is not wired yet.
- No integration into the minimal flat-intra trace, which still decodes only
  all-zero coefficient blocks.
- No scan-order walk, coefficient base/br/sign reads, `Level[]` or `Quant[]`
  writes, `read_quant`, dequantization, inverse transform, residual add, or
  decoded output change.

## Decisions

1. Keep transform/EOB class facts caller-resolved.

   The helper takes `EobPtSize`, `coeff_cdf_q_ctx`, and `eob_ctx` directly.
   This preserves the dependency boundary: the helper does not invent transform
   size semantics and does not depend on reconstruction types.

2. Reuse `TileCdfSubset::read_block_symbol_trace` for the CDF handoff.

   That helper already validates selectors, uses the caller-owned
   `SymbolDecoder`, and obeys the configured CDF update policy. The new
   coefficient helper composes it rather than adding a parallel row-access path.

3. Return typed crate-private errors for symbol and literal failures.

   Selector/symbol failures propagate through the existing block-symbol read
   error; literal-bit failures are wrapped separately so later integration can
   distinguish CDF symbol failures from raw-bit EOB refinements.

## Risks / Trade-offs

- The helper remains loaded ahead of runtime use -> focused tests must prove the
  read sequence without claiming decoded output changes.
- Size-class extra-bit rules are easy to misplace -> keep them in a small helper
  and cover the 256/512/1024 extension cases.
- CDF mutation must stay transactional at higher layers -> this helper mutates
  rows as it reads, matching normal symbol-decoder semantics; callers remain
  responsible for checkpoint/rollback around unsupported broader decode paths.
