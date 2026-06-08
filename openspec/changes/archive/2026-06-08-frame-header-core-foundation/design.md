# Design: frame-header core foundation

## Background

`frame_header_info()` is the first frame-bearing syntax structure that ties together sequence state, MFH state, reference state, order hints, frame sizes, QM, film grain, tiling, segmentation, and later tile payload. The existing `splot` parser intentionally reads only the activation/reference prefix. That prefix enabled correct HLS availability checks but now blocks deeper validator coverage.

## Architecture

Introduce parser modes:

```rust
pub enum FrameHeaderParseMode {
    ActivationPrefix,
    Core,
}
```

The current behavior maps to `ActivationPrefix`. New code uses `Core` where the validator has sufficient state.

Every parse result carries a status:

```rust
pub enum FrameHeaderParseStatus {
    ActivationFieldsOnly,
    CoreFieldsOnly,
    ShowExistingFrameComplete,
    StoppedBeforeFilteringQuantSegmentation,
    UnsupportedUntilFeature { feature_id: &'static str },
}
```

This prevents accidental claims of full §5.18 coverage.

## State input

Frame parsing must accept explicit state:

```rust
pub struct FrameHeaderParseInput<'a> {
    pub obu_type: ObuType,
    pub first_picture_in_tu: bool,
    pub active_sequence: Option<&'a SequenceHeader>,
    pub mfh_record: Option<&'a MultiFrameHeaderRecord>,
    pub reference_state: FrameReferenceStateView<'a>,
    pub mode: FrameHeaderParseMode,
}
```

`active_sequence` is the full `SequenceHeader`, not just its general fields: the core
parser reads `OrderHintBits`, `NumRefFrames`, `long_term_frame_id_bits`,
`enable_short_refresh_frame_flags`, and the screen-content force flags from the
sequence inter and screen-content configs. The validator therefore retains the full
parsed `SequenceHeader` per `seq_header_id`. When the inter/scc config is absent (the
header was not fully parsed) the core parser degrades to `ActivationFieldsOnly` rather
than guessing.

The parser must not guess missing state.

## Core parse surface

The first core implementation should cover:

- `cur_mfh_id`;
- direct/MFH sequence-reference resolution;
- derived frame-kind flags;
- start-CVS derivation;
- bridge-frame reference index when reference count is known;
- show-existing-frame path fields when state-supported;
- frame type derivation/signaling;
- output flags;
- order hint;
- primary-reference signaling;
- refresh flags;
- direct/default `frame_size()` where dimensions are available;
- screen-content and intra-block-copy params where reached.

The implementation may stop before filtering/quantization/segmentation/tiling and return an explicit status.

## Validator

Add only locally decidable checks. If a check requires unimplemented reference-frame state, leave it deferred or emit a non-error unsupported-state diagnostic only in strict mode.

## Inspector

Expose parse status and known fields in JSON. Unknown fields stay absent/null.

## Alternatives considered

### Implement tile group first

Rejected. Tile-group completion depends on frame-derived `NumTiles`, `bru_inactive`, `use_bru`, and frame-level `tile_info()`.

### Implement full §5.18 in one PR

Rejected. The section is too broad and would hide risk across filtering, quantization, segmentation/tiling, reference state, global motion, and frame film grain.

### Jump to AVM differential harness

Deferred. Useful soon, but existing parser coverage still has a clear internal blocker: frame-header core.
