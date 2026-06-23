# decode-inter-compound-average Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `decode-inter-compound-average`.

## Requirements
### Requirement: Minimal compound-average inter decode
`splot decode` SHALL decode the AV2 two-reference equal-weight compound-average
inter subset tracked by Feature ID `DECODE-INTER-COMPOUND-AVERAGE` when all of
the following are true: `NumTotalRefs == 2`, `reference_select` chooses compound,
the two implicit references are `[0, 1]`, the block has no neighbour-dependent
compound MV stack, compound mode is non-joint `NEAR_NEARMV`, both MVs resolve to
zero, residual is skipped, reference scaling is identity, and CWP, masked
compound, implicit masked blend, optical-flow refinement, refine-MV, TIP,
warped motion, and temporal MV features are disabled.

#### Scenario: compound fixture decodes
- **WHEN** `splot decode` runs on the committed three-frame compound-average
  fixture with raw output enabled
- **THEN** decoding succeeds without `decode/unsupported-feature`
- **AND** the produced raw output matches the recorded `avmdec` and `dav2d`
  local-reference digest for that fixture

#### Scenario: old gate is exercised
- **WHEN** the committed compound-average fixture is checked against the
  pre-change decoder behavior
- **THEN** it reaches the `reference_select` compound branch that was previously
  rejected

### Requirement: Compound prediction uses AV2 intermediate precision
The compound motion-compensation implementation SHALL keep each reference
prediction as an unclipped signed intermediate after the § 7.13.3.18 compound
`InterRound1` step and SHALL apply the equal-weight `COMPOUND_AVERAGE` blend from
§ 7.13.3.16 as `Clip1(Round2(P0 + P1, 5))`, deriving the final shift from
`InterPostRound` rather than treating it as an unrelated magic constant.

#### Scenario: intermediate helper is not clipped early
- **WHEN** the compound subpel helper is unit-tested with samples whose
  intermediate value can exceed the visible output range
- **THEN** it returns the signed intermediate value before final blending
- **AND** only the equal-weight compound blend clips to the output bit depth

### Requirement: Broader compound tools stay unsupported
The decoder SHALL reject compound inter blocks outside the
`DECODE-INTER-COMPOUND-AVERAGE` subset before producing output, using the
structured `decode/unsupported-feature` diagnostic with a stable matrix row and
Feature ID.

#### Scenario: unsupported compound branch rejects before output
- **WHEN** a decoded stream reaches compound CWP, masked compound, implicit masked
  blend, optical-flow/refine-MV, TIP, joint compound mode, neighbour-dependent
  compound MV derivation, non-zero compound MV, compound residual, scaled
  reference, or more than two total references
- **THEN** `splot decode` exits with `decode/unsupported-feature`
- **AND** no raw, Y4M, or hash output is emitted for that stream
