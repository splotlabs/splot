## Context

The residual path has the § 7.14.2 quantizer functions (`dc_quantizer` /
`ac_quantizer`) and the § 7.15 inverse transforms. The § 7.14.4 dequantization
process is the arithmetic step between coefficient decode and the inverse
transform: it scales each coded `Quant` coefficient by its quantizer (and,
optionally, a quantization-matrix weight) into the `Dequant` array.

## Goals / Non-Goals

Goals:

- Implement the § 7.14.4 per-coefficient dequant (steps 3-8) and the
  transform-block application exactly, total and panic-free.

Non-Goals:

- The quantization-matrix weighting (`Quantizer_Matrix` / `UserQm`), the `shift`
  / `useFsc` / `allow_tcq` derivation, the coefficient entropy decode, and the
  inverse-transform invocation.

## Decisions

- **Caller-resolved quantizer and denominator.** The per-coefficient quantizer
  `q2` (the § 7.14.2 DC/AC quantizer, optionally weighted `Round2(q * m, 5)`) and
  `dq_denom = 1 << shift` are inputs, mirroring how the inverse transform takes
  caller-resolved shifts. This keeps the dequant arithmetic free of the
  quantization-matrix tables and the `shift` derivation (which pulls in
  `allow_tcq` / `useFsc` / plane-and-segment state) — both deferred.
- **Per-coefficient core + block helper.** `dequant_coefficient` is the spec
  steps 3-8: `sign`, `dqHigh = Abs(qc) * q2`, `dq = Round2(dqHigh & 0xFFFFFF,
  QUANT_TABLE_BITS=3)`, `dq2 = sign * (dq / dq_denom)`,
  `Clip3(-(1 << (7 + BitDepth)), (1 << (7 + BitDepth)) - 1, dq2)`.
  `dequantize_block` applies it over the `tx_width * tx_height` block, choosing
  `dc_quant` for the `(0, 0)` coefficient and `ac_quant` otherwise — the
  non-quantization-matrix path where every AC coefficient shares one quantizer.
  (Per-position quantization-matrix weighting would need a per-coefficient `q2`;
  that is the deferred extension, supported through `dequant_coefficient`.)
- **Totality.** The product and rounding use `i64`; `unsigned_abs` handles
  `i32::MIN`; a zero `dq_denom` is treated as 1; the `Clip3` bound keeps the
  result within `i32` and the documented range. Invalid shapes and buffer
  lengths return typed `ReconError`.

## Risks / Trade-offs

- The block helper covers only the non-quantization-matrix path; the
  quantization-matrix weighting (per-position `m`) is deferred, but
  `dequant_coefficient` already accepts a pre-weighted `q2`, so the weighting can
  be layered on without changing the core.

## Migration Plan

Additive; a new module and two new `ReconError` variants. No existing API
changes, and the runtime is unaffected.

## Open Questions

None.
