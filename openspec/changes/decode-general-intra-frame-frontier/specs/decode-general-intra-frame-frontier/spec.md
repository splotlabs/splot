## ADDED Requirements

### Requirement: General intra frame decode frontier
The decoder SHALL provide a crate-private general intra decode frontier that
accepts a single-tile 64x64 8-bit 4:2:0 intra key frame whose `base_q_idx`
differs from the frozen minimal-tier fixture value 255 and whose segmentation,
quant matrices, delta-Q, in-loop filters, CCSO, GDF, and film grain are all
disabled, runs the real AV2 § 5.20.3.1 root partition traversal to the
single-block frontier, and then returns a structured
`decode/unsupported-feature` diagnostic because block-symbol, coefficient, and
reconstruction decode are not yet implemented. The frontier SHALL NOT decode
intra modes, read coefficient symbols, write `Quant`, dequantize, inverse
transform, add residuals, reconstruct pixels, produce output, refresh
references, invoke AVM or dav2d, expose a public API, or mutate the frozen
`base_q_idx == 255` minimal hash contract.

#### Scenario: General intra fixture reaches the partition frontier
- **WHEN** `splot decode` is given the committed minimal-tool intra key frame
  `syn-flat-intra-64x64-q80.ivf`
- **THEN** the general intra frontier runs the AV2 § 5.20.3.1 root partition
  traversal to the single-block frontier
- **AND** it emits a `decode/unsupported-feature` diagnostic with feature id
  `DECODE-GENERAL-INTRA-FRAME-FRONTIER` and reason
  `general_intra_block_decode_unimplemented`

#### Scenario: base_q_idx == 255 frames route to the frozen tier, not the general path
- **WHEN** `splot decode` is given an intra key frame with `base_q_idx == 255`
- **THEN** the general intra frame frontier does not run for that frame; it
  routes to the frozen minimal hash tier
- **AND** the committed `syn-flat-intra-64x64-minimal.ivf` fixture is no longer a
  `base_q_idx == 255` frame: change `decode-minimal-fixture-avm-skip-polarity`
  replaced it with the AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream
  that routes through the general intra path

#### Scenario: Non-single-block partition is reported, not panicked
- **WHEN** an accepted general intra frame does not resolve to a supported
  single-block root partition frontier
- **THEN** the frontier returns a typed `decode/unsupported-feature` diagnostic
  with reason `general_intra_partition_frontier`
- **AND** no reconstruction, output, or reference state is produced
