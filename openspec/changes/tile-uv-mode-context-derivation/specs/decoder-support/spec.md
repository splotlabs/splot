## ADDED Requirements

### Requirement: Block-symbol uv_mode CDF context derivation

The tile CDF selection boundary (`DECODE-TILE-CDF-SELECTION-BOUNDARY`) SHALL
derive the AV2 § 8.3.2 `uv_mode` CDF context from the reconstructed luma `YMode`
instead of selecting it with a hardcoded literal. The minimal block-symbol trace
SHALL decode its symbols sequentially so a later symbol's context can depend on
earlier decodes: after decoding `y_mode_set` and `y_mode_index`, it SHALL
reconstruct the typed luma `YMode` (§ 5 `intra_y_mode_info`,
`get_intra_y_mode_set`, `Reordered_Y_Mode`) for the supported subset
(`y_mode_set == 0` with a non-directional `y_mode_index`, giving
`YMode == Reordered_Y_Mode[y_mode_index]`), and SHALL select
`TileUVModeCflNotAllowedCdf[ctx]` with `ctx = is_directional_mode(YMode)`
(§ 5 `is_directional_mode`: `V_PRED..=D67_PRED`). For the minimal flat-intra
fixture `YMode == DC_PRED`, which is non-directional, so the derived context is 0.
The reconstruction SHALL be total: inputs outside the supported subset SHALL
return a typed error routed to a `decode/unsupported-feature` diagnostic, without
panicking. This requirement SHALL NOT implement the in-frame `IntraJointModes`
neighbour lookup, the directional / `y_mode_offset` escape / `y_second_mode`
reconstruction paths, the `txb_skip` or `v_txb_skip` contexts, partition
decisions, full § 8.3 CDF selection, `decode_tile()`, reconstruction, hashes,
Y4M output, or reference refresh.

#### Scenario: uv_mode context is derived from the reconstructed YMode

- **WHEN** the minimal flat-intra block-symbol trace decodes `y_mode_set == 0`
  and `y_mode_index == 0` and then selects the `uv_mode` CDF row
- **THEN** the reconstructed luma `YMode` is `DC_PRED` (non-directional), so the
  derived § 8.3.2 context is `is_directional_mode(DC_PRED) == 0`
- **AND** the decoded trace is byte-for-byte unchanged (the no-output-change
  snapshot of symbol count, trailing bit, and padding end stays green)

#### Scenario: A directional luma mode raises the uv_mode context

- **WHEN** the `uv_mode` context is derived for a luma mode in the directional
  range `V_PRED..=D67_PRED`
- **THEN** the context is 1
- **AND** a non-directional mode (including `DC_PRED` and the `SMOOTH` / `PAETH`
  modes) gives a context of 0

#### Scenario: Unsupported YMode reconstruction is typed

- **WHEN** the decoded `y_mode_set` / `y_mode_index` fall outside the supported
  non-directional subset
- **THEN** `splot-decode` returns a structured error routed to a
  `decode/unsupported-feature` diagnostic
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Block-symbol context derivation remains partial

- **WHEN** decoder support status is rendered
- **THEN** the tile CDF selection boundary still reports partial status
- **AND** the in-frame neighbour lookup, the directional / escape / second-mode
  reconstruction paths, and the remaining block-symbol contexts (`txb_skip`,
  `v_txb_skip`) remain out of scope
