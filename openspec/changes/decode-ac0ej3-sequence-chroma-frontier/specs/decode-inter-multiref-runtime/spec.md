## MODIFIED Requirements

### Requirement: Multi-frame runtime reaches precise unsupported gates

For the verified minimal multi-frame runtime subset, the decoder SHALL parse and
validate sequence-level chroma prediction capability flags before deciding whether
the current runtime can decode their reachable tile syntax. The runtime MUST
reject CfL or MHCCP before tile mode-info symbols are decoded whenever §5.20.5.6
could require `is_cfl`, `UV_CFL_PRED`, `is_mhccp_allowed`, or related MHCCP
handling that the minimal runtime has not implemented.

The runtime SHALL continue to reject unsupported later frames before producing
caller-visible output using the existing precise gates, including but not limited
to incomplete key-frame headers, extra OBUs in the leading key IVF payload,
unsupported 10-bit runtime storage, non-regular-tile-group frame candidates, more
than two valid references, `NumTotalRefs > 2`, neighbour-dependent unproven
single-ref contexts, unmodeled cross-frame CDF loads, temporal MV state,
unsupported tools, and unsupported geometry. This change SHALL NOT claim bit-exact
decode for CfL, MHCCP, 10-bit streams, or streams beyond the committed fixtures.

#### Scenario: ac0ej3 reaches the key-frame header frontier

- **WHEN** `splot decode /Users/bartosztomczyk/Documents/SplotLabs/ac0ej3.ivf`
  runs with default decode limits
- **THEN** it advances past the former sequence CFL gate
- **AND** it rejects before output at `unsupported_reason =
  "incomplete_frame_header"`

#### Scenario: sequence chroma tools stay fail-closed before tile decode

- **WHEN** a stream has otherwise reachable leading key-frame syntax but its
  sequence enables CfL or MHCCP
- **THEN** the runtime rejects before tile mode-info decode with
  `unsupported_reason = "unsupported_cfl_intra"` or
  `unsupported_reason = "unsupported_mhccp"`
- **AND** the diagnostic tracks `DECODE-AC0EJ3-SEQUENCE-CHROMA-FRONTIER`
