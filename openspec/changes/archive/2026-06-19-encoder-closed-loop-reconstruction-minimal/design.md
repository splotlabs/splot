## Context

`ENC-RESIDUAL-FOUNDATION`, `ENC-FORWARD-TRANSFORM-FOUNDATION`,
`ENC-QUANTIZATION-V0`, and `ENC-COEFFICIENT-TOKENIZATION-MINIMAL` have landed as
private, non-emitting `splot-encode` stages. Each proves a single arithmetic
step, and the quantization and forward-transform foundations already prove an
isolated dequant/inverse handoff through `splot-recon`. What does not yet exist
is a single composed closed loop that takes a borrowed input block, predicts it,
forms and quantizes a residual, and reconstructs the decoder-visible samples
through `splot-recon`, then freezes them into a current-frame workspace and
hashes them.

This is the core encoder correctness invariant from the mission's closed-loop
reference invariant: before any reconstructed frame may be used as a reference
(or published), the stored reconstruction must be exactly what a conforming
decoder produces from the emitted bitstream. This change builds and evidences
that loop for the smallest legal subset, still without emitting any packet.

Normative AV2 v1.0.0 sections used (all in
`docs/spec/av2/1.0.0/07-decoding-process.md`):

- §7.13.2.10 DC intra prediction process (`#s-7-13-2-10`) — decoder-visible,
  via `splot-recon`.
- §7.14.2 dequantization functions (`#s-7-14-2`) — decoder-visible, via
  `splot-recon`.
- §7.14.3 reconstruct process / residual addition (`#s-7-14-3`) —
  decoder-visible, via `splot-recon`.
- §7.14.4 dequantization process (`#s-7-14-4`) — decoder-visible, via
  `splot-recon`.
- §7.15.4 2D inverse transform process (`#s-7-15-4`) — decoder-visible, via
  `splot-recon`.

The encoder-policy stages (residual subtraction, forward transform,
quantization) are original encoder math and are not described as spec-exact.

## Goals / Non-Goals

**Goals:**

- Add a private `closed_loop` module for
  `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL`.
- Reconstruct the current top-left 8-bit luma 4x4 DCT_DCT DC-only uniform-block
  subset end to end through DC prediction, residual, forward transform,
  quantization, dequantization, inverse transform, and residual addition.
- Use `splot-recon` for every decoder-visible step, including the current-frame
  workspace freeze and decoded-frame hash; keep encoder-policy stages in
  `splot-encode`.
- Prove the qindex-zero flat subset is lossless (reconstruction equals source),
  reconstruction and hash are deterministic, and the emitted tokenization
  decisions decode back to the exact quantized coefficient the loop
  reconstructs from.
- Preserve the no-packet invariant in `Context`.

**Non-Goals:**

- No public API, CLI success path, tile payload writer, packet output, or
  Baseline Encoder Profile v1 claim.
- No reference-frame-store insertion: storing a reconstructed frame as a
  reference is only meaningful for inter prediction and is deferred to the
  reference-state / inter phase with its own Feature ID.
- No chroma, inter, multi-block, non-4x4, non-DC, non-uniform, FSC, IDTX, TCQ,
  or 10-bit reconstruction; no broad intra mode set beyond no-neighbor DC.
- No transform/mode/quantizer search and no rate control.
- No dependency graph change and no AVM/dav2d evidence; this helper emits no
  stream, so external decode comparison arrives with the first packet-producing
  milestone.

## Decisions

1. **Keep the closed loop private and loaded-but-unwired.**
   The module is loaded from `splot-encode` and exposes only crate-private
   types and helpers, matching the residual/transform/quantization/tokenization
   foundations. It does not change `Context::receive_packet`.

2. **`splot-recon` owns all decoder-visible math.**
   Prediction (`predict_intra_dc_square_into`), dequantization
   (`dequantize_block`), inverse transform (`inverse_transform_2d_outer`),
   residual addition (`reconstruct_add_residual`), the current-frame workspace
   (`CurrentFrameWorkspace`), and the decoded-frame hash
   (`DecodedFrameHashInput`) are all called from `splot-recon`. The encoder
   never reimplements these. Only the residual subtraction, forward transform,
   and quantization policy live in `splot-encode`.

3. **No-neighbor DC is the correct first-block prediction.**
   The top-left block of a frame has no decoded neighbors, so AV2 §7.13.2.10 DC
   prediction yields the midpoint `1 << (BitDepth - 1)` (128 for 8-bit). The
   module uses `IntraDcEdges::none()`, which is the genuine decoder behavior for
   that block rather than a placeholder.

4. **Uniform 4x4 source only, to match the existing forward transform.**
   The forward-transform foundation accepts only a uniform residual block. With
   a uniform DC prediction, that requires a uniform (flat) source block. The
   module reconstructs a 4x4 monochrome frame from a 4x4 borrowed source view;
   non-uniform sources are rejected by the existing forward-transform error.

5. **Current-frame workspace + decoded-frame hash are part of the loop.**
   The reconstructed block is written into a `splot-recon` monochrome 4x4
   `CurrentFrameWorkspace`, frozen into a `DecodedFrame`, and hashed with
   `DecodedFrameHashInput`. The hash is the deterministic artifact external
   decoders will be compared against at later milestones, so it is produced and
   tested now even though no packet is emitted.

6. **Emitted-decision equivalence is proven through the existing tokenizer.**
   The independent equivalence test tokenizes the same quantized block with
   `tokenize_quantized_4x4_dct_dct_dc_only`, roundtrips the token records through
   the in-tree AV2 §8.2 symbol encoder/decoder, recovers the quantized DC
   coefficient from the decoded symbols, and asserts it equals the quantized DC
   coefficient the closed loop reconstructed from. For the qindex-zero flat
   subset it also asserts the reconstruction equals the source, so "what we
   would emit" provably reconstructs to "what we reconstructed".

## Flight Manifest

- Change ID: `encoder-closed-loop-reconstruction-minimal`
- Feature IDs: `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL`
- Base commit: `14dadb502760efe6896e6fb279b162a29fa1ce0e`
- Depends on merged changes: `encoder-residual-foundation`,
  `encoder-forward-transform-foundation`, `encoder-quantization-v0`,
  `encoder-coefficient-tokenization-minimal`, `encoder-recon-dependency`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/closed_loop.rs`
  - `crates/splot-encode/src/error.rs`
  - `crates/splot-encode/src/lib.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-closed-loop-reconstruction-minimal/**`
  - `openspec/changes/archive/2026-06-19-encoder-closed-loop-reconstruction-minimal/**`
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
- Matrix rows owned: `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit
  `14dadb502760efe6896e6fb279b162a29fa1ce0e`.
- Changed-file intersection with each sibling PR: none (no sibling PR open). If
  a decoder-mission PR opens and lands first, the only expected overlap is the
  generated/tracking docs (`docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`); rebase and regenerate
  before merge.
- Semantic overlap with each sibling PR: none; this change is private encoder
  reconstruction and does not depend on decoder code.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] A small monochrome 4x4 closed loop could be mistaken for a usable
  encoder frame path. -> Mitigation: keep it private, preserve
  `receive_packet` behavior, and make the matrix/docs explicitly exclude packet
  output, chroma, multi-block frames, references, and Baseline Encoder Profile
  v1.
- [Risk] The lossless qindex-zero assertion is narrower than a full
  rate/distortion proof. -> Mitigation: frame it as exact decoder-visible
  reconstruction for the declared subset, not a quality claim; lossy qindex
  reconstruction is still proven deterministic and emitted-decision-equivalent.
- [Risk] Deferring reference-store insertion leaves part of the mission's
  closed-loop bullet for a later change. -> Mitigation: reference storage is
  only exercised by inter prediction, which does not exist yet; the gap audit
  records the deferral so it is not silently dropped.
