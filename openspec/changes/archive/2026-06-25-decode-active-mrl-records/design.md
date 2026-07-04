## Context

The merged selectable narrow-record handoff moves the local decoder mission probe to
`unsupported_wienerns_lr_live_transform_record_mrl_mode` at byte offset 110. At
that point the runtime has consumed AV2 §5.20.5.5 `mrl_index` and
`mrl_sec_index`, but returns before retaining mode metadata or deriving the
current block's `LrTxSkip` facts.

AV2 §5.20.5.3 writes `UsesMrls` for every luma/shared MI cell after mode-info,
and AV2 §8.3.2 uses neighbouring `UsesMrls` to select the next `mrl_index` and
`mrl_sec_index` CDF rows. The current implementation hardcodes both MRL contexts
to zero because all admitted blocks had `mrl_index == 0`.

## Goals / Non-Goals

**Goals:**
- Retain decoded `mrl_index` and optional `mrl_sec_index` for luma/shared
  leaves in the active Wiener NS LR selectable transform-record path.
- Add tile-local `UsesMrls` state and use it for AV2 §8.3.2 MRL CDF context
  selection.
- Allow active MRL mode-info to proceed into transform-record and skipped
  residual parsing when no decoded sample prediction is claimed.
- Move the local decoder mission probe beyond the active-MRL unsupported diagnostic to
  the next precise unsupported runtime frontier.

**Non-Goals:**
- No §7.13.2 MRL prediction, edge preparation, DIP, IBP, or intra-edge filtering.
- No decoded `CurrFrame` / `CdefFrame` sample population, loop-restoration
  filtering/output, reference refresh, AVM/dav2d byte equality, or successful
  local decoder mission decode.
- No new dependencies, crate dependency changes, encoder behavior, or spec
  mirror edits.

## Decisions

1. Store MRL state beside existing intra neighbour state.

   `TileIntraJointModeState` already owns tile-local per-MI luma mode context.
   Add a sibling `TileUsesMrlsState` in the same module so §8.3.2 `UsesMrls`
   context derivation follows the same indexing and clipping conventions as
   `IntraJointModes`.

2. Return `UsesMrls` through `GeneralIntraLeafMode`.

   The partition walker is the single place that records per-leaf luma/shared
   mode state. Extending the existing `GeneralIntraLeafMode` return value keeps
   MRL state recording centralized and avoids special-casing the local decoder mission LR
   callback.

3. Replace active-MRL erroring with metadata retention for mode-info only.

   `decode_general_intra_luma_block_mode` should still read the exact
   §5.20.5.5 symbols, but should return `mrl_index`, `mrl_sec_index`, and the
   derived `UsesMrls` value instead of treating nonzero MRL as an error. Any
   caller that actually reconstructs samples must continue rejecting active MRL
   before prediction until §7.13.2 support exists.

4. Keep failure modes structured and narrow.

   Unsupported branches after this handoff remain `decode/unsupported-feature`
   diagnostics tied to the active local decoder mission matrix rows. The change must not turn a
   later unsupported prediction/filtering requirement into a successful output.

## Risks / Trade-offs

- Active MRL affects neighbour CDF context. If state is not recorded after each
  luma/shared leaf, later MRL symbols may be read from the wrong CDF row.
  Mitigation: add focused tests for left/above `UsesMrls` context derivation and
  symbol row selection.
- General intra reconstruction callers may accidentally accept active MRL.
  Mitigation: keep the existing runtime sample-decode gate rejecting nonzero
  `mrl_index` before prediction, and scope the new admission evidence to LR
  tx-skip record derivation only.
- The next local decoder mission frontier is not known until the runtime advances past active
  MRL. Mitigation: update the matrix/support notes and CLI regression after the
  implementation probe identifies the new structured diagnostic.
