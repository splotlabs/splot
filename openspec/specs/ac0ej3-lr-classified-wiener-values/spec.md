## Purpose

Track the ac0ej3 fail-closed runtime frontier for AV2 §7.20.4 classified-Wiener
value derivation.

## Requirements

### Requirement: ac0ej3 Classified Wiener Value Frontier

The decoder SHALL track `DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-VALUES` as a
fail-closed runtime frontier for AV2 §7.20.4 luma classified-Wiener value
derivation after classified source-read and `LrTxSkip` lookup coordinates have
been resolved.

#### Scenario: Supplied values derive FilterClass

- **WHEN** active luma Wiener NS LR blocks have `frame_filters_on == 1` and
  `NumFilterClasses > 1`, and the decoder helper is supplied source sample
  values plus `LrTxSkip` values
- **THEN** the helper SHALL call the §7.20.4 PC-Wiener classifier and record the
  derived `FilterClass[y >> 2][x >> 2]` value for each classified block

#### Scenario: Live ac0ej3 stops at storage boundary

- **WHEN** the live ac0ej3 minimal runtime reaches active classified-luma Wiener
  NS LR units before 10-bit current/CDEF frame storage and `LrTxSkip` storage are
  available
- **THEN** the runtime SHALL return `decode/unsupported-feature` referencing
  `DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-VALUES`
- **AND** the diagnostic SHALL NOT claim real source sample reads, real
  `LrTxSkip` reads, loop-restoration filtering, output, or successful ac0ej3
  decode
