## ADDED Requirements

### Requirement: ac0ej3 LR Source-Bounds Frontier Support Row

The decoder support model SHALL track
`DECODE-AC0EJ3-LR-SOURCE-BOUNDS-FRONTIER` as a distinct ac0ej3 support row. The
row SHALL describe that the minimal runtime consumes supported active
Wiener NS LR-unit syntax, consumes required §5.20.10.6 per-unit filter syntax,
retains per-unit selection state, derives active §7.20.1 source-bound facts, and
still fails closed before
source-frame reads, loop-restoration filtering, 10-bit output, or successful
ac0ej3 decode.

#### Scenario: Matrix evidence records the source-bounds boundary

- **WHEN** decoder support status is validated
- **THEN** `ac0ej3-lr-source-bounds-frontier` remains partial
- **AND** the row lists tests proving active source-bound retention and the live
  ac0ej3 runtime diagnostic
- **AND** the live diagnostic reason is `unsupported_wienerns_lr_source_bounds`
