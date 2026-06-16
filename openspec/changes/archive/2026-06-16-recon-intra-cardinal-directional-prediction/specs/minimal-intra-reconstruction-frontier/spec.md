## MODIFIED Requirements

### Requirement: Minimal Flat Intra Reconstruction Handoff
The decoder SHALL reconstruct the already-supported
`minimal-intra-8bit420-hash-v1` 64x64 8-bit YUV420 fixture through
`splot-recon` current-frame workspace primitives after the traced partition and
block-symbol frontiers pass. This requirement is limited to the traced luma DC
path from AV2 v1.0.0 §5.20.5.5 and §7.13.2.10 plus the traced top-left chroma
`H_PRED` path from §5.20.5.6, §7.13.2.1, §7.13.2.8, and §9.2. The top-left
chroma path SHALL prepare the AV2 no-neighbor left-edge fallback value
`(1 << (BitDepth - 1)) + 1` explicitly before invoking the cardinal H_PRED
primitive, but it SHALL NOT claim full edge preparation or broad chroma
directional prediction. The existing no-residual, no-filter, no-grain minimal
tier guards SHALL still be enforced before output construction.

#### Scenario: Minimal fixture reconstructs through the workspace
- **WHEN** the committed minimal IVF fixture passes the current minimal plan,
  tile, partition, block-symbol, and `exit_symbol()` checks
- **THEN** the runtime constructs the decoded frame by using `splot-recon`
  workspace operations, luma DC intra prediction, explicit traced chroma
  `H_PRED` handling, and freezing the workspace into a `DecodedFrame`

#### Scenario: Hash output records spec-correct chroma H_PRED samples
- **WHEN** the committed minimal fixture is decoded with hash output
- **THEN** the decoded-frame hash report records the deterministic
  `minimal-intra-8bit420-hash-v1` raw-intermediate output with luma DC samples
  and top-left chroma H_PRED fallback samples

#### Scenario: Y4M output records spec-correct chroma H_PRED samples
- **WHEN** the committed minimal fixture is decoded with Y4M output
- **THEN** the emitted Y4M stream contains luma DC samples and top-left chroma
  H_PRED fallback samples without using the old neutral-chroma fallback

#### Scenario: Out-of-tier streams fail before reconstruction output
- **WHEN** a stream fails any existing minimal tier guard, traced symbol check,
  resource limit, malformed source check, or output serialization check
- **THEN** the decoder reports the existing structured diagnostic and does not
  publish reconstructed output for that failed stream

#### Scenario: Broad reconstruction remains unsupported
- **WHEN** a conforming stream requires syntax outside the traced 64x64 flat DC
  luma, top-left fallback `H_PRED` chroma, all-zero-transform, no-filter
  minimal subset
- **THEN** the decoder keeps reporting `decode/unsupported-feature` rather than
  claiming broad `decode_block()`, `decode_tile()`, full intra reconstruction,
  general directional prediction, residual/transform reconstruction, loop
  filtering, reference refresh, film grain, raw output, or complete decoder
  conformance
