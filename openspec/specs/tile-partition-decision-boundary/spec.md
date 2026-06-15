# tile-partition-decision-boundary Specification

## Purpose
TBD - created by archiving change tile-partition-decision-boundary. Update Purpose after archive.
## Requirements
### Requirement: Tile partition decision boundary

The decoder SHALL provide a crate-private AV2 §5.20.3.2 partition decision boundary tracked by Feature ID `DECODE-TILE-PARTITION-DECISION-BOUNDARY` and decoder support matrix row `tile-partition-decision-boundary`. The boundary SHALL accept caller-provided allowed-partition facts, implied-partition facts, BRU-active state, rect-type implication facts, and already-bounded CDF context inputs; SHALL follow the AV2 §5.20.3.2 branch order to return one typed partition outcome; SHALL use the existing partition-entry `S()` symbol-read boundary only for reached `do_split`, `do_square_split`, `rect_type`, `do_ext_partition`, and `do_uneven_4way_partition` branches; and SHALL consume `uneven_4way_partition_type L(1)` only when that branch is reached. The boundary SHALL NOT claim `partition_implied`, `init_allowed_partitions`, `is_partition_allowed`, recursive `read_partition()`, recursive `decode_partition()`, `decode_tile()`, block syntax traversal, `MiSizes` mutation, `exit_symbol()`, Saved CDF mutation, reconstruction, output, reference refresh, public API behavior, or external decoder invocation.

#### Scenario: Early return branches do not consume symbols
- **WHEN** the caller-provided facts make §5.20.3.2 return through an allowed implied partition, a single allowed partition, or inactive BRU mode
- **THEN** the decision boundary returns the expected typed partition outcome
- **AND** no `S()` symbol read or `L(1)` literal read is performed
- **AND** tile CDF rows remain unchanged

#### Scenario: Disallowed implied partitions fall through
- **WHEN** `partition_implied` supplies an implied partition that is not allowed by the caller-provided allowed set
- **THEN** the boundary follows §5.20.3.2 by continuing to the later single-allowed, inactive-BRU, or syntax-read branches
- **AND** it does not return a typed error solely because the implied partition was disallowed

#### Scenario: Conditional partition symbols are consumed in branch order
- **WHEN** caller-provided facts require the boundary to evaluate `do_split`, `do_square_split`, `rect_type`, `do_ext_partition`, or `do_uneven_4way_partition`
- **THEN** each reached syntax element is read through the existing crate-private symbol-read boundary exactly once
- **AND** unreachable later syntax elements are not read after an earlier branch returns
- **AND** the returned trace records which syntax elements were consumed

#### Scenario: Rectangular partition table maps reached flags
- **WHEN** the branch order reaches the final `Rect_Part_Table` lookup with bounded `do_ext_partition`, `do_uneven_4way_partition`, `uneven_4way_partition_type`, and `rectType` facts
- **THEN** the decision boundary returns the corresponding AV2 §5.20.3.2 typed partition outcome
- **AND** out-of-range or internally inconsistent caller facts return a typed error instead of panicking

#### Scenario: Uneven four-way literal read is isolated
- **WHEN** `do_ext_partition` is true and `do_uneven_4way_partition` is true
- **THEN** the boundary consumes exactly one `L(1)` literal for `uneven_4way_partition_type`
- **AND** it skips the `do_uneven_4way_partition S()` read when that value is implied by allowed facts
- **AND** it still consumes the literal whenever the resulting `do_uneven_4way_partition` value is true
- **AND** it does not consume the literal when the resulting `do_uneven_4way_partition` value is false

#### Scenario: Error paths are transactional
- **WHEN** selector derivation, symbol decoding, literal reading, empty allowed-set validation, or table-result validation fails
- **THEN** the boundary returns a crate-private typed error
- **AND** failures detected before syntax consumption do not advance the symbol decoder or mutate CDF rows
- **AND** failures after a reached syntax read expose the underlying symbol/literal error without panicking

#### Scenario: Scope remains narrower than traversal
- **WHEN** decoder support status is generated
- **THEN** `tile-partition-decision-boundary` records support only for one partition decision over caller-provided facts
- **AND** broader tile payload decode, full CDF selection, recursive partition traversal, reconstruction, output, reference refresh, and public runtime decode remain tracked by their existing rows
