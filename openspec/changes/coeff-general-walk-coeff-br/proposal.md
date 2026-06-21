## Why

Sub-brick 5b-i of the general multi-coefficient tokenizer. The base-tier walk
(5a) capped coefficient magnitudes at 4. Real quantized blocks reach larger
magnitudes, which the decoder reads with the §5.20.7.27 `coeff_br` base-range
token. This adds `coeff_br` for the END-OF-BLOCK coefficient (magnitude 1..=7).

The EOB coefficient is special: it is visited first in the reverse-scan base pass,
so its running `Level[]` is empty when its `coeff_br` context is derived — making
that context a CONSTANT (mirroring the decoder `CoeffBrContext::ctx` with an
all-zero `Level[]`). So this sub-brick needs no neighbour-offset table. The
data-dependent non-EOB `coeff_br` (which does need `Mag_Ref_Offset_With_Tx_Class`)
is a later sub-brick.

## What Changes

- Add `ENC-COEFF-GENERAL-WALK-COEFF-BR` as a private `splot-encode` encoder-tool
  feature.
- Extend `tokenize_general_lf_luma_block` so the EOB coefficient may carry magnitude
  1..=7: emit an interleaved `coeff_br` right after its `coeff_base_eob` when the
  magnitude exceeds the base tier, with the constant empty-`Level[]` context (0 at
  the DC raster position, 7 at a non-DC LF position) and symbol `mag - 5`. The
  non-EOB coefficient stays base-tier 1..=4.
- Extend `recover_quant_from_tokens` to read the interleaved `coeff_br` (EOB level =
  `coeff_base_eob + 1 + coeff_br`). Still §8.2 self-consistency, not decoder
  conformance.
- Add the routed `CoeffBrLf` ctx-7 and `CoeffBaseLf` 4x4 ctx-3 CDF rows (from the
  generated splot-core tables) and a reusable `coeff_br_lf_token` constructor. Make
  the scope validation position-aware (EOB coeff limit 7, non-EOB limit 4).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: extend the general low-frequency coefficient walk so the EOB
  coefficient carries the `coeff_br` base-range tier (magnitude 1..=7).

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/general_walk.rs`
  (+ tests), `multi_coeff.rs` (`coeff_br_lf_token`), `coefficient_tokenization.rs`,
  `block_symbol_trace/{cdf_rows,mod}.rs` + `coefficient_tokenization/cdf_rows.rs`
  (routed CDF rows).
- Scope (explicitly NOT claimed): the non-EOB coefficient's data-dependent
  `coeff_br`, magnitudes beyond 7 (golomb), eob > 2 / eob_extra, high-frequency or
  chroma coefficients, sizes other than 4x4, types other than DCT_DCT, packets, and
  decoder context conformance (the §8.2 roundtrip proves self-consistency only).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status
  / spec coverage.
