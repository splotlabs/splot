# Validator HLS availability and sequence state design

`scope: AV2 §6.2.2, §6.4, §7.3.7, §7.3.8`

## Purpose

The validator now has enough syntax coverage to maintain real state. The next state model should track which high-level syntax (HLS) OBUs are available, which sequence header is active for each extended layer, and how temporal units are ordered.

This state is for validation only. It is not a decoder state and must not require pixel reconstruction.

## Proposed state objects

```rust
pub struct ValidatorContext {
    sequence_headers: SequenceHeaderStore,
    hls: HlsAvailabilityStore,
    temporal_unit: TemporalUnitState,
    coded_video_sequences: CodedVideoSequenceStore,
    diagnostics_mode: DiagnosticsMode,
}

pub struct SequenceHeaderStore {
    by_key: BTreeMap<SequenceHeaderKey, StoredSequenceHeader>,
    active_by_xlayer: BTreeMap<ExtendedLayerId, SequenceHeaderKey>,
}

pub struct StoredSequenceHeader {
    key: SequenceHeaderKey,
    header: SequenceHeader,
    payload_fingerprint: PayloadFingerprint,
    first_seen_obu_index: usize,
    source: HlsSource,
}

pub enum HlsSource {
    InBand,
    External,
}

pub struct TemporalUnitState {
    index: u64,
    saw_global_temporal_delimiter: bool,
    hls_prefix_closed: bool,
    last_coded_xlayer: Option<ExtendedLayerId>,
    saw_coded_layer: bool,
    pending_global_suffix_metadata: bool,
}
```

The exact Rust names may differ. The important rules are:

- no global mutable state;
- one context instance per validation run;
- state changes are deterministic and testable;
- parsed payload objects are stored by strong IDs, not raw integers.

## Sequence-header state rules

### Availability

A parsed sequence header becomes available only after its payload has been parsed enough to know `seq_header_id`, layer identity, and the fields required for local checks. If parsing fails, do not store a partial header as available.

### Activation

For the next phase, activation may remain conservative:

- a sequence header OBU associated with an extended layer may become the active sequence for that layer when no active sequence exists yet;
- if a later frame header / CLK activation rule is not yet parseable, record a partial-coverage warning rather than fabricating activation semantics;
- when frame header parsing lands, move activation to the exact spec-defined trigger.

### Repeated copies

Within a coded video sequence of an extended layer, redundant copies of the activated sequence header are allowed only if bit-identical. Store a stable fingerprint over the OBU payload bytes used for the sequence header. Do not compare pretty-printed structs only; inferred values can hide payload differences.

### One active sequence per xlayer

Keep only one active sequence per extended layer until the validator can model the reset condition. Additional sequence headers with different IDs may be available but inactive. If a stream attempts to activate a different header before the reset is known to be legal, emit a partial/strict diagnostic.

## HLS availability rules

HLS OBUs must be available before reference. External availability should be modeled explicitly but disabled by default in the CLI/API.

Suggested API shape:

```rust
pub struct ValidationOptions {
    pub strict_unimplemented_payloads: bool,
    pub external_hls: ExternalHlsMode,
}

pub enum ExternalHlsMode {
    Disabled,
    AllowProvided(Vec<ExternalHlsObu>),
}
```

Do not assume external HLS exists unless the caller supplies it.

## Temporal-unit ordering rules to keep enforcing

Initial checks already exist. Strengthen them without needing frame/tile parsing:

1. a temporal unit starts with exactly one global temporal delimiter;
2. global HLS prefix OBUs appear before coded extended layer units;
3. coded extended layer units appear in ascending `obu_xlayer_id`;
4. padding outside coded extended layer units must be global;
5. metadata prefix/suffix positions stay todo until metadata fields are parsed;
6. frame-unit details stay todo until frame header and tile-group syntax are available.

## HLS payload priorities

| Priority | OBU | Feature ID | Reason |
|---:|---|---|---|
| 1 | Temporal delimiter | `AV2-5.5-TEMPORAL-DELIMITER` | Explicit temporal-unit boundary and state reset. |
| 2 | MSDO | `AV2-5.6-MSDO` | Multistream mode, substream maps, and global HLS ordering. |
| 3 | Multi-frame header | `AV2-5.7-MULTI-FRAME-HEADER` | Future frame header reuse; sequence reference validation. |
| 4 | LCR | `AV2-5.8-LAYER-CONFIG-RECORD` | Sequence/LCR association and layer maps. |
| 5 | OPS | `AV2-5.10-OPERATING-POINT-SET` | Operating point and extraction state. |
| 6 | Atlas | `AV2-5.9-ATLAS-SEGMENT` | LCR/atlas relationships. |

This phase should complete priorities 1–3 and leave 4–6 as the next HLS phase unless they remain small and fully tested.

## New diagnostics

| Diagnostic ID | Severity | Section | Trigger |
|---|---|---|---|
| `hls/unavailable-sequence-header` | error | §7.3.8 | OBU references a sequence header that is neither in-band-available nor externally provided. |
| `hls/repeated-sequence-header-not-identical` | error | §7.3.8 | Activated sequence header is repeated with different payload bytes inside a coded video sequence. |
| `hls/multiple-active-sequence-headers` | error or partial-warning | §7.3.8 | Different sequence header appears to become active before a valid reset is observed. |
| `hls/external-hls-disabled` | error | §7.3.8 | Validation would require external HLS but options disallow it. |
| `obu-order/duplicate-temporal-delimiter` | error | §7.3.7 | More than one global temporal delimiter starts the same temporal unit. |
| `obu-order/metadata-suffix-before-coded-layer` | error | §7.3.7 | Once metadata parsing exists: suffix metadata appears before coded layers. |
| `msdo/non-global-layer-id` | error | §6.6 | MSDO is not `tlayer=0`, `mlayer=0`, `xlayer=GLOBAL_XLAYER_ID`. |
| `msdo/too-many-streams` | error | §6.6 | `num_streams_minus_2 > 2`. |
| `mfh/seq-header-id-out-of-range` | error | §6.7 | MFH references a sequence header id outside `MAX_SEQ_NUM`. |
| `mfh/id-out-of-range` | error | §6.7 | `mfh_id_minus_1 + 1 >= MAX_MFH_NUM`. |

## Tests

Use synthetic streams. Each stream should be small and targeted:

```text
valid_temporal_unit_sequence_only.av2
bad_duplicate_temporal_delimiter.av2
bad_msdo_non_global.av2
bad_msdo_too_many_streams.av2
bad_sequence_repeated_different_payload.av2
bad_obu_tlayer_exceeds_active_sequence.av2
```

Unit tests should construct byte arrays directly where possible. Add fixture files only when they are useful for CLI/inspect tests.
