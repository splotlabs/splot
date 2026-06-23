## Context

The coefficient-loop frontier has been built in narrow crate-private pieces:
nonzero EOB derivation, checked scan walking, base/base-range symbol reads,
local `Level[]` writes, sign reads, and quantized-state writes. The latest
quantized-state helper still accepts caller-provided § 5.20.7.28 `read_quant`
outputs, so the next missing syntax boundary is the literal-bit parser that
turns a local `level` plus caller-resolved block facts into `(quant,
hrLevelAvg)`.

The AV2 v1.0.0 pseudocode for `read_quant` is in § 5.20.7.28 at
`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28`. It reads only `L(1)`
and `L(length)` bits, not CDF symbols, so it belongs in `splot-decode` near the
existing coefficient-loop syntax helpers and should remain independent of
`splot-recon`.

## Goals / Non-Goals

**Goals:**

- Add Feature ID `DECODE-COEFF-READ-QUANT-SYNTAX`.
- Implement a crate-private ordinary non-FSC `read_quant` parser that follows
  AV2 § 5.20.7.28 over caller-resolved inputs.
- Return the decoded `quant` and updated `hrLevelAvg` records that the existing
  quant-state writer can later consume.
- Exercise the threshold skip path, finite q-length path, Golomb extension path,
  hidden DC `lvlShift`, TCQ quant-step doubling, malformed-prefix handling, and
  overflow guards with focused tests.

**Non-Goals:**

- Do not wire the parser into runtime `coeffs()` yet.
- Do not derive `maxLevel`, `isHidden`, `hrLevelAvg`, `allowTcq`, scan order, or
  transform class inside this helper.
- Do not write `Level[]`, `QuantSign[]`, `Quant[]`, tile context lines, or CDF
  rows in this helper.
- Do not dequantize, run inverse transforms, add residuals, reconstruct pixels,
  update references, or invoke AVM/dav2d.

## Decisions

1. **Keep the helper caller-fact driven.**

   `read_quant` depends on values already derived in § 5.20.7.27: local level,
   raster position, hidden-parity state, `maxLevel`, current `hrLevelAvg`, and
   TCQ allowance. Passing these facts explicitly keeps the brick small and
   matches the surrounding coefficient-loop helpers.

2. **Read literal bits through the existing tile symbol-decoder wrapper.**

   The syntax uses `L(1)` q-length bits, `L(1)` Golomb-length bits, and
   `L(length)` coefficient remainder bits. Reusing the existing literal-read
   path keeps bit accounting and EOF behavior consistent with the EOB/sign
   helpers.

3. **Use checked arithmetic for every variable-width expression.**

   The spec expressions include shifts by caller-derived `m`, `k`, `length`, and
   optional TCQ doubling. Even though conformant streams bound these values,
   the helper should return typed errors for pathological caller facts or
   payloads instead of panicking.

4. **Leave runtime output unchanged.**

   This change creates the missing parser boundary only. The later `coeffs()`
   integration PR will compose EOB, scan, base, level, sign, `read_quant`, and
   quant-state writes into the runtime tile path.

## Risks / Trade-offs

- **Off-by-one in the q-length and Golomb loops** -> Tests pin both the finite
  `q < cMax` path and the `q == cMax` extension path, including consumed-bit
  behavior.
- **Hidden DC handling is easy to miss** -> Tests cover `pos == 0 && isHidden`,
  where `lvlShift = 1` affects both prediction and `hrLevelAvg`.
- **Unbounded length or shift values can overflow local arithmetic** -> The
  implementation uses checked shifts/adds/subtractions and typed errors.
- **Loaded-but-unwired status can be overclaimed** -> Matrix and roadmap notes
  keep runtime coefficient decode, reconstruction, and fixture output partial.
