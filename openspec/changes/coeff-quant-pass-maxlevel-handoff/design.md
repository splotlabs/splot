## Context

AV2 § 5.20.7.27 derives `maxLevel` in the same second ordinary non-FSC
coefficient pass that invokes § 5.20.7.28 `read_quant`. The existing
`apply_nonzero_coeff_quant_pass` boundary composes `read_quant` and signed
`Quant[]` writes, but still receives `CoeffQuantPassInput { max_level }` from
the caller.

The preceding `DECODE-COEFF-MAX-LEVEL-DERIVE` change made that derivation a
checked, testable helper. This change connects the two loaded boundaries without
wiring runtime `coeffs()` yet.

## Decisions

1. **Use the quant-pass hidden flag for max-level derivation.**

   The wrapper accepts only plane and transform class as max-level facts, then
   passes `CoeffQuantPassConfig::is_hidden` into the max-level derivation. This
   avoids two independently supplied hidden-parity values drifting apart.

2. **Delegate after deriving inputs.**

   The existing quant-pass composer remains the only implementation of
   preflight, literal reads, and signed `Quant[]` writes. The new helper derives
   records, converts them to `CoeffQuantPassInput`, then calls the composer.

3. **Stay loaded-but-unwired.**

   Runtime scan-table, transform-class, sign-source, base-symbol, hidden parity,
   TCQ, lossless, and `sumAbs1` derivation remain later work. This bridge only
   removes one per-coefficient caller fact.
