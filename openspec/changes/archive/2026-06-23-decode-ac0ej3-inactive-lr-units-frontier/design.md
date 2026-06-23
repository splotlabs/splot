## Context

The minimal runtime currently parses the ac0ej3 key frame through AV2
§5.20.10.4/§5.20.10.5 LR-unit syntax and then unconditionally emits
`unsupported_wienerns_lr_unit_syntax`. That is fail-closed, but it loses the
semantic result of each `use_wiener_ns` symbol. AV2 §5.20.10.5 assigns
`RESTORE_NONE` when `use_wiener_ns == 0`; only `RESTORE_WIENER_NONSEP` enters
the following `read_wienerns_filter(..., readFrameFilters == 0)` branch.

## Goals / Non-Goals

**Goals:**

- Record how many supported frame-level Wiener NS LR units were consumed and how
  many selected `RESTORE_WIENER_NONSEP`.
- Let the runtime continue past the LR frontier when all consumed units selected
  `RESTORE_NONE`.
- Preserve existing resource-limit, SDP, unsupported LR variant, and
  transactional CDF behavior.
- Keep the implementation matrix and decoder support matrix honest about the
  new narrow Feature ID `DECODE-AC0EJ3-INACTIVE-LR-UNITS-FRONTIER`.

**Non-Goals:**

- Applying Wiener NS, PC-Wiener, switchable LR, CDEF, deblocking, GDF, CCSO, or
  film grain.
- Modeling per-unit Wiener coefficient parsing when frame-level filters are not
  available.
- Adding 10-bit sample storage, output serialization, reference refresh support,
  broad ac0ej3 decode success, new public APIs, or new dependencies.

## Decisions

- Return LR activity in the existing crate-private
  `TileLoopRestorationRootFrontier` summary. This keeps the behavior local to
  the traversal/runtime boundary and avoids exposing a new public API.
- Count active units at the point of the `use_wiener_ns` CDF read. This mirrors
  AV2 §5.20.10.5 directly: the symbol selects either `RESTORE_NONE` or
  `RESTORE_WIENER_NONSEP`.
- Keep active units unsupported in the minimal runtime. The existing
  `splot-recon` primitive is luma-only and does not cover runtime source-sample
  selection, chroma, unit traversal, filter ordering, or 10-bit output.
- Continue using existing `DecodeError::Limit` mapping for LR-frontier resource
  limits. A tight limit must still fail before committing CDF updates or
  reporting a later unsupported feature.

## Risks / Trade-offs

- Active LR units could remain the current ac0ej3 blocker. Mitigation: the
  local ignored regression will assert the exact current diagnostic after the
  activity summary is wired.
- A later diagnostic such as 10-bit storage may reuse an older unsupported
  reason. Mitigation: focused tests will prove LR-unit syntax was consumed and
  all units were inactive before that later gate is reached.
- Counting activity without storing per-unit coordinates is intentionally
  limited. Mitigation: broad reconstruction remains unsupported, and the row
  names the missing per-unit LR state as out of scope.
