## Context

`DECODE-AC0EJ3-LR-UNIT-SELECTIONS-FRONTIER` extended the supported LR syntax
frontier from aggregate active/inactive counts to a syntax-ordered per-unit
selection list. The runtime still rejects the local ac0ej3 stream at byte offset
74 because at least one retained unit selected active
`RESTORE_WIENER_NONSEP`, but the diagnostic still cites the older aggregate row.

## Goals / Non-Goals

**Goals:**

- Make the live active LR-unit diagnostic cite the selection-state row and
  Feature ID.
- Keep the existing unsupported reason stable:
  `unsupported_active_wienerns_lr_units`.
- Preserve fail-closed behavior before allocation, reference retention, hash,
  raw, or Y4M output.

**Non-Goals:**

- Applying AV2 loop restoration.
- Adding 10-bit decoded-frame storage or output.
- Changing LR syntax parsing, CDF mutation, or unit-selection storage.
- Changing public error fields or adding dynamic diagnostic payloads.

## Decisions

- Reuse the existing unsupported reason so downstream tooling that keys on the
  active LR-unit stop remains stable.
- Change only the owning matrix row / Feature ID and human-readable message,
  because the new row is now the live state frontier.
- Keep the older aggregate row as historical support evidence in the matrices.

## Risks / Trade-offs

- The diagnostic still cannot include concrete unit coordinates because
  `DecodeUnsupportedFeature` stores a static message. This change avoids a
  broader public error-shape change; coordinate payloads can be added later if
  a structured detail type is introduced.
