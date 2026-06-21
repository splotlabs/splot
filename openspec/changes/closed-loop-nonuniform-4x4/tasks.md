## 1. Implementation

- [x] 1.1 Add `MinimalClosedLoopReconstruction::reconstruct_luma_4x4` (non-uniform
      entry point via the real 16-coefficient forward DCT).
- [x] 1.2 Refactor the shared pipeline into `prepare` + `finish`; both entry points
      reuse them and differ only in the forward-transform call.
- [x] 1.3 Keep `reconstruct_luma_4x4_dc_only` (flat entry point; still rejects a
      non-uniform residual).
- [x] 1.4 Rename `reconstruct_dc_only_from_quantized` → `reconstruct_from_quantized`
      and `dequantize_dc_only` → `dequantize_block_4x4` (already general).
- [x] 1.5 Fold in the #416 nit: drop a stray `§` from a `quantization.rs` doc.

## 2. Tests

- [x] 2.1 Non-uniform source reconstructs with non-zero AC levels.
- [x] 2.2 Near-lossless reconstruction at qindex 0 (bounded, not bit-exact).
- [x] 2.3 Reconstruction + hash deterministic.
- [x] 2.4 Uniform source matches the flat entry point (samples, levels, hash).
- [x] 2.5 The flat entry point still rejects a non-uniform source (preserved).

## 3. Tracking

- [x] 3.1 Add the `ENC-CLOSED-LOOP-NONUNIFORM-4X4` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
