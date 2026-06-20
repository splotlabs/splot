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

#### Scenario: Frozen minimal hash contract is unchanged
- **WHEN** `splot decode` is given the committed frozen
  `syn-flat-intra-64x64-minimal.ivf` fixture with `base_q_idx == 255`
- **THEN** routing the general intra path does not run for that frame
- **AND** the decoded-frame hash remains the committed
  `splot-dfh-sha256-v1` digest

#### Scenario: Non-single-block partition is reported, not panicked
- **WHEN** an accepted general intra frame does not resolve to a supported
  single-block root partition frontier
- **THEN** the frontier returns a typed `decode/unsupported-feature` diagnostic
  with reason `general_intra_partition_frontier`
- **AND** no reconstruction, output, or reference state is produced
