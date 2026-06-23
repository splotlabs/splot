# minimal-intra-reconstruction-frontier Specification

## Purpose
Document the narrow minimal-tier runtime reconstruction handoff that converts the
already-validated fixture trace into a `splot-recon` current-frame workspace and
freezes it into the existing hash/Y4M output contract without claiming broad
AV2 reconstruction support.
## Requirements

### Requirement: Minimal Flat Intra Reconstruction Handoff

The decoder SHALL reconstruct a `base_q_idx == 255` minimal-tier 64x64 8-bit
YUV420 frozen-trace frame through `splot-recon` current-frame workspace
primitives after the traced partition and block-symbol frontiers pass. This
requirement is limited to the traced luma DC path from AV2 v1.0.0 §5.20.5.5 and
§7.13.2.10 plus the traced top-left chroma `H_PRED` path from §5.20.5.6,
§7.13.2.1, §7.13.2.8, and §9.2. The top-left chroma path SHALL prepare the AV2
no-neighbor left-edge fallback value `(1 << (BitDepth - 1)) + 1` explicitly
before invoking the cardinal H_PRED primitive, but it SHALL NOT claim full edge
preparation or broad chroma directional prediction. The committed
`syn-flat-intra-64x64-minimal.ivf` fixture no longer reaches this frozen handoff:
it was replaced with an AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream
that reconstructs through the general intra path
(`DECODE-GENERAL-INTRA-FRAME-RECON`). The frozen luma-DC / chroma-H_PRED handoff
remains in code and stays covered by its `runtime_minimal_recon` unit tests.

#### Scenario: The committed fixture reconstructs through the general intra path
- **WHEN** the committed minimal IVF fixture is decoded with hash, raw, or Y4M
  output
- **THEN** it routes through the general intra path and reconstructs the luma as a
  flat 128 `all_zero == 1` skip block over a real coded chroma residual,
  byte-identical to the avmdec and dav2d raw output

#### Scenario: The frozen luma-DC / chroma-H_PRED handoff is unit-tested
- **WHEN** the frozen `LumaDcNoResidual8Bit420_64x64` reconstruction handoff is
  exercised by its `runtime_minimal_recon` unit tests
- **THEN** it constructs the decoded frame using `splot-recon` workspace
  operations, luma DC intra prediction, and the explicit traced chroma `H_PRED`
  fallback samples

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
