## Context

The current minimal inter runtime can retain two decoded references and select a
single reference through AV2 § 5.20.7.12 `read_single_ref`, but it still rejects
`reference_select` compound blocks before output. The next decoder mission brick
is the smallest compound subset that can be fixture-proven against `avmdec` and
`dav2d`: two references, no neighbour-derived MV stack, non-joint
`NEAR_NEARMV`, zero MVs, no residual, no CWP or masks, and equal-weight
`COMPOUND_AVERAGE`.

The relevant AV2 mirror sections are:

- `docs/spec/av2/1.0.0/05-syntax-structures.md`: § 5.20.7.10
  `inter_block_mode_info`, § 5.20.7.11 `read_ref_frames`, § 5.20.7.13
  compound mode reads, and § 5.20.7.18 `assign_mv`.
- `docs/spec/av2/1.0.0/07-decoding-process.md`: § 7.13.3.16 inter prediction
  setup and compound blending, and § 7.13.3.18 inter prediction samples.
- `docs/spec/av2/1.0.0/08-parsing-process.md`: § 8.3.2 CDF selection for
  `comp_mode`, `is_joint`, and `compound_mode_non_joint`.

## Goals / Non-Goals

**Goals:**

- Decode one committed, three-oracle fixture that exercises a real
  `reference_select` compound block and fails on the old gate.
- Preserve AV2 compound subpel intermediate precision, then apply the
  equal-weight average blend derived from the spec constants.
- Keep all unsupported compound tools rejected through structured
  `decode/unsupported-feature` diagnostics before any output is written.
- Keep dependency direction unchanged: `splot-decode` drives runtime behavior and
  `splot-recon` owns the lower-level motion-compensation primitive.

**Non-Goals:**

- More than two references, neighbour-derived compound contexts, non-zero
  compound MVs, residual compound blocks, masks, CWP, implicit masked blend,
  optical-flow/refine-MV, TIP, temporal MV, warped motion, cross-frame CDF
  save/load, or broader local decoder mission decode.

## Decisions

1. Gate compound parsing to the fixture-proven shape.

   The runtime will only leave the old `reference_select` rejection when
   `NumTotalRefs == 2`, the block has no neighbour context, and the sequence/frame
   feature flags disable the compound tools that alter blend or MV semantics.
   This keeps the parser honest: every broader branch is still parsed only far
   enough to identify a stable unsupported reason before output.

2. Add a dedicated compound prediction primitive instead of reusing the clipped
   single-reference path.

   AV2 § 7.13.3.18 stores compound `Preds[refList]` after `Round2(...,
   InterRound1)` and before `Clip1`. Reusing the existing clipped subpel helper
   would lose precision and could produce confident-wrong pixels. The new helper
   returns i32 intermediates, and the equal-weight blend applies
   `Clip1(Round2(P0 + P1, 5))`, with `5` derived from
   `4 + InterPostRound`, not hard-coded as a free constant.

3. Keep compound parser helpers private and local to minimal inter decode.

   The branch is not a general `read_compound_ref` implementation. A private
   helper keeps the narrow assumptions visible and prevents accidental reuse by
   future broader compound work.

4. Treat the fixture as the acceptance oracle.

   The committed stream must decode byte-identically to local `avmdec` and
   `dav2d`, and the same raw output must match `splot decode`. The fixture must
   fail on the pre-change `reference_select` gate to prove the new path is
   exercised.

## Risks / Trade-offs

- [Risk] A fixture accidentally remains single-reference and passes without
  exercising compound. → Mitigation: require the old decoder to reject it at the
  `reference_select` compound gate and add a regression assertion for the
  compound branch.
- [Risk] A future compound stream enters an unproven path after partial parsing.
  → Mitigation: add explicit unsupported checks for masks, CWP, implicit masked
  blend, OPFL/refine-MV, joint modes, neighbours, non-zero MVs, residuals, and
  more than two refs.
- [Risk] Rounding mistakes in compound MC can produce plausible but wrong pixels.
  → Mitigation: expose focused `splot-recon` unit tests for the intermediate
  helper and pin full-frame raw equality against both reference decoders.
