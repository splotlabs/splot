## Why

The minimal decoder currently rejects inter blocks that choose compound
references, even when the stream uses the simplest AV2 compound form: two
available references, equal-weight `COMPOUND_AVERAGE`, no masks, no CWP, no
optical-flow refinement, and no residual. The ac0ej3 decoder mission needs this
brick because real multi-frame streams can require the decoder to average two
previously retained references rather than selecting only one.

## What Changes

- Add Feature ID `DECODE-INTER-COMPOUND-AVERAGE` to the implementation matrix and
  decoder-support metadata.
- Extend the minimal inter parser to admit only the fixture-proven
  `reference_select` compound branch for `NumTotalRefs == 2`:
  `comp_mode == 1`, two implicit refs `[0, 1]`, non-joint `NEAR_NEARMV`, no
  neighbour-dependent MV stack, zero MVs, skipped residual, and fixed/equal
  compound average.
- Add the AV2 compound inter CDF rows needed by that branch:
  `comp_mode`, `is_joint`, and `compound_mode_non_joint`.
- Add a compound subpel prediction path that preserves the § 7.13.3.18
  intermediate precision and blends the two refs with the § 7.13.3.16
  equal-weight average formula.
- Commit a three-oracle fixture that fails on the old `reference_select` gate and
  decodes byte-identically to `avmdec` and `dav2d` after this change.
- Preserve verified-subset discipline: any compound branch involving masks,
  CWP, implicit masked blend, optical-flow/refine-MV, joint modes, non-zero MVs,
  residuals, neighbours, scaled refs, or additional refs remains a structured
  `decode/unsupported-feature` rejection before output.

## Capabilities

### New Capabilities
- `decode-inter-compound-average`: Decode the minimal AV2
  two-reference equal-weight compound-average inter subset, proven by a committed
  local-reference fixture and guarded by structured unsupported diagnostics for
  broader compound modes.

### Modified Capabilities
- `decoder-support`: Track `DECODE-INTER-COMPOUND-AVERAGE` as a partial decoder
  runtime row and document the supported/rejected subset.
- `conformance`: Record portable local-reference evidence for the committed
  compound-average fixture.

## Impact

- Affected code: `splot-decode` minimal inter parsing/runtime, tile CDF rows, and
  `splot-recon` subpel motion compensation primitives.
- Affected tests/docs: decoder runtime tests, CLI decode tests, fixture manifest,
  local-reference evidence, decoder support status, implementation matrix, and
  OpenSpec artifacts.
- No public API, dependency graph, encoder, validator, or AV2 broad conformance
  claim changes.
- Non-goals: general `read_compound_ref` for more than two references,
  neighbour-derived compound contexts, non-zero compound MVs, residual compound
  blocks, masked/difference-weighted/inter-intra compound, CWP, implicit masked
  blend, optical-flow refinement, TIP, temporal MV, warped motion, cross-frame
  CDF save/load, and any ac0ej3-wide decode claim beyond the committed
  fixture-proven subset.
