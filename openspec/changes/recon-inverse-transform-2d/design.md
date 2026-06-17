## Context

The residual path now has all three § 7.15.2 1D inverse transforms and the
§ 7.14.3 residual-addition step. The § 7.15.4.1 2D matrix transform is the
row-then-column orchestration that drives those 1D transforms over a dequantized
coefficient block, producing the `Residual` array. It is a clean self-contained
brick that depends only on the existing 1D primitives.

## Goals / Non-Goals

Goals:

- Implement the § 7.15.4.1 2D matrix transform exactly: the row pass (with the
  √2 rescale) and the column pass, each dispatching to the Walsh-Hadamard
  transform (lossless),
  identity, or kernel 1D transform.
- Keep it total, panic-free, and free of frame/segment/tile state.

Non-Goals:

- The § 7.15.4 outer process (the `Adjusted_Tx_Size` lookup itself, the
  `Transform_Shift` / `get_transform_1d_type` derivations, the `Lossless &&
  IDTX` bit-shift shortcut, the DPCM cumulative sum, and the adjusted-size
  sample duplication), the § 7.14.4 dequantization process, the § 7.15.3
  secondary transform, residual addition, or workspace integration.

## Decisions

- **Take the original `txSz` log2 dimensions; derive the adjusted size.**
  § 7.15.4.1 is invoked with `adjTxSz` and `txSz`, deriving `log2W` / `log2H`
  from `txSz` and `adjLog2W` / `adjLog2H` from `adjTxSz`. The `Adjusted_Tx_Size`
  table caps each dimension's log2 at 5 (the spec also writes this directly as
  `Min(Tx_Width_Log2[txSz], 5)`), so this brick takes the original log2
  dimensions (each 2..=6) and derives the adjusted operating size as
  `1 << Min(log2, 5)`. This keeps the API self-contained and impossible to feed
  inconsistent original/adjusted dimensions, while leaving the
  `Adjusted_Tx_Size` lookup itself to the future § 7.15.4 outer row.
- **Rescale parity and identity scale use the *original* log2 dimensions.** The
  § 7.15.4.1 √2 rescale fires when `Abs(log2W - log2H)` is odd, and the identity
  scale is `get_identity_scale(log2W)` / `get_identity_scale(log2H)` — all from
  the original (unadjusted) dimensions. This is load-bearing for transforms with
  a 64-sample side (e.g. TX_64X32: original log2 (6, 5) is odd, so the rescale
  fires, even though both adjusted dimensions are 32 and would read as even). A
  dedicated regression test pins this against a pre-rescaled 32x32 equivalent.
- **Row/column transform selection is caller-resolved.** `row_type` / `col_type`
  carry the § 7.15.4.1 `rowType` / `colType` (already resolved by the caller via
  `get_transform_1d_type`), and `row_shift` / `col_shift` carry the
  `Transform_Shift[txSz]` values. Lossless forces the Walsh-Hadamard transform
  (row shift 3, column
  shift 0) and requires a 4x4 block.
- **Totality.** The intermediate and per-pass buffers are fixed-size 32x32 /
  32-element stack arrays sized to the maximum adjusted dimension, and the 1D
  primitives are already total; invalid log2 shapes and `w * h` buffer-length
  mismatches return typed `ReconError` before any transform runs.

## Risks / Trade-offs

- Deriving the adjusted size internally folds the one-line `Min(log2, 5)`
  relationship into this brick rather than receiving `adjTxSz` directly. This is
  faithful (the spec states the relationship explicitly) and removes an
  inconsistent-input footgun; the full `Adjusted_Tx_Size` table and the rest of
  the § 7.15.4 outer process remain a separate, later row.

## Migration Plan

Additive; new module, two new `ReconError` variants, and new public exports. No
existing API changes, and the runtime is unaffected.

## Open Questions

None.
