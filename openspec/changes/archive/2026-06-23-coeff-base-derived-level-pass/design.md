## Context

The current ordinary non-FSC composer is faithful in broad phase order but still
uses caller-supplied `base_inputs`: every `coeff_base_eob`, `coeff_base`, and
`coeff_br` selector is precomputed before the helper runs. AV2 § 5.20.7.27 does
not work that way for ordinary non-FSC blocks. The first pass walks from
`eob - 1` down to `0`, derives the current coefficient's base selector from
§ 8.3.2 state, reads the symbol, writes `Level[row][col]`, and only then moves to
the next coefficient so later selectors see the evolved `Level[]` block and TCQ
state.

The repository already has the pieces this needs: checked scan entries, local
`TransformCoeffBlockState`, base/base-range symbol read primitives, § 8.3.2
`coeff_base_eob`, `coeff_base`, and `coeff_br` context derivations, and the TCQ
state table used by the second pass. This change composes those pieces for the
ordinary non-FSC first pass only.

## Goals / Non-Goals

**Goals:**

- Add a crate-private helper that derives ordinary non-FSC base/base-range CDF
  selectors while iterating over checked scan entries in § 5.20.7.27 first-pass
  order.
- Write each decoded level into local `Level[]` immediately after its base and
  optional base-range symbols are read.
- Track first-pass `tcqState`, `sumAbs1`, `numNz`, and derived `isHidden` so a
  later change can feed them into sign and quant-pass integration.
- Preserve transactional preflight where possible: static geometry/fact errors
  fail before symbol consumption, while symbol/CDF failures leave the already
  consumed prefix explicit in the returned error boundary.

**Non-Goals:**

- Do not wire the helper into runtime `coeffs()` or widen accepted decode input.
- Do not derive scan tables from `get_scan`, transform class from
  `PlaneTxType`, plane/lossless/TCQ/parity flags from real frame syntax, or
  sign sources for the second pass.
- Do not support FSC/IDTX base rows in this ordinary non-FSC helper.
- Do not commit above/left tile coefficient context lines for nonzero blocks,
  dequantize, inverse-transform, add residuals, reconstruct pixels, or compare
  against AVM/dav2d.

## Decisions

1. **Add a first-pass module beside the existing base and level helpers.**

   A small `base_level_pass.rs` module will own the state-derived orchestration
   instead of expanding `base_symbol.rs` or `ordinary_pass.rs`. The existing
   caller-supplied `read_nonzero_coeff_base_symbols` remains useful for focused
   lower-level tests and for comparing derived selectors against hand-authored
   fixtures.

2. **Reuse existing context derivations and selector types.**

   The helper maps `CoeffBaseSelection` to `CoeffCdfSelector` using
   caller-supplied `coeff_cdf_q_ctx`, `tx_size_ctx`, and current TCQ context. This
   keeps CDF row bounds centralized in the existing `CoeffCdfSelector`/CDF row
   access layer and avoids adding new generated tables. The
   parity-hidden-only `CoeffBaseSelection::Ph` bank is not loaded yet, so this
   change reports a typed unsupported boundary if that row is actually reached
   instead of pretending the CDF exists.

3. **Expose a shared TCQ transition helper.**

   The first pass needs `Tcq_Next_State[tcqState][level & 1]` before the second
   pass resets `tcqState` to 0. The existing table in `quant_state.rs` will gain a
   small crate-private transition function so both first-pass selector derivation
   and second-pass quant writes use the same source.

4. **Return first-pass summary separately from the block.**

   The result should expose decoded base reads, final local block state, and a
   summary containing `sumAbs1`, `numNz`, `isHidden`, and final first-pass
   `tcqState`. The later ordinary-pass composer can then replace caller-supplied
   hidden/sumAbs1 facts without re-reading the symbol stream.

## Risks / Trade-offs

- Derived selector mapping can drift from § 8.3.2 -> tests will compare selected
  rows against explicit selectors for EOB, low-frequency, high-frequency, chroma,
  BR, and TCQ-dependent cases.
- Immediate `Level[]` writes reduce all-or-nothing transactionality after symbol
  reads begin -> tests will distinguish preflight errors that preserve state from
  reached symbol/CDF errors that may consume a prefix, matching parser-style
  behavior.
- The helper still leaves scan/transform/runtime facts caller-resolved -> matrix,
  support, and roadmap notes remain partial and explicitly list those deferred
  runtime integrations.
