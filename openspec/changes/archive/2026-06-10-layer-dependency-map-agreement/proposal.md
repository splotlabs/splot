# Layer-dependency-map agreement checks

## Why

The sequence-header model now exposes the derived § 5.4.1 dependency maps
(`SequenceHeaderGeneral::{m,t}layer_dependency_map`, landed with
`AV2-5.4.1-SEQUENCE-HEADER-GENERAL`), which unblocks three long-deferred
cross-OBU agreement checks that today are documented as intentional non-checks
in `docs/VALIDATOR-DIAGNOSTICS.md` ("Deferred pending infrastructure") and as
backlog rows in `docs/VALIDATOR-ROADMAP.md`: the § 6.10.7 OPS dependency-map
agreement, the § 6.8.9 LCR dependency-map agreement, and the § 7.3.8.7
multi-frame-header layer-dependency constraints (concrete predicate in
§ 6.17.2). A conformance validator that parses these maps but never compares
them silently accepts streams whose extraction/operating-point signalling
contradicts the activated sequence header.

## What Changes

- New `ops/mlayer-dependency-missing` and `ops/tlayer-dependency-missing`
  error diagnostics (§ 6.10.7, mirror
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10-7`): explicitly
  signalled `ops_mlayer_map` / `ops_tlayer_map` entries must be
  dependency-closed under the activated sequence header's
  `MLayerDependencyMap` / `TLayerDependencyMap`.
- New `lcr/mlayer-dependency-missing` and `lcr/tlayer-dependency-missing`
  error diagnostics (§ 6.8.9, mirror
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-9`): the
  activated LCR's `lcr_mlayer_map[isGlobal][xId]` /
  `lcr_tlayer_map[isGlobal][xId][cMId]` must be dependency-closed under the
  activated sequence header's maps.
- New `frame-header/mfh-mlayer-dependency-missing` and
  `frame-header/mfh-tlayer-dependency-missing` error diagnostics (§ 7.3.8.7,
  mirror `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-3-8-7`; concrete
  predicate § 6.17.2 `MLayerDependencyMap[obu_mlayer_id][MfhMLayerId[cur_mfh_id]] == 1`
  and `TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][MfhTLayerId[cur_mfh_id]] == 1`),
  resolving the `TODO(spec: AV2-5.7-MULTI-FRAME-HEADER)` in
  `crates/splot-validate/src/context.rs`.
- Validator state grows what the checks need: the active-OPS records keep the
  explicitly signalled per-entry mlayer/tlayer maps, and the HLS store keeps
  the parsed LCR embedded-layer info per `(global_id, xId)` and
  `(xlayer, local_id)`.
- Registry/docs updates: new rows in `docs/VALIDATOR-DIAGNOSTICS.md`, removal
  of the corresponding "Deferred pending infrastructure" non-check note,
  removal of the two landed backlog rows in `docs/VALIDATOR-ROADMAP.md`, and
  matrix updates (see Impact).

All checks are conservative: they only run when the required activated
sequence header (and, for the MFH check, the in-band-resolved referenced
sequence header) is modeled in-band — no diagnostic is fabricated from
defaults, guessed state, or external HLS.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `validator`: three new requirement groups — OPS § 6.10.7 dependency-map
  agreement, LCR § 6.8.9 dependency-map agreement, and frame-header MFH
  § 7.3.8.7 layer-dependency checks — each with positive (silent) and negative
  (flagged) scenarios and explicit no-false-positive scenarios for unavailable
  state.

## Impact

- **Feature IDs:** `AV2-5.10-OPERATING-POINT-SET` (umbrella; § 6.10.7 noted in
  its notes), `AV2-5.11-OPERATING-POINT-PAYLOAD` / `AV2-5.11.5-OPS-MLAYER-INFO`
  (the § 5.11.5 fields the § 6.10.7 requirement constrains, if that child row
  exists — otherwise the 5.11 umbrella), `AV2-5.8-LAYER-CONFIG-RECORD` umbrella
  and child `AV2-5.8.8-LCR-EMBEDDED-LAYER-INFO` (§ 6.8.9), and
  `AV2-5.7-MULTI-FRAME-HEADER` (§ 7.3.8.7). `validate` stages stay `partial`
  (Annex A/E operating-point semantics, decoder-ignored reserved-value rules,
  and `lcr_max_expected_*` frame-size bounds remain future); notes and
  `diagnostics` proofs are updated.
- **Code:** `crates/splot-validate/src/context.rs` (OPS semantics check, LCR
  observation/storage, frame-header MFH reference resolution, sequence-header
  activation hook), the validator-side OPS/HLS stores, and inline tests in
  `crates/splot-validate/src/validator.rs`. `splot-core` parsing is already
  complete; at most a small read-only accessor is added if needed.
- **Docs:** `docs/VALIDATOR-DIAGNOSTICS.md` (six new registry rows; drop the
  now-implemented deferred note), `docs/VALIDATOR-ROADMAP.md` (drop the two
  landed backlog rows; refresh the Phase 6 status sentence),
  `docs/IMPLEMENTATION-MATRIX.toml` (notes + diagnostics for the rows above).
- **User-facing:** six new stable error rule ids; no existing rule id,
  severity, or spec section changes.

## Non-goals

- The § 6.17.2 MFH frame-size override bounds
  (`mfh_frame_width_minus_1 <= max_frame_width_minus_1`, same for height) —
  frame-size conformance is tracked separately from layer dependencies.
- Annex A/E operating-point level/schedule semantics (stays future per the
  matrix).
- § 6.8.9 constraints that need unmodeled state: decoder-ignore rules for
  reserved `lcr_layer_type` / `lcr_auxiliary_type` / `lcr_view_type` values and
  the `lcr_max_expected_width/height` vs `FrameWidth`/`FrameHeight` bounds.
- External-HLS modeling changes (`TODO(spec: AV2-7.3.8-HLS-AVAILABILITY)`
  stays).
- Encoder, writer, or AVM-differential work.
