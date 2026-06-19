## Context

`DECODE-COEFF-FRAME-FACTS-HANDOFF` adds a loaded-but-unwired wrapper that carries
parsed coefficient frame facts to the staged base-q and `useFsc` branch stack.
The next lower ordinary packet still contains two booleans that are not syntax
reads at the lower branch boundary:

- `parityHiding = allow_parity_hiding && !Lossless && plane == 0 &&
  PlaneTxType != IDTX` from AV2 § 5.20.7.27
  (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`);
- `useTcq = allow_tcq && plane == 0 && !Lossless &&
  txClass == TX_CLASS_2D && !useFsc` from the same section.

`splot-core` already parses `allow_tcq` and `allow_parity_hiding` in the
§ 5.18.2 lossless tail. `splot-decode` already has the remaining staged block
facts needed for the formulas: plane geometry, `PlaneTxType`, lossless from
`LosslessArray[segmentId]`, and `useFsc`.

## Goals / Non-Goals

**Goals:**

- Add a crate-private `DECODE-COEFF-PARITY-TCQ-HANDOFF` derivation inside the
  existing frame-facts wrapper path.
- Propagate parsed frame `allow_tcq` and `allow_parity_hiding` through
  `FrameCandidateTileFacts`, `TileFrameFacts`, and `DecodeTileWorkUnit`.
- Derive ordinary lower-branch `parity_hiding` and `use_tcq` before base-q
  delegation, using the exact AV2 § 5.20.7.27 conditions.
- Keep all-zero inputs independent of frame/ordinary facts.
- Prove equivalence with explicit lower packets and parser-fact propagation.

**Non-Goals:**

- Do not call the wrapper from runtime `coeffs()` yet.
- Do not implement additional `compute_tx_type` branches, segment-map
  derivation, block-mode traversal, tile context writes, dequantization, inverse
  transform, residual add, reconstruction, output, or reference refresh.
- Do not change public APIs, crate dependencies, encoder behavior, or runtime
  diagnostics.

## Decisions

1. Extend `TileCoeffFrameFacts` rather than adding a parallel packet.

   Rationale: `allow_tcq` and `allow_parity_hiding` are frame-header lossless-tail
   facts used by the same coefficient frame-facts wrapper that already carries
   `LosslessArray[]`, `base_q_idx`, and sequence transform flags.

   Alternative considered: keep the flags only in the wrapper test inputs. That
   would prove the formula but leave future runtime `coeffs()` callers without
   deterministic tile-local access to parsed frame flags.

2. Derive `parity_hiding` and `use_tcq` inside lower-input construction.

   Rationale: the wrapper already validates `segment_id`, computes `Lossless`,
   and derives `useFsc` before constructing the base-q packet. The § 5.20.7.27
   formulas depend on those values, so deriving the booleans there preserves
   fail-atomic behavior and avoids duplicating the formulas at call sites.

   Alternative considered: add a separate wrapper above frame facts. That would
   create another packet layer without adding a new runtime boundary.

3. Keep `is_inter` caller-resolved.

   Rationale: § 5.20.7.27 `useFsc` depends on the block-mode `is_inter` fact
   stored by `decode_block()`, not simply on frame type. Deriving it from
   `FrameType` would be wrong for intra blocks in inter frames.

## Risks / Trade-offs

- [Risk] A future caller could assume `allow_tcq` itself is the lower branch's
  `useTcq`. -> Mitigation: keep the field names distinct and test suppression by
  chroma, lossless, transform class, and `useFsc`.
- [Risk] The new formulas could consume symbols before rejecting an invalid
  segment id. -> Mitigation: continue deriving the lower packet before
  delegation and keep the existing invalid-segment fail-atomic test.
- [Risk] The handoff could be mistaken for runtime coefficient support. ->
  Mitigation: keep support rows partial and state that runtime `coeffs()`,
  block traversal, reconstruction, and output remain unsupported.
