## Context

The tile CDF selection boundary already derives bounded § 8.3.2 contexts for the
partition-entry symbols (`do_split`, `do_square_split`, etc.). The block-symbol
side of the minimal flat-intra trace still uses hardcoded `ctx` literals. This
change begins block-symbol § 8.3.2 context derivation with `y_mode_index`.

## Goals / Non-Goals

Goals:

- Derive the § 8.3.2 `y_mode_index` context exactly, replacing the literal in the
  minimal trace, with no change to decoded output.
- Keep the derivation total and panic-free.

Non-Goals:

- `uv_mode` / `txb_skip` / `v_txb_skip` context derivation, `YMode`
  reconstruction, the consume-trace sequential refactor, in-frame
  `get_joint_mode` neighbour lookup, and neighbour mode-state tracking.

## Decisions

- **Out-of-frame branch only, for the single-block tile-origin case.** The
  minimal flat-intra frontier decodes one block at `MiRow == MiCol == 0`. The
  `y_mode_index` derivation reads `get_joint_mode(0)` (left, `MiCol - 1`) and
  `get_joint_mode(1)` (above, `MiRow - 1`); both are out of frame, so § 5
  `get_joint_mode` returns `DC_PRED`. `DC_PRED` (mode 0) is non-directional
  (`< NON_DIRECTIONAL_MODES_COUNT`, which is 5), so each § 8.3.2 indicator term is
  0 and the context is 0 — matching the previously hardcoded literal. The in-frame
  `IntraJointModes[mvRow][mvCol]` lookup is marked deferred with a
  `TODO(spec: DECODE-TILE-CDF-SELECTION-BOUNDARY)` because the frontier tracks no
  neighbour mode state yet.
- **General formula, specialized constructor.** `YModeIndexContext` stores both
  neighbours' joint modes and `ctx()` is the § 8.3.2 sum-of-indicators formula
  (honest and reusable for any joint modes); `tile_origin_block()` is the
  minimal-frontier specialization that supplies `DC_PRED` for both.
- **No-output-change proof reuses the existing snapshot.** The derived ctx equals
  the old literal, so the frozen-trace snapshot then named
  `block_symbol_frontier_accepts_minimal_fixture_trace` (symbol count 6, trailing
  bit 14, padding end 16) stayed green unchanged at the time of this change. (That
  snapshot test was later retired by `decode-minimal-fixture-avm-skip-polarity`,
  which replaced the committed fixture with a conformant general-path luma-skip
  stream; the frozen trace is now covered by
  `block_symbol_trace_rejects_legacy_inverted_skip`.) New unit tests pin the
  derivation directly (ctx 0 at the origin; ctx 1/2 for directional neighbours;
  the non-directional boundary).

## Risks / Trade-offs

- Deriving a context that currently always evaluates to 0 for the fixture is a
  small step, but it replaces a magic literal with the spec formula and
  establishes the block-symbol context-derivation scaffold the remaining banks
  build on.

## Migration Plan

Additive; a new crate-private module and a one-line trace change. No public API
change; the runtime and the decoded output are unaffected.

## Open Questions

None.
