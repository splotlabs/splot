## ADDED Requirements

### Requirement: single_ref entropy-element support row
The decoder support model SHALL track `DECODE-INTER-SINGLE-REF-SYMBOL` as a
distinct partial `splot-decode` row named `inter-single-ref-symbol`. The row SHALL
cite AV2 § 5.20.7.12, § 8.2.6, § 8.3.2, and § 9.3, SHALL record the
`SymbolEncoder` round-trip tests, and SHALL keep the § 8.3.2 neighbour-derived
context derivation, the runtime wiring (relaxing the `NumTotalRefs == 1` gate and
feeding two valid references), and `read_compound_ref` out of scope as deferred
work.

#### Scenario: Matrix records the partial single_ref entropy-element support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `inter-single-ref-symbol` appears with Feature ID
  `DECODE-INTER-SINGLE-REF-SYMBOL`
- **AND** it is marked partial rather than supported for inter decode
- **AND** it does not claim runtime `single_ref` decode, the § 8.3.2 neighbour
  context derivation, or `read_compound_ref`

#### Scenario: single_ref is proven bit-exact by a round-trip
- **WHEN** the `SymbolEncoder` <-> `read_single_ref` round-trip tests run
- **THEN** every selectable `RefFrame[0]` selection round-trips bit-exact over
  `TileSingleRefCdf[ctx][ref]` with `exit_symbol()` consistency
