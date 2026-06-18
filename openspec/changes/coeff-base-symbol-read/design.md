## Context

`DECODE-COEFF-BASE-CDF-ROWS` exposed the ordinary non-IDTX coefficient
base/base-EOB/base-range rows in the tile CDF subset. `DECODE-COEFF-SCAN-WALK`
already returns checked reverse-order scan entries after nonzero EOB syntax.
The remaining gap at this boundary is the §5.20.7.27 symbol sequence that reads
`coeff_base_eob` for the final scan entry, `coeff_base` for the others, and
conditionally `coeff_br` when the decoded base level crosses the caller-resolved
base-level threshold. This change stays inside `splot-decode` and does not add
dependencies or runtime output behavior.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-COEFF-BASE-SYMBOL-READ` as a focused feature row and
  decoder-support row.
- Add a new `coeff_loop/base_symbol.rs` helper that consumes a
  `NonZeroCoeffScanWalk` plus caller-resolved coefficient CDF selectors and
  reads the ordinary non-FSC base/base-EOB/base-range symbol sequence.
- Preserve row-selection and symbol-decoder transactional behavior: selector
  errors occur before consumption, and disabled CDF updates leave selected rows
  unchanged.
- Return decoded level-building symbols and accumulated level values without
  writing `Level[]`, `Quant[]`, or tile context lines.
- Prove the boundary with self-contained unit tests.

**Non-Goals:**

- No runtime `decode_block()` / `coeffs()` integration and no decode-output
  change.
- No derivation of `get_scan`, `compute_tx_type`, `get_lf_limits`, `baseLevels`,
  `tcqState`, or parity-hiding facts from real block syntax.
- No FSC/IDTX/parity-hidden-only CDF rows, sign reads, `dc_sign`, `idtx_sign`,
  nonzero coefficient-state writes, `read_quant`, dequantization, inverse
  transform, residual add, reconstruction, reference refresh, public API,
  AVM/dav2d invocation, scheduler change, or dependency change.

## Decisions

1. Keep selector derivation caller-owned.

   Rationale: the repo already has total §8.3.2 context helpers
   (`coeff_base_eob_ctx`, `CoeffBaseContext`, `CoeffBrContext`) and checked
   scan-walk entries. This boundary should sequence reads over caller-provided
   selectors rather than mixing in broader transform-type, LF, TCQ, or
   parity-hiding derivations that still need real block syntax.

   Alternative considered: derive every selector inside the new read helper.
   That would couple this PR to transform-type, LF, and future coefficient-state
   mutation choices, making the change too broad.

2. Model BR as an explicit enabled/disabled per-entry read.

   Rationale: AV2 §5.20.7.27 reads `coeff_br` only when the decoded base level is
   above `baseLevels` and the branch is not low-frequency chroma. A
   caller-provided `CoeffBaseRangeRead` keeps the helper honest about read
   ordering without forcing this boundary to own LF/plane policy derivation.

   Alternative considered: always require a BR selector and read it when level
   crosses the threshold. That would make chroma low-frequency entries harder to
   express and blur the caller-owned `!(isLf && plane > 0)` condition.

3. Put implementation and tests in new modules.

   Rationale: `coeff_loop.rs` is at the 1000-line advisory budget. A new module
   keeps the root file small and keeps the coefficient base-read tests separate
   from the EOB/scan-walk tests.

   Alternative considered: extend `coeff_loop.rs` and `eob_symbol_tests.rs`
   directly. That would make both files harder to review and more likely to
   trip source-line warnings.

## Risks / Trade-offs

- Caller-owned selectors can be mismatched with scan entries -> mitigate by
  checking the input slice length and exact scan-entry identity before each
  read.
- A BR selector could be invalid but not reached because the base level does not
  cross the threshold -> this matches the spec read order; tests should prove
  unreachable invalid selectors do not consume symbols.
- It is easy to overstate runtime support -> matrix, decoder support, roadmap,
  and OpenSpec text must keep this partial and explicitly exclude state writes
  and reconstruction.
