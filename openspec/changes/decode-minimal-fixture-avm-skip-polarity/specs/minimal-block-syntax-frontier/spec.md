## MODIFIED Requirements

### Requirement: Minimal Flat Intra Block Symbol Trace Frontier

The decoder SHALL provide a crate-private tile-payload trace frontier that
consumes only the supported minimal-tier flat intra block symbols after the
partition frontier using AV2 v1.0.0 §5.20.4.1, §5.20.5.1, §5.20.5.3, §5.20.5.5,
§5.20.5.6, §5.20.6.1, §5.20.6.2, §5.20.7.23, §5.20.7.24, §5.20.7.27, §8.2.4,
§8.2.6, §8.3.1, §8.3.2, and generated §9.3 CDF defaults. The luma and V
`txb_skip` reads SHALL assert the AVM `all_zero == 1` skip polarity
(§5.20.7.27 / AVM `decodetxb.c`). This frozen trace is reachable only by a
`base_q_idx == 255` frame; the committed `syn-flat-intra-64x64-minimal.ivf`
fixture is no longer such a frame — it was replaced with an AVM/dav2d-conformant
`base_q_idx` 210 luma-skip stream that routes through the general intra path
(`DECODE-GENERAL-INTRA-FRAME-RECON`) — so no committed conformant fixture
exercises this frozen trace's happy path.

#### Scenario: The frozen trace rejects the retired inverted-skip payload
- **WHEN** the frozen minimal block-symbol trace consumes the retired pre-AVM
  payload whose luma `txb_skip` was coded with inverted polarity (`all_zero == 0`)
- **THEN** it fails closed with a typed symbol mismatch (expected 1, decoded 0)
  and rolls back the tile CDF mutations

#### Scenario: Block symbol mismatch fails closed
- **WHEN** a tile payload mutation changes one of the traced flat block symbols
- **THEN** the minimal runtime reports `decode/unsupported-feature` with a stable
  minimal block-symbol reason and does not construct output

#### Scenario: The committed fixture decodes through the general intra path
- **WHEN** the committed `syn-flat-intra-64x64-minimal.ivf` fixture is decoded
  through hash, raw, or Y4M output
- **THEN** it routes through the general intra path rather than this frozen trace
  and produces the AVM/dav2d-conformant output (luma flat 128 `all_zero == 1`
  skip over a real coded chroma residual)

#### Scenario: Broad decode tile remains out of scope
- **WHEN** a conforming stream requires syntax outside the traced flat
  block-symbol subset
- **THEN** the decoder keeps failing closed with `decode/unsupported-feature`
  rather than claiming broad `decode_block()`, `decode_tile()`, or reconstruction
  support
