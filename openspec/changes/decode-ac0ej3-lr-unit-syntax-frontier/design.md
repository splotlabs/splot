## Context

The current ac0ej3 frontier is `unsupported_wienerns_filter_bank`: the core
frame-header parser consumes AV2 §5.18.7.11 `lr_params()` and the frame-level
§5.20.10.6 Wiener NS bank, but the minimal runtime rejects before tile
mode-info because §5.20.10.4 `read_lr()` unit syntax is still outside the tile
partition traversal boundary. For frame-level Wiener NS filters, the per-unit
§5.20.10.6 call is a no-op when `readFrameFilters == 0`, so the next useful
brick is limited to the §5.20.10.5 `use_wiener_ns S()` symbols and LR type
accounting.

## Goals / Non-Goals

**Goals:**

- Track the work as `DECODE-AC0EJ3-LR-UNIT-SYNTAX-FRONTIER`.
- Add the `Default_Use_Wiener_Ns_Cdf` row to the existing tile CDF subset.
- Let the partition traversal consume the covered frame-level Wiener NS LR unit
  symbols before partition reads and surface a typed frontier once tile LR unit
  syntax has been modeled.
- Preserve fail-closed behavior before reconstruction, output, reference
  retention, and successful ac0ej3 decode.

**Non-Goals:**

- PC-Wiener, switchable LR unit syntax, per-unit Wiener coefficient syntax,
  temporal/reference Wiener state, source-sample handling, loop-restoration
  filtering, 10-bit reconstruction/output, AVM/dav2d integration, or encoder
  behavior.

## Decisions

- Use the existing tile CDF owner for `TileUseWienerNsCdf` instead of a
  standalone reader. This keeps CDF copy/average lifecycle behavior consistent
  with partition and block-symbol rows.
- Consume only `FrameRestorationType::WienerNonsep` planes with
  `frame_filters_on == true`. This matches the ac0ej3 frame-level bank path and
  avoids guessing per-unit coefficient parsing or PC-Wiener/switchable semantics.
- Keep runtime rejection as a structured `decode/unsupported-feature`
  diagnostic under a new decoder support row after LR unit syntax is consumed.
  This advances the frontier without claiming reconstruction correctness.

## Risks / Trade-offs

- **Risk:** A unit-count or tile-offset derivation bug could desynchronize the
  arithmetic stream. **Mitigation:** cover zero-symbol no-LR, single-unit, and
  multi-unit tests, and keep `exit_symbol()` / downstream syntax checks as the
  bit-exact guard.
- **Risk:** Supporting `use_wiener_ns == 0` could imply reconstruction decisions
  we do not yet store. **Mitigation:** consume and count LR unit types only for
  the traversal frontier, then reject before reconstruction/output.
- **Risk:** Tile CDF lifecycle drift if the new row is not copied/saved/scaled.
  **Mitigation:** add default-copy and update-mode tests for the new row.
