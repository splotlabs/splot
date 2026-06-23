## Context

> **Historical note.** This change predates `decode-minimal-fixture-avm-skip-polarity`, which retired the frozen `base_q_idx == 255` committed minimal fixture (`syn-flat-intra-64x64-minimal.ivf`) and replaced it with an AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream that routes through the general intra path. References below to the committed minimal fixture as the frozen `base_q_idx == 255` anchor — and to keeping that committed fixture's hash byte-identical — are historical; the routing rule (a `base_q_idx == 255` frame falls through to the frozen gate) still holds.

The general intra frame path reaches the AV2 § 5.20.3.1 single-block partition
frontier and then returns `general_intra_block_decode_unimplemented`. The frozen
minimal-tier trace (`block_symbol.rs::consume_trace`) decodes mode symbols but
asserts their values and interleaves a `luma_all_zero` symbol between
`y_mode_index` and `uv_mode` — a hand-crafted order for the synthetic frozen
fixture, not the § 5.20.5.3 `intra_frame_mode_info` order. The general path must
follow spec order without assertions.

## Goals / Non-Goals

**Goals:**
- Decode the § 5.20.5.3 mode symbols in spec order for the minimal-tool intra
  block: `read_intra_y_mode` then `read_intra_uv_mode`.
- Reconstruct the typed non-directional luma `YMode` and derive the § 8.3.2
  `uv_mode` context from it.
- Advance the general intra rejection from the partition frontier to the
  residual step.
- Keep the frozen `base_q_idx == 255` minimal hash contract byte-identical.

**Non-Goals:**
- No typed `UVMode` reconstruction (`get_intra_uv_mode_set`) yet.
- No residual / transform-block syntax, coefficient decode, dequantization,
  inverse transform, residual add, reconstruction, or output.
- No directional / escape / second-mode `YMode` reconstruction.
- No in-repo AVM/dav2d dependency.

## Decisions

1. New module `general_intra_block.rs`, separate from the frozen
   `block_symbol.rs` trace.

   Rationale: the frozen trace asserts values and uses a non-spec symbol order;
   reusing it would entangle the synthetic-fixture contract with the general
   path. The new module reuses the § 8.3.2 context helpers and the typed
   `YMode` reconstruction from `cdf::block_context` but reads symbols in spec
   order without assertions.

2. Decode (consume) `uv_mode` but defer typed `UVMode` reconstruction.

   Rationale: consuming the `uv_mode` symbol (and its `uv_mode_idx` escape) is
   required to position the bitstream for the residual step; the typed `UVMode`
   is only needed by chroma prediction, a later brick. The decoded `uv_mode`
   index is recorded for that future consumer.

3. Rely on the disabled-tool subset to bound the symbol set.

   Rationale: with intra block copy, segmentation, GDF, CDEF, CCSO, delta-Q,
   lossless DPCM, palette, DIP, FSC, MRL, CfL, and MHCCP disabled, the only
   § 5.20.5.3 mode symbols are `y_mode_set`, `y_mode_index`, and `uv_mode`
   (plus the escape). The committed q80 fixture is encoded with exactly this
   tool set.

## Risks / Trade-offs

- [Risk] Wrong symbol order or context would decode garbage modes that only
  surface at the eventual frame-output comparison.
  -> Mitigation: the order and contexts are transcribed from the § 5.20.5.3 and
  § 8.3.2 spec mirror; a unit test pins `y_mode == DC_PRED` for the synthetic
  payload, and the CLI test proves the q80 fixture decodes its modes without a
  read error and reaches the residual step.
- [Risk] The decoded `uv_mode` is recorded but not yet consumed.
  -> Mitigation: it advances the bitstream for the residual step and is unit
  tested; the typed `UVMode` consumer lands with chroma prediction.
