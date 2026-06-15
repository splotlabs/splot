## ADDED Requirements

### Requirement: Minimal Flat Intra Reconstruction Handoff
The decoder SHALL reconstruct the already-supported
`minimal-intra-8bit420-hash-v1` 64x64 8-bit YUV420 fixture through
`splot-recon` current-frame workspace primitives after the traced partition and
block-symbol frontiers pass. This requirement is limited to the traced luma DC
path from AV2 v1.0.0 §5.20.5.5 and §7.13.2.10 plus neutral chroma workspace
materialization that preserves the existing minimal output contract. The traced
§5.20.5.6 chroma mode is not claimed as fully reconstructed until horizontal /
vertical chroma prediction support lands. The existing no-residual, no-filter,
no-grain minimal tier guards SHALL still be enforced before output construction.

#### Scenario: Minimal fixture reconstructs through the workspace
- **WHEN** the committed minimal IVF fixture passes the current minimal plan,
  tile, partition, block-symbol, and `exit_symbol()` checks
- **THEN** the runtime constructs the decoded frame by using `splot-recon`
  workspace operations, luma DC intra prediction, neutral chroma workspace
  writes, and freezing the workspace into a `DecodedFrame`

#### Scenario: Hash output identity is preserved
- **WHEN** the committed minimal fixture is decoded with hash output
- **THEN** the decoded-frame hash report remains byte-for-byte compatible with
  the existing `minimal-intra-8bit420-hash-v1` output contract

#### Scenario: Y4M output identity is preserved
- **WHEN** the committed minimal fixture is decoded with Y4M output
- **THEN** the emitted Y4M stream remains byte-identical to the existing
  supported minimal runtime output

#### Scenario: Out-of-tier streams fail before reconstruction output
- **WHEN** a stream fails any existing minimal tier guard, traced symbol check,
  resource limit, malformed source check, or output serialization check
- **THEN** the decoder reports the existing structured diagnostic and does not
  publish reconstructed output for that failed stream

#### Scenario: Broad reconstruction remains unsupported
- **WHEN** a conforming stream requires syntax outside the traced 64x64 flat DC
  luma, neutral-chroma, all-zero-transform, no-filter minimal subset
- **THEN** the decoder keeps reporting `decode/unsupported-feature` rather than
  claiming broad `decode_block()`, `decode_tile()`, full intra reconstruction,
  chroma H/V prediction, residual/transform reconstruction, loop filtering,
  reference refresh, film grain, raw output, or complete decoder conformance
