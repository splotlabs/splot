## MODIFIED Requirements

### Requirement: Multi-frame runtime reaches precise unsupported gates

For the verified minimal multi-frame runtime subset, the decoder SHALL parse and validate AV2 v1.0.0 § 6.4.1 `bit_depth_idc` values before deciding whether the current runtime sample storage can decode them. The runtime MUST reject a 10-bit stream before caller-visible output whenever that stream would otherwise enter the existing `DecodedFrame<u8>` decode path.

The runtime SHALL continue to reject unsupported later frames before producing caller-visible output using the existing precise gates, including but not limited to extra OBUs in the leading key IVF payload, unsupported 10-bit runtime storage, non-regular-tile-group frame candidates, more than two valid references, `NumTotalRefs > 2`, neighbour-dependent unproven single-ref contexts, unmodeled cross-frame CDF loads, temporal MV state, unsupported tools, and unsupported geometry. This change SHALL NOT claim bit-exact decode for 10-bit streams or for streams beyond the committed fixtures.

#### Scenario: ac0ej3 reaches the next parse-only runtime gate

- **WHEN** `splot decode /Users/bartosztomczyk/Documents/SplotLabs/ac0ej3.ivf` runs with default decode limits
- **THEN** it advances past the former sequence bit-depth gate
- **AND** after the follow-on sequence chroma frontier it rejects before output at
  `unsupported_reason = "incomplete_frame_header"`

#### Scenario: 10-bit single-frame stream still cannot enter 8-bit decode

- **WHEN** a committed 10-bit IVF stream has an otherwise minimal leading `[TD, SEQ, CLK]` payload
- **THEN** the runtime rejects it before output with `unsupported_reason = "unsupported_bit_depth"`
- **AND** the diagnostic tracks `DECODE-AC0EJ3-10BIT-SEQUENCE-FRONTIER`
