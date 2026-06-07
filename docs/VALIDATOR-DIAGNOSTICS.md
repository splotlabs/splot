# Validator diagnostics registry

`status: draft`  
`owner: validator`  
`purpose: stable rule IDs for missing validator work`

Diagnostics are the validator product. Every finding must have:

- stable `rule_id`;
- `severity` (`error`, `warning`, `info`);
- optional `spec_section`;
- optional byte offset and bit offset;
- human-readable message;
- test coverage when the diagnostic is marked proven in `docs/IMPLEMENTATION-MATRIX.toml`.

## 1. Existing diagnostic namespaces

Keep these stable:

| Namespace | Purpose |
|---|---|
| `bitstream/` | Envelope, LEB128, OBU size, EOF, parse errors. |
| `obu-header/` | §6.2.2 OBU header conformance. |
| `obu-reserved/` | Reserved OBU checks. |
| `trailing-bits/` | §6.2.3 trailing bit conformance for modeled payload boundaries. |
| `byte-alignment/` | §6.2.4 byte-alignment zero-bit conformance for modeled syntax. |
| `sequence-header/` | §6.4 sequence-header local syntax and semantics. |
| `sequence-state/` | Activated sequence-header availability and max-layer checks. |
| `obu-order/` | Temporal-unit and coded-extended-layer ordering checks. |
| `hls/` | §7.3.8 high-level-syntax availability (sequence-header / multi-frame-header references and repeats). |
| `msdo/` | §6.6 Multi Stream Decoder Operation OBU checks. |
| `mfh/` | §5.7 / §6.4.1 multi-frame-header local id-range checks. |
| `content-interpretation/` | §5.15 / §6.14 content interpretation constraints. |
| `frame-header/` | §5.18 / §6.17 frame-header prefix references and local id ranges. |
| `tile-group/` | §5.19 tile-group prefix and ordering checks. |
| `tile-params/` | §6.17.7 sequence tile-params local constraints (tile counts, frame coverage). |
| `lcr/` | §5.8 / §6.8 / §7.3.8.3 layer-configuration-record syntax and availability. |
| `atlas/` | §5.9 / §6.9 / §7.3.8.4 atlas-segment syntax and availability. |

Existing examples:

```text
bitstream/parse-error
obu-header/global-xlayer-required
obu-header/global-xlayer-requires-base-layers
obu-header/global-xlayer-allowed-types
obu-header/base-layer-only-types
obu-header/temporal-layer-zero-only-types
obu-header/reserved-obu-type
obu-reserved/all-zero-payload
trailing-bits/missing-one-bit
trailing-bits/zero-bit-not-zero
byte-alignment/zero-bit-not-zero
sequence-header/chroma-format-out-of-range
sequence-state/no-active-sequence-header
sequence-state/tlayer-exceeds-max
sequence-state/mlayer-exceeds-max
obu-order/temporal-unit-missing-delimiter
obu-order/global-hls-after-coded-layer
obu-order/xlayer-order-not-ascending
obu-order/padding-non-global-outside-coded-layer
obu-order/duplicate-temporal-delimiter
sequence-header/timing-display-tick-zero
sequence-header/timing-time-scale-zero
hls/repeated-sequence-header-not-identical
msdo/non-global-layer-id
msdo/too-many-streams
mfh/seq-header-id-out-of-range
mfh/id-out-of-range
tile-params/tile-cols-out-of-range
tile-params/tile-rows-out-of-range
tile-params/nonuniform-cols-do-not-cover-frame
tile-params/nonuniform-rows-do-not-cover-frame
```

## 2. New diagnostic namespaces to allow

Add these namespaces in the xtask diagnostic allowlist when corresponding checks land.

| Namespace | Owner | First features |
|---|---|---|
| `obu-payload/` | `AV2-5.2.1-OBU-DISPATCH` | unparsed payload in strict mode, payload overread/underread. |
| `hls-availability/` | `AV2-7.3.8-HLS-AVAILABILITY` | missing or repeated high-level syntax OBUs. |
| `msdo/` | `AV2-5.6-MSDO` | multistream decoder operation syntax/semantics. |
| `ops/` | `AV2-5.10-OPERATING-POINT-SET` | operating point set syntax/semantics. |
| `metadata/` | `AV2-5.17-METADATA` | metadata unit and type-specific checks. |
| `padding/` | `AV2-5.16-PADDING` | padding payload constraints. |
| `film-grain/` | `AV2-5.14-FILM-GRAIN` | film grain field constraints. |
| `quant-matrix/` | `AV2-5.13-QUANTIZATION-MATRIX` | quantization matrix field constraints. |
| `content-interpretation/` | `AV2-5.15-CONTENT-INTERPRETATION` | content interpretation constraints. |
| `frame-header/` | `AV2-5.18-FRAME-HEADER` child rows | frame header syntax/semantics. |
| `tile-group/` | `AV2-5.19-TILE-GROUP` and `AV2-5.20-TILE-GROUP-PAYLOAD` | tile group syntax/payload boundary checks. |
| `annex-a/` | Annex A rows | profile/level/tier constraints. |
| `decoder-model/` | Annex E rows | timing/decoder-model constraints. |

## 3. Phase 1 diagnostics

Feature: `AV2-5.2.3-TRAILING-BITS`

```text
trailing-bits/empty
trailing-bits/missing-one-bit
trailing-bits/zero-bit-not-zero
trailing-bits/payload-bits-remaining-negative
trailing-bits/payload-bits-unconsumed
```

Feature: `AV2-5.2.4-BYTE-ALIGNMENT`

```text
byte-alignment/zero-bit-not-zero
byte-alignment/eof
```

Feature: `AV2-5.2.1-OBU-DISPATCH`

```text
obu-payload/unimplemented-in-strict-mode
obu-payload/parsed-beyond-declared-size
obu-payload/trailing-bits-invalid
obu-payload/extensible-obu-extension-data-present
obu-payload/extensible-obu-extension-data-invalid
```

Severity guidance:

- malformed payload bits are `error`;
- extension data in an extensible OBU may be `info` or `warning` if preserved and spec-compliant;
- unimplemented payload in non-strict partial validator mode may be `warning`, but strict mode should reject once the repository policy says payload parsing is required.

## 4. Sequence header diagnostics

Feature: `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`

Initial local checks:

```text
sequence-header/seq-header-id-out-of-range
sequence-header/chroma-format-out-of-range
sequence-header/bit-depth-out-of-range
sequence-header/seq-max-mlayer-count-out-of-range
sequence-header/crop-left-out-of-range
sequence-header/crop-right-out-of-range
sequence-header/crop-top-out-of-range
sequence-header/crop-bottom-out-of-range
sequence-header/timing-num-units-zero
sequence-header/timing-time-scale-zero
sequence-header/timing-num-ticks-out-of-range
sequence-header/timing-fields-change-within-cvs
sequence-header/decoder-model-fields-change-within-cvs
sequence-header/long-term-frame-id-bits-required
sequence-header/user-qm-zero-value
sequence-header/quant-delta-out-of-range
sequence-header/monotonic-output-order-mismatch-in-multistream
```

Activation/state checks:

```text
sequence-state/no-active-sequence-header
sequence-state/unknown-sequence-header-id
sequence-state/sequence-header-changed-within-cvs
sequence-state/tlayer-exceeds-max
sequence-state/mlayer-exceeds-max
sequence-state/mlayer-count-exceeds-sequence-max
sequence-state/lcr-reference-unavailable
sequence-state/global-lcr-does-not-include-xlayer
```

Severity guidance:

- local §6.4 conformance violations are `error`;
- unavailable external HLS objects should be `error` unless the CLI/API explicitly supplies them;
- repeated but bit-identical active sequence header can be `info` or no diagnostic;
- repeated active sequence header with changed content is `error`.

## 5. OBU ordering diagnostics

Feature: `AV2-7.3-OBU-ORDERING` child rows

```text
obu-order/temporal-unit-missing-delimiter
obu-order/temporal-unit-duplicate-delimiter
obu-order/global-hls-after-coded-layer
obu-order/xlayer-order-not-ascending
obu-order/padding-non-global-outside-coded-layer
obu-order/metadata-prefix-after-coded-layer
obu-order/metadata-suffix-before-coded-layer
obu-order/frame-unit-mixed-layer-ids
obu-order/frame-unit-missing-frame-header
obu-order/tile-group-outside-frame-unit
obu-order/random-access-msdo-missing
obu-order/msdo-changed-outside-random-access
```

Severity guidance:

- direct presence-order violations are `error`;
- incomplete ordering checks due to missing payload parsers are `warning` with a clear feature ID until the dependent row lands.

## 6. HLS availability diagnostics

Feature: `AV2-7.3.8-HLS-AVAILABILITY`

```text
hls-availability/sequence-header-unavailable
hls-availability/msdo-unavailable-at-rap
hls-availability/msdo-non-identical-repeat
hls-availability/lcr-global-unavailable
hls-availability/lcr-local-unavailable
hls-availability/atlas-unavailable
hls-availability/ops-unavailable
hls-availability/external-hls-not-supported
```

## 7. HLS OBU diagnostics

### MSDO

```text
msdo/profile-out-of-range
msdo/level-out-of-range
msdo/tier-exceeds-stream-tier
msdo/sub-xlayer-id-duplicate
msdo/sub-xlayer-id-not-ascending
msdo/doh-constraint-violated
```

### LCR

```text
lcr/global-id-out-of-range
lcr/local-id-zero
lcr/xlayer-map-empty
lcr/xlayer-map-bit31-set
lcr/dependency-info-invalid
lcr/profile-tier-level-mismatch
lcr/max-expected-width-exceeds-sequence
lcr/max-expected-height-exceeds-sequence
lcr/computed-payload-bits-negative
```

### OPS

```text
ops/mlayer-info-idc-reserved
ops/embedded-op-index-out-of-range
ops/computed-payload-size-mismatch
ops/ptl-missing
ops/xlayer-map-bit31-set
ops/mlayer-map-invalid
ops/tlayer-map-invalid
```

### Atlas

```text
atlas/segment-mode-out-of-range
atlas/region-columns-out-of-range
atlas/region-rows-out-of-range
atlas/segment-count-out-of-range
atlas/lcr-reference-missing
atlas/region-overlap-invalid
```

## 8. Non-HLS OBU diagnostics

### Metadata

```text
metadata/type-out-of-range
metadata/unit-payload-size-mismatch
metadata/group-payload-size-mismatch
metadata/muh-layer-idc-out-of-range
metadata/muh-xlayer-map-bit31-set
metadata/muh-metadata-type-mismatch
metadata/scan-type-invalid
metadata/pic-struct-invalid
metadata/display-hash-invalid
```

### Padding

```text
padding/non-zero-byte
padding/global-layer-rule-violated
```

### Film grain

```text
film-grain/update-flags-zero
film-grain/chroma-idc-out-of-range
film-grain/payload-size-mismatch
```

### Quantization matrix

```text
quant-matrix/quant-delta-out-of-range
quant-matrix/user-qm-zero-value
quant-matrix/payload-size-mismatch
```

### Content interpretation

```text
content-interpretation/field-out-of-range
content-interpretation/payload-size-mismatch
```

## 9. Frame and tile diagnostics

Frame header diagnostics should be added only with frame-header child features. Start with these namespaces and split later.

```text
frame-header/sequence-header-missing
frame-header/mfh-reference-missing
frame-header/frame-size-exceeds-sequence-max
frame-header/crop-exceeds-frame-size
frame-header/order-hint-inconsistent
frame-header/reference-map-invalid
frame-header/long-term-reference-id-invalid
frame-header/tile-info-invalid
frame-header/quantizer-out-of-range
frame-header/filter-param-out-of-range
frame-header/global-motion-invalid
frame-header/film-grain-reference-invalid
```

Tile group diagnostics:

```text
tile-group/frame-header-missing
tile-group/tile-count-out-of-range
tile-group/tile-size-exceeds-payload
tile-group/arithmetic-stream-overread
tile-group/exit-symbol-trailing-bits-invalid
tile-group/tile-payload-unparsed-in-strict-mode
```

## 10. Testing expectations per diagnostic

Every new diagnostic requires:

1. one positive case that does **not** emit it;
2. one negative case that emits it;
3. byte offset when available;
4. spec section in the diagnostic;
5. proof entry in `docs/IMPLEMENTATION-MATRIX.toml` when the feature stage is `done`;
6. CLI JSON test for at least one diagnostic per new namespace.

Suggested test naming:

```rust
#[test]
fn sequence_header_rejects_chroma_format_greater_than_3() { ... }

#[test]
fn obu_order_rejects_non_ascending_xlayer_units() { ... }
```

## 11. JSON compatibility rule

Diagnostic JSON is part of the product. Do not rename existing fields without a compatibility plan. Adding fields is acceptable when the CLI tests are updated.

Recommended diagnostic JSON fields:

```json
{
  "severity": "error",
  "rule_id": "sequence-header/chroma-format-out-of-range",
  "spec_section": "6.4.1",
  "byte_offset": 42,
  "bit_offset": 3,
  "message": "chroma_format_idc must be <= 3, found 4",
  "feature_id": "AV2-6.4-SEQUENCE-HEADER-SEMANTICS"
}
```

`feature_id` can be added later; if added, update all snapshot tests and docs.

## 12. Frame activation HLS skeleton (implemented)

OpenSpec change `frame-activation-hls-skeleton`. The prefix-only frame/tile-group
parser (`AV2-5.18.2-FRAME-HEADER-INFO`, `AV2-5.19-TILE-GROUP`) drives these
generic high-level-syntax reference checks.

Emitted (error unless noted):

```text
hls/unavailable-sequence-header        # §7.3.8.6: cur_mfh_id == 0 references a missing seq header
hls/unavailable-multi-frame-header     # §7.3.8.7: cur_mfh_id > 0 references a missing MFH
frame-header/seq-header-id-out-of-range  # §6.17: seq_header_id_in_frame_header >= MAX_SEQ_NUM
frame-header/cur-mfh-id-out-of-range     # §6.17: cur_mfh_id >= MAX_MFH_NUM
```

An out-of-range id emits only the `frame-header/*-out-of-range` diagnostic, not the
matching `hls/unavailable-*` (no double-report). The existing
`mfh/sequence-header-unavailable`, `hls/external-hls-disabled`,
`hls/repeated-sequence-header-not-identical`, and
`content-interpretation/repeated-ci-not-identical` diagnostics are preserved; the
last two benefit from the new temporal-unit-scoped CVS reset.

Reserved (the `splot-core` prefix parser returns a typed `Error` on EOF / invalid
descriptors; validator emission is deferred until strict frame/tile payload parsing
lands, so the current unparsed-frame-payload behavior — and its tests — are
preserved):

```text
frame-header/prefix-parse-error
tile-group/prefix-parse-error
```

## 13. Sequence tile params (implemented)

OpenSpec change `segmentation-tile-params-foundation`. The sequence `tile_params()`
helper (`AV2-5.18.7.3-TILE-PARAMS`, used by `AV2-5.4.2-SEQUENCE-TILE-CONFIG`) drives
these local §6.17.7 tile constraints on a fully parsed sequence tile config.

Emitted (all `error`):

```text
tile-params/tile-cols-out-of-range            # §6.17.7.2: TileCols > MAX_TILE_COLS (64)
tile-params/tile-rows-out-of-range            # §6.17.7.2: TileRows > MAX_TILE_ROWS (64)
tile-params/nonuniform-cols-do-not-cover-frame  # §6.17.7.3: column starts != sbCols
tile-params/nonuniform-rows-do-not-cover-frame  # §6.17.7.3: row starts != sbRows
```

The tile-count diagnostics are reachable for a non-uniform config that codes more than
`MAX_TILE_COLS` / `MAX_TILE_ROWS` tiles. The frame-coverage diagnostics are a defensive
cross-check: the `ns()`-bounded non-uniform parse caps each tile to the remaining
superblocks, so coverage is exact for any decodable stream. They are therefore
**unreachable for a stream that parses without error** (a parse error surfaces first)
and are unit-tested via a synthetic `TileParams` rather than a bitstream — they only
guard the invariant should a `TileParams` be produced another way.

Wiring note: with `seg_info()` (`AV2-5.4.9-SEGMENT-INFO`) and `tile_params()` now
parsed in full, a valid sequence header and a multi-frame header parse completely, so
the existing §5.2.1 payload-tail checks (`trailing-bits/*`, `byte-alignment/*`,
`obu-header/extension-flag-not-zero`) now run on them and a malformed tail after the
segment or tile info is diagnosed. The only residual bounded sequence-header case is a
reserved (non-conformant) `seq_level_idx` with tile info present, which has no defined
tile bit layout (`AV2-5.4.2-SEQUENCE-TILE-CONFIG`).

## 14. HLS LCR/atlas foundation (implemented)

OpenSpec change `hls-lcr-atlas-foundation`. The full §5.8 / §5.9 parsers
(`AV2-5.8-LAYER-CONFIG-RECORD`, `AV2-5.9-ATLAS-SEGMENT`) drive these checks. Syntax
checks (`LayerConfigRecordSyntax`, `AtlasSegmentSyntax`) run statelessly; the
availability checks are stateful and live in `crate::context`.

Emitted (error unless noted):

```text
lcr/reserved-bits-nonzero                 # warning, §6.8: a reserved-zero field is non-zero
lcr/dependent-xlayers-flag-nonzero        # warning, §6.8.2: lcr_dependent_xlayers_flag must be 0
lcr/payload-size-overflow                 # §6.8.6: lcr_global_payload parsed bits > lcr_data_size * 8
lcr/global-id-out-of-range                # §6.8.2: lcr_global_config_record_id must be in 1..7
lcr/xlayer-map-empty                      # §6.8.2: lcr_xlayer_map must be in 1..(1 << 31) - 1
lcr/local-id-zero                         # §6.8.3: lcr_local_id must not be 0
lcr/global-lcr-unavailable                # §7.3.8.3: local LCR lcr_global_id has no global LCR
lcr/global-xlayer-map-missing-xlayer      # §6.4.1: seq_lcr_id global LCR omits the header xlayer
atlas/segment-mode-out-of-range           # §6.9: ats_atlas_segment_mode_idc > 4
atlas/region-dimension-out-of-range       # §6.9.3.1: region columns/rows >= MAX_ATLAS_COLS/ROWS
atlas/segment-count-out-of-range          # §6.9.6: segment count >= MAX_NUM_ATLAS_SEGMENTS
atlas/multistream-requires-global-xlayer  # §6.9: MULTISTREAM(_ALPHA) requires GLOBAL_XLAYER_ID
atlas/duplicate-input-stream-id           # §6.9.4/§6.9.6: ats_input_stream_id / ats_msi_input_stream_id must be unique
atlas/local-atlas-unavailable             # §7.3.8.4: local LCR lcr_local_atlas_id has no local atlas
hls/unavailable-layer-configuration-record  # §7.3.8.3/§7.3.8.6: seq_lcr_id resolves to no LCR
```

Availability errors are gated on external HLS being disabled (matching the
multi-frame-header path), since an externally-provided LCR/atlas is not modeled. The
availability store records global-LCR (`id -> lcr_xlayer_map`), local-LCR
(`xlayer -> {lcr_local_id}`), and local-atlas (`{(xlayer, atlas_segment_id)}`) entries
after a successful parse and a valid §5.2.1 payload tail, and stays monotonic.

Intentional non-checks (spec honesty):

- The global atlas (§7.3.8.4) uses "can be available", so a missing global atlas is
  not flagged.
- §6.8 / §6.9 define no "repeated record must be identical" requirement, so no
  duplicate-not-identical check is emitted (unlike `OBU_MSDO` / sequence headers).
- MFH layer-dependency-map checks (`mfh/mlayer-dependency-violation`,
  `mfh/tlayer-dependency-violation`) remain reserved: `MLayerDependencyMap` /
  `TLayerDependencyMap` are not exposed by the sequence-header model, so they are not
  fabricated from max layer ids (`TODO(spec: AV2-5.7-MULTI-FRAME-HEADER)`).
