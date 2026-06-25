# Change: Decode ac0ej3 IntrABC NEWMV records

## Summary

Advance the local `ac0ej3.ivf` Wiener NS LR selectable-transform runtime frontier
past the current IntrABC NEWMV stop by consuming the bounded AV2 §5.20.7.13 /
§5.20.7.20 block-vector syntax needed by §5.20.5.4 `read_intrabc_info()`.

## Motivation

The current probe reaches
`unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv` at byte
offset 110 after reading the active IntrABC mode-info prelude. That is the next
concrete syntax gap before current-frame block-copy prediction. The existing
inter `read_mv()` helper only exposes EighthPel/MvCtx0 rows, while IntrABC uses
`MV_INTRABC_CONTEXT` and frame/block precision from `intrabc_precision` /
`force_integer_mv`.

## Scope

- Expose the generated §9.3 `read_mv()` CDF rows needed for IntrABC P=5
  quarter-pel and P=3 one-pel syntax, while preserving the existing inter P=6
  path.
- Generalize the bounded SHELL-coded `read_mv()` helper so callers can select
  `MvCtx` and `MvPrecision`.
- In the ac0ej3 IntrABC path, derive the bounded reference block-vector
  candidate stack, apply §5.20.7.13 `assign_mv(0)` for NEARMV/NEWMV, and retain
  the decoded block vector.
- Continue to reject before prediction/current-frame copy, decoded sample
  population, output, and AVM/dav2d byte-equality claims.

## Out of Scope

- Current-frame IntrABC prediction / block copy.
- Broad §7.12.2 block-vector candidate modeling beyond the bounded local stack.
- Decoded `CurrFrame` / `CdefFrame` sample population.
- Loop-restoration filtering, reference refresh, final output, or successful
  full `ac0ej3.ivf` decode.
