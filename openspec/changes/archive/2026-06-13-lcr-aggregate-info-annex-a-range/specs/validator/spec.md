# validator delta: lcr-aggregate-info-annex-a-range

Closes the `validate` residual on `AV2-5.8.3-LCR-AGGREGATE-INFO` by enforcing the three
§ 6.8.4 Annex-A value-space constraints on a global LCR's `lcr_aggregate_info()`.

## ADDED Requirements

### Requirement: global LCR aggregate-info fields stay within Annex A value spaces

The validator SHALL, for every parsed global layer configuration record whose
`lcr_aggregate_info_present_flag == 1`, verify the three § 6.8.4 Annex-A value-space
constraints (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-4, lines 1737-1760)
directly from the parsed `lcr_aggregate_info()`, without requiring activation:

- `lcr_config_idc` is one of the Annex A.3 Table A.5 defined multi-sequence configurations
  (`0..=2`); a value in `3..=63` produces `lcr/config-idc-reserved`.
- `lcr_aggregate_level_idx` is not a reserved Annex A.4 Table A.7 level index; a value in
  `22..=30` produces `lcr/aggregate-level-idx-reserved`.
- `lcr_max_interop` is one of the Annex A.3 Table A.3 defined interoperability points
  (`0`, `1`, `2`, `15`); a value in `3..=14` produces `lcr/max-interop-reserved`.

Each diagnostic is an error anchored at the layer configuration record OBU's byte offset and
cites § 6.8.4. `lcr_max_tier_flag` carries no such clause (a 1-bit field) and is not checked.
The checks are local to the parsed record — disjoint from the § 6.8.2 MSDO
aggregate-agreement checks — so a global LCR observed without activation is still checked.

#### Scenario: reserved multi-sequence configuration

- **WHEN** a global LCR carries `lcr_aggregate_info` with `lcr_config_idc == 3`
- **THEN** an error diagnostic `lcr/config-idc-reserved` (§ 6.8.4) is produced

#### Scenario: reserved aggregate level index

- **WHEN** a global LCR carries `lcr_aggregate_info` with `lcr_aggregate_level_idx == 22`
- **THEN** an error diagnostic `lcr/aggregate-level-idx-reserved` (§ 6.8.4) is produced

#### Scenario: reserved interoperability point

- **WHEN** a global LCR carries `lcr_aggregate_info` with `lcr_max_interop == 3`
- **THEN** an error diagnostic `lcr/max-interop-reserved` (§ 6.8.4) is produced

#### Scenario: defined aggregate values stay silent

- **WHEN** a global LCR carries `lcr_aggregate_info` with `lcr_config_idc == 2`,
  `lcr_aggregate_level_idx == 31` ("Maximum parameters"), and `lcr_max_interop == 15` ("max")
- **THEN** none of `lcr/config-idc-reserved`, `lcr/aggregate-level-idx-reserved`, or
  `lcr/max-interop-reserved` is produced

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
