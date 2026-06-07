# Design: frame activation HLS skeleton

## 1. Boundary

This change adds **prefix parsing** for frame and tile-group syntax. It is a
validator-state feature, not a full frame parser.

The prefix parser stops after the fields needed for:

- frame header → sequence header reference (`seq_header_id_in_frame_header`);
- frame header → multi-frame header reference (`cur_mfh_id`);
- multi-frame header → sequence header reference (`mfh_seq_header_id`, already parsed);
- CLK/OLK activation paths that can be modeled without tile payload or entropy
  parsing.

The parser must be explicit that it does **not** consume all of §5.18 / §5.19, and
the validator must never run a full-payload trailing-bits check after a prefix-only
parse.

## 2. Parser model (`splot-core`)

`frame_header( isFirst )` (§5.18.1) calls `frame_header_info()` (§5.18.2) when
`isFirst`, and `frame_header_copy()` otherwise. The reachable activation prefix is
the head of `frame_header_info()`:

```text
frame_header_info() {
    IsBridge = obu_type == OBU_BRIDGE_FRAME
    if ( IsBridge ) cur_mfh_id = 0
    else            cur_mfh_id                       uvlc()
    if ( cur_mfh_id == 0 ) {
        seq_header_id_in_frame_header                uvlc()
        load_sequence_header( seq_header_id_in_frame_header )
    } else {
        load_sequence_header( MfhSeqHeaderId[ cur_mfh_id ] )
    }
    ...                                              # stop here (deeper §5.18 follows)
}
```

`FrameHeaderPrefix` carries `obu_type`, the obu-type-derived `is_key_frame` /
`is_bridge` / `is_regular`, `starts_cvs`, `cur_mfh_id`,
`seq_header_id_in_frame_header` (raw, for range checks), an optionally resolved
`referenced_sequence_header_id`, the consumed bit count, and a
`FrameHeaderPrefixStatus` that records that only activation fields were consumed.

`tile_group_obu( sz )` (§5.19) begins:

```text
tile_group_obu( sz ) {
    is_first_tile_group                              f(1)
    if ( is_first_tile_group ) frame_header_present_flag = 1
    else                       frame_header_present_flag  f(1)
    if ( frame_header_present_flag )
        frame_header( is_first_tile_group )
    ...                                              # tile payload (out of scope)
}
```

`TileGroupHeaderPrefix` carries `is_first_tile_group`, `frame_header_present_flag`,
an optional `FrameHeaderPrefix` (parsed only when `is_first_tile_group`, since a
non-first tile group carries `frame_header_copy()`, not a readable header), and the
consumed bit count.

The OBU dispatch (§5.2.1) routes `is_sef()` / `is_tip_frame()` / `OBU_BRIDGE_FRAME`
to `frame_header( 1 )` directly, and `is_tile_group()` (CLK, OLK, switch, RAS,
leading/regular tile groups) to `tile_group_obu()`.

## 3. HLS store (`splot-validate`)

Extend the in-band store with multi-frame-header records keyed by `mfh_id`:

```rust
struct MultiFrameHeaderRecord {
    mfh_id, mfh_seq_header_id, mfh_tlayer_id, mfh_mlayer_id, offset
}
```

On a parsed MFH OBU, record availability when the ids are in range, keeping the
existing `mfh/sequence-header-unavailable` reference check. External MFH availability
is **future**; `ValidationOptions` continues to model only external sequence headers.
Availability stays monotonic.

## 4. Activation model (`splot-validate`)

Separate three concepts:

- **available** sequence header: seen in-band or supplied externally;
- **stored** sequence header: the latest well-formed header parsed per `seq_header_id`;
- **active** sequence header: selected for an extended layer.

For skeleton-covered CLK/OLK paths (`is_key_frame`):

1. resolve the referenced sequence header directly (`cur_mfh_id == 0`) or through the
   MFH record (`cur_mfh_id > 0`);
2. when it resolves in-band, set it active for the frame's extended layer
   (overriding the OBU-order fallback);
3. perform layer-limit checks against the selected stored sequence header.

CVS-scoped sequence-header fingerprints and content-interpretation records are reset
at the **global temporal delimiter** (a temporal-unit boundary), so a sequence header
that opens a CVS keeps its fingerprint across the activating CLK and a non-identical
repeat later in the temporal unit is caught. A coded video sequence can span temporal
units, so cross-temporal-unit-within-CVS repeats remain a documented sound-over-
complete false negative (never a false positive). Frame paths the skeleton cannot
parse keep the previous conservative behavior.

This change does **not** model random-access / long-term-reference state, so exact
per-CVS scoping beyond the temporal unit is deferred (see
`AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY`).

## 5. Diagnostics

Emitted this phase:

- `hls/unavailable-sequence-header`
- `hls/unavailable-multi-frame-header`
- `frame-header/seq-header-id-out-of-range`
- `frame-header/cur-mfh-id-out-of-range`

Reserved (the prefix parser returns a structured `Error`; validator emission is
deferred until strict frame/tile payload parsing lands, so the existing
unparsed-payload behavior is preserved):

- `frame-header/prefix-parse-error`
- `tile-group/prefix-parse-error`

Existing diagnostics preserved: `mfh/sequence-header-unavailable`,
`hls/external-hls-disabled`, `hls/repeated-sequence-header-not-identical`,
`sequence-state/no-active-sequence-header`,
`content-interpretation/repeated-ci-not-identical`.

## 6. Inspector output

`inspect --json` adds a prefix-only summary for frame-bearing OBUs, clearly labelled:

```json
{
  "payload_kind": "frame_header_prefix",
  "prefix_status": "activation_fields_only",
  "cur_mfh_id": 0,
  "seq_header_id_in_frame_header": 1,
  "referenced_sequence_header_id": 1
}
```

Prefix-only data is never labelled a complete frame header.

## 7. Testing strategy

Use tiny synthetic OBU streams. Core parser tests cover `cur_mfh_id == 0`,
`cur_mfh_id > 0`, the tile-group-first path, and structured EOF errors. Validator
tests cover the four emitted diagnostics, external-HLS satisfaction, CLK-driven
activation, and the preserved in-CVS repeated-sequence-header check. AVM differential
testing remains a later milestone.
