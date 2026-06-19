## 1. coeff_base_lf LF luma context

- [x] 1.1 Add `pub(crate)` `coeff_base_lf_luma_context(pos, bwl, txw, txh, tx_class, c, level)` in `coefficient_tokenization`, importing `splot_core::tables::conversion::SIG_REF_DIFF_OFFSET`.
- [x] 1.2 Mirror the decoder's `CoeffBaseContext` LF luma branch: neighbour-sum with `magLimit` (5 for LF near-DC, else 3), `ctx = (mag+1)>>1`, then the §8.3.2 LF luma mapping (2D: `c==0`→`ctx.min(8)`; `row+col<2`→`ctx.min(6)+9`; else→`ctx.min(4)+16`; horiz/vert: `21 + ...`).
- [x] 1.3 Keep it total and panic-free (saturating geometry, slice-bounds-guarded reads contribute 0); scope to low-frequency luma (parity-hidden DC and chroma out of scope, documented).

## 2. Tests

- [x] 2.1 Prove the DC (pos 0) context for a single AC neighbour of level 1 at pos 1 is `1` (mag 1 → ctx 1 → LF c==0 `ctx.min(8)`), the exact case the eob=2 trace brick will use.
- [x] 2.2 Prove the neighbour-sum clamping (`magLimit` 5 for LF near-DC) and the three 2D LF context bands (`c==0`, `row+col<2`, else) with representative `Level[]` inputs.
- [x] 2.3 Prove out-of-bounds / short-slice neighbours contribute 0 and the function does not panic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-COEFF-BASE-LF-CONTEXT` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming token emission, multi-coefficient trace, chroma/parity-hidden contexts, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
