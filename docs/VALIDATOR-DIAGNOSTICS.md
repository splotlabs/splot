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
```

## 2. New diagnostic namespaces to allow

Add these namespaces in the xtask diagnostic allowlist when corresponding checks land.

| Namespace | Owner | First features |
|---|---|---|
| `obu-payload/` | `AV2-5.2.1-OBU-DISPATCH` | unparsed payload in strict mode, payload overread/underread. |
| `obu-order/` | `AV2-7.3-OBU-ORDERING` child rows | temporal-unit and frame-unit order. |
| `hls-availability/` | `AV2-7.3.8-HLS-AVAILABILITY` | missing or repeated high-level syntax OBUs. |
| `msdo/` | `AV2-5.6-MSDO` | multistream decoder operation syntax/semantics. |
| `lcr/` | `AV2-5.8-LAYER-CONFIG-RECORD` | layer configuration record syntax/semantics. |
| `ops/` | `AV2-5.10-OPERATING-POINT-SET` | operating point set syntax/semantics. |
| `atlas/` | `AV2-5.9-ATLAS-SEGMENT` | atlas segment syntax/semantics. |
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
