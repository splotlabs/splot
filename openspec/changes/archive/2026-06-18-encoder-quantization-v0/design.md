## Context

`ENC-RESIDUAL-FOUNDATION` and `ENC-FORWARD-TRANSFORM-FOUNDATION` have landed as
private, non-emitting `splot-encode` arithmetic stages. The encoder still lacks
a checked quantization step that produces quantized coefficients and proves the
decoder-visible dequant/inverse handoff. `splot-recon` already owns the AV2
decoder-visible dequantization functions and block dequantizer for
`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-14-2` and
`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-14-4`, plus the inverse
transform path for `#s-7-15-4`.

This change stays on the current private 4x4 DCT_DCT DC-only encoder subset. It
does not emit the quantized coefficient syntax described by
`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28`; that is left to
`encoder-coefficient-tokenization-minimal`.

## Goals / Non-Goals

**Goals:**

- Add a private `quantization` module for `ENC-QUANTIZATION-V0`.
- Accept the current 4x4 DCT_DCT DC-only transform block and a fixed quantizer
  index.
- Validate qindex against `splot-recon::max_quantizer_index`, reject zero
  dequant denominators, and reject coefficients outside the decoder-visible
  dequant range before quantization.
- Use deterministic round-to-nearest integer quantization in wide arithmetic.
- Feed quantized coefficients through `splot-recon::dequantize_block` and prove
  the dequantized coefficients can reconstruct through the existing inverse
  transform path.
- Preserve the no-packet invariant in `Context`.

**Non-Goals:**

- No quantization matrix or user-QM support.
- No delta-Q, segmentation, lossless mode, transform selection, tokenization,
  entropy/range bytes, tile body, packet output, CLI success path, rate control,
  quality claim, or Baseline Encoder Profile v1 claim.
- No dependency graph, public API, or CLI change.
- No AVM/dav2d evidence; this helper is not stream-emitting.

## Decisions

1. **Keep quantization private and loaded-but-unwired.**
   The module is `mod quantization;` and all types/functions are `pub(crate)`.
   This matches the residual and forward-transform foundations while avoiding a
   public contract before coefficient syntax exists.

2. **Use qindex + zero deltas for v0 policy.**
   `FixedQuantizationParams::new(bit_depth, qindex)` resolves DC/AC quantizers
   through `splot-recon::dc_quantizer` / `ac_quantizer` with zero deltas. This
   keeps the decoder-visible quantizer functions in `splot-recon` as the source
   of truth and avoids frame-header/delta-Q state modeling in this slice.

3. **Reject unsupported dequant-visible ranges instead of relying on clipping.**
   The encoder rejects coefficients outside the AV2 dequant clip range for the
   selected bit depth. That makes the v0 proof about preserving intended
   coefficients through dequant/inverse, not about accepting clipped arithmetic.

4. **Use deterministic half-up magnitude rounding.**
   For a coefficient `c`, quantizer `q`, and denominator `d`, v0 computes
   `round(abs(c) * d * 8 / q)` and reapplies the sign. The `8` is the inverse of
   the dequant process's `QUANT_TABLE_BITS = 3` divide. All products use widened
   arithmetic and are checked before narrowing.

5. **Reject dequant-product wrap risk.**
   `splot-recon::dequant_coefficient` follows AV2 § 7.14.4 and masks the
   product to 24 bits. The encoder v0 path rejects a quantized coefficient whose
   product with the selected quantizer would exceed that 24-bit domain, so the
   proof does not depend on wraparound.

## Flight Manifest

- Change ID: `encoder-quantization-v0`
- Feature IDs: `ENC-QUANTIZATION-V0`
- Base commit: `78d11b66dfe2ec9f75f7c45b46ee833f8efa9532`
- Depends on merged changes: `encoder-residual-foundation`,
  `encoder-forward-transform-foundation`, `encoder-recon-dependency`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/quantization.rs`
  - `crates/splot-encode/src/error.rs`
  - `crates/splot-encode/src/lib.rs`
  - `crates/splot-encode/src/context.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-quantization-v0/**`
  - `openspec/changes/archive/2026-06-18-encoder-quantization-v0/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - `crates/splot-core/**`
  - `crates/splot-decode/**`
  - `crates/splot-recon/**`
  - `crates/splot-validate/**`
  - `crates/splot-cli/**`
  - workspace manifests and `Cargo.lock`
  - AV2 spec mirror files under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-QUANTIZATION-V0`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none
- Changed-file intersection with each sibling PR: none
- Semantic overlap with each sibling PR: none
- Can build/test/merge directly onto main without another open PR: yes

## Risks / Trade-offs

- [Risk] The quantization policy is deliberately tiny and does not represent a
  usable encoder quality mode. -> Mitigation: keep it private, document
  non-goals, and require later tokenization/tile-body milestones before output
  claims.
- [Risk] Round-to-nearest policy may not be final. -> Mitigation: isolate it as
  encoder policy with tests and no public API commitment.
- [Risk] qindex zero exactness can look like a lossless claim. -> Mitigation:
  state it is a helper proof for the DC-only subset, not a public lossless mode.
