## Context

The current staged coefficient branch stack has reached a base-q handoff that
derives `coeff_cdf_q_ctx` before delegating to the shared `useFsc` selector.
That wrapper still expects callers to provide several values that are frame or
sequence facts rather than transform-block syntax facts:

- `enable_fsc` from AV2 § 5.4.8 / § 6.4.8 sequence transform/quant/entropy
  configuration
  (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-8`,
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-8`);
- `enable_chroma_dctonly` from the same sequence configuration;
- `LosslessArray[segmentId]` from AV2 § 5.18.2
  (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`);
- `reduced_tx_set` from the AV2 § 5.18.2 frame-header tail;
- frame `base_q_idx`, already handled by the lower q-context wrapper.

`splot-core` already parses these facts for the current intra path, and
`splot-decode` already carries `base_q_idx` through the tile payload boundary.
This change adds the next crate-private handoff layer and carries the remaining
parsed facts through tile payload planning for future runtime `coeffs()` wiring.

## Goals / Non-Goals

**Goals:**

- Add a crate-private `DECODE-COEFF-FRAME-FACTS-HANDOFF` wrapper above
  `apply_coeff_use_fsc_branch_from_base_q_facts`.
- Derive nonzero lower inputs from one frame-facts packet:
  `enable_fsc`, `enable_chroma_dctonly`, `reduced_tx_set`,
  `LosslessArray[segmentId]`, and `base_q_idx`.
- Preserve all-zero ordering: all-zero inputs SHALL bypass frame facts and route
  to the existing all-zero branch.
- Propagate parsed frame/sequence facts through crate-private tile payload facts
  and deterministic tile work units.
- Prove equivalence with explicit base-q inputs for ordinary and FSC selected
  branches, and prove parser-fact propagation from `FrameHeaderCore`.

**Non-Goals:**

- Do not implement runtime `coeffs()` wiring or call the new wrapper from the
  minimal block-symbol trace.
- Do not implement the remaining `compute_tx_type` branches, broad block syntax,
  dequantization, inverse transform, residual add, reconstruction, output, or
  reference refresh.
- Do not change public APIs, crate dependencies, encoder behavior, or runtime
  diagnostics.

## Decisions

1. Keep the frame-facts wrapper in `coeff_loop/use_fsc_branch.rs`.

   Rationale: the wrapper composes directly above the existing base-q and
   shared-facts `useFsc` handoffs. Keeping it in the same module keeps the staged
   branch stack visible and avoids a public or cross-crate abstraction.

   Alternative considered: place the helper in `tile_payload/input.rs`. That
   file owns parser-to-tile derivation, but it should not know the shape of the
   coefficient branch stack.

2. Model lossless as `LosslessArray[segmentId]`, not only `CodedLossless`.

   Rationale: AV2 coefficient decode uses the block segment's `Lossless` value,
   and the parser already exposes per-segment lossless facts. The wrapper should
   accept a caller-resolved `segment_id` and derive the boolean from the frame
   array, rejecting out-of-range segment ids before any symbol/CDF mutation.

   Alternative considered: pass `coded_lossless`. That would be enough for the
   current minimal tier but would create another staged caller mismatch for
   segmented streams.

3. Propagate frame facts through tile work units now, without consuming them in
   runtime decode.

   Rationale: the future runtime `coeffs()` caller needs deterministic tile-local
   access to the same facts. Carrying them now is no-output-change and keeps
   parser-fact derivation separate from coefficient symbol reads.

   Alternative considered: test the wrapper only with synthetic frame facts.
   That would prove the wrapper but leave the parser-to-runtime handoff debt
   untouched.

## Risks / Trade-offs

- [Risk] The wrapper could be mistaken for full runtime coefficient support. →
  Mitigation: keep the row partial, keep runtime calls unchanged, and state the
  remaining gaps in matrix, roadmap, OpenSpec, and tests.
- [Risk] Segment-id validation could consume symbols before failing. →
  Mitigation: build the lower input packet before delegating to the base-q
  wrapper, so invalid segment ids fail before CDF or symbol mutation.
- [Risk] Propagating more tile work-unit fields can look like broad tile decode
  support. → Mitigation: keep fields crate-private, keep the minimal tile
  admission gates unchanged, and test only fact propagation/no-output-change.
