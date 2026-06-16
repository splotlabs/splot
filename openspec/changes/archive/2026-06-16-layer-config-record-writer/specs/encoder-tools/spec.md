# encoder-tools delta: layer-config-record-writer

## ADDED Requirements

### Requirement: layer configuration record OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `layer_config_record_obu()` (§ 5.8) and
its § 5.8.1–5.8.9 sub-structures back to bytes — the inverse of `parse_layer_config_record` — so the
complete-OBU dispatch round-trips this OBU type instead of returning `Unimplemented`. The writer SHALL
reproduce the parser-tolerated reserved-zero and descriptive values verbatim within their descriptor
domain (the `lcr_*_reserved_zero_*` fields the § 6.8 semantics say "shall be equal to 0" but the parser
retains), so an already-parsed model always round-trips. It SHALL be reject-before-write and SHALL never
panic on a constructed model, rejecting the decidable inconsistencies (a gated `Option` that disagrees
with its present flag, a `seq_ptl_infos` / `payloads` / embedded-`layers` list that disagrees with the
set-bit map it is derived from, a `lcr_global_payload` whose content plus `remaining_payload_bits` does
not equal `lcr_data_size * 8`, the embedded-layer-vs-atlas else-branch exclusivity, and out-of-range
field values).

#### Scenario: a parsed layer configuration record OBU round-trips

- **WHEN** a parsed `layer_config_record_obu()` of either scope (global or local), including the
  aggregate, PTL, payload, embedded-layer, color, and atlas sub-structures, is written by the dispatch
  and the bytes are reparsed
- **THEN** the reparsed `LayerConfigurationRecord` SHALL equal the original, byte-exact on the canonical
  subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `LayerConfigurationRecord` the parser could never produce (a
  flag-vs-`Option`, set-bit-derived list, payload-size, atlas-vs-embedded exclusivity, or out-of-range
  inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.
