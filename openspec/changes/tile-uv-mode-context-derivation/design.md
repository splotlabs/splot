## Context

The block-symbol context derivation began with `y_mode_index`. The `uv_mode`
context is `is_directional_mode(YMode)`, which depends on the luma mode decoded
earlier in the same block. This change reconstructs `YMode` and restructures the
trace so the context can depend on prior decodes.

## Goals / Non-Goals

Goals:

- Derive the § 8.3.2 `uv_mode` context from the reconstructed luma `YMode`, with
  no change to decoded output.
- Establish the sequential-decode pattern (a later symbol's context derived from
  earlier decodes), keeping everything total and panic-free.

Non-Goals:

- `txb_skip` / `v_txb_skip` context derivation, the in-frame `get_joint_mode`
  neighbour lookup, and the directional / `y_mode_offset` escape / `y_second_mode`
  `YMode` reconstruction paths.

## Decisions

- **Sequential trace.** `consume_trace` changes from iterating a static
  `MinimalTraceItem` array to decoding each symbol explicitly via a
  `decode_block_symbol` helper (which preserves the existing `SymbolRead` /
  `UnexpectedSymbol` typed errors and returns the decoded value). This lets the
  `uv_mode` selector use a context computed from the `y_mode_set` / `y_mode_index`
  decoded just before it — the general entropy-decode shape where contexts depend
  on prior syntax.
- **`YMode` reconstruction, supported subset only.** `reconstruct_minimal_y_mode`
  implements § 5 for `y_mode_set == 0` with a non-directional `y_mode_index`
  (`0..NON_DIRECTIONAL_MODES_COUNT`): `modeIdx == y_mode_index` (the
  `MODE_INDEX_COUNT - 1 == 7` escape never applies for these indices),
  `get_intra_y_mode_set` passes it through, and `YMode == Reordered_Y_Mode[index]`
  (the non-directional reorder prefix). It returns `None` outside the subset; the
  directional reordering, the `y_mode_offset` escape, and the `y_mode_set != 0`
  (`y_second_mode`) path are `TODO(spec)`-deferred. For the fixture (set 0,
  index 0) it yields `DC_PRED`.
- **`uv_mode` context.** `uv_mode_ctx(YMode) = is_directional_mode(YMode)`
  (§ 5 `is_directional_mode`: `V_PRED..=D67_PRED`, canonical values 1..=8). For
  the non-directional subset (including `DC_PRED`) this is 0 — matching the
  previously hardcoded literal.
- **Totality.** The trace asserts `y_mode_set == 0` and `y_mode_index == 0`
  before reconstruction, so `reconstruct_minimal_y_mode` always succeeds in the
  minimal trace; the `None` case is handled with a typed
  `UnsupportedYMode` error (no panic) and routed to a
  `decode/unsupported-feature` diagnostic.
- **No-output-change proof.** Both derived contexts equal the previous literals
  for the fixture, so `block_symbol_frontier_accepts_minimal_fixture_trace`
  (symbol count 6, trailing bit 14, padding end 16) stays green; new unit tests
  pin the reconstruction and the `uv_mode` context directly.

## Risks / Trade-offs

- The sequential refactor must preserve the exact decode order and contexts; the
  no-output-change snapshot and the existing rollback/parse-failure tests guard
  against regressions.

## Migration Plan

Additive plus an internal refactor of `consume_trace`; one new typed error
variant and its diagnostic mapping. No public API change; the runtime and the
decoded output are unaffected.

## Open Questions

None.
