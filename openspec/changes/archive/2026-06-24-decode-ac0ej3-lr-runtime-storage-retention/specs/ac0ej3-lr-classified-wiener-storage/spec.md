## MODIFIED Requirements

### Requirement: ac0ej3 Classified Wiener Storage Frontier

The decoder SHALL track `DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-STORAGE` as a
fail-closed runtime frontier for AV2 §7.20.4 luma classified-Wiener value
derivation backed by decoded current/CDEF frame storage and retained `LrTxSkip`
storage.

#### Scenario: Storage values derive FilterClass

- **WHEN** active luma Wiener NS LR blocks have `frame_filters_on == 1` and
  `NumFilterClasses > 1`, and the decoder helper is supplied decoded current
  and CDEF frame views plus a bounded `LrTxSkip` grid
- **THEN** the helper SHALL read §7.20.2 source sample values from the selected
  frame storage, read boolean `LrTxSkip` values from the grid, call the §7.20.4
  PC-Wiener classifier, and record the derived `FilterClass[y >> 2][x >> 2]`
  value for each classified block

#### Scenario: Storage lookup failures remain typed

- **WHEN** the supplied `LrTxSkip` storage cannot cover a classifier lookup or
  contains a non-boolean value
- **THEN** the helper SHALL return a typed error instead of fabricating a
  transform-skip value or panicking

#### Scenario: Live ac0ej3 advances to runtime storage retention

- **WHEN** the live ac0ej3 minimal runtime reaches active classified-luma Wiener
  NS LR units after decoded-frame/grid-backed classified-Wiener helper plumbing
  exists
- **THEN** the runtime SHALL advance to the
  `DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION` diagnostic after deriving and
  limit-checking live 10-bit frame-buffer and `LrTxSkip` grid storage shapes
- **AND** this storage-helper row SHALL NOT claim loop-restoration filtering,
  output, reference refresh, AVM/dav2d equality, or successful ac0ej3 decode
