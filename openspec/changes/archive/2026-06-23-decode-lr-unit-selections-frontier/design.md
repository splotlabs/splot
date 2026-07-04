## Context

`TileLoopRestorationRootFrontier` currently reports only two aggregate counters:
how many supported frame-level Wiener NS LR units were consumed and how many were
active. AV2 §5.20.10.4 computes absolute `unitRow`/`unitCol` coordinates before
calling §5.20.10.5 `read_lr_unit()`. Preserving those coordinates alongside the
`use_wiener_ns` result is the next small state boundary needed by later §7.20
loop-restoration reconstruction wiring.

## Goals / Non-Goals

**Goals:**

- Preserve each supported frame-level Wiener NS LR-unit selection in syntax
  order.
- Store the plane and absolute LR unit row/column after tile-origin offset
  adjustment.
- Preserve current aggregate counters, inactive/all-active helpers,
  transactional CDF behavior, and resource-limit diagnostics.
- Keep the runtime's active-unit diagnostic unchanged until reconstruction is
  actually implemented.

**Non-Goals:**

- Applying §7.20 loop restoration, §7.20.2 source-sample clipping/stripe
  behavior, §7.20.4 PC-Wiener classification, chroma Wiener NS filtering,
  temporal/reference Wiener state, or output serialization.
- Changing runtime admission for active LR units.
- Adding public APIs, dependencies, or oracle fixtures.

## Decisions

- Store the selections in the existing crate-private
  `TileLoopRestorationRootFrontier`. This keeps the state at the traversal /
  runtime boundary that will eventually consume it.
- Use a small `WienerNsLrUnitSelection` record with plain coordinate fields.
  The caller can later combine this with frame-level filter banks from
  `FrameHeaderCore::lr_params` without reparsing tile syntax.
- Continue deriving active counts at the symbol read site. The stored `active`
  flag remains a direct representation of AV2 §5.20.10.5 `use_wiener_ns`.

## Risks / Trade-offs

- The new vector is not yet consumed by runtime reconstruction. Mitigation: the
  row remains partial and the existing active LR diagnostic remains the live
  local decoder mission stop.
- This preserves coordinates for the supported one-tile/root traversal subset
  only. Mitigation: unsupported SDP, switchable/PC-Wiener, and non-frame-filter
  paths remain rejected before a selection-state claim is made.
