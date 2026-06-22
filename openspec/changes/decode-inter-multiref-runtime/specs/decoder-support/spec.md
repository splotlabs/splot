## ADDED Requirements

### Requirement: multi-reference inter runtime support row
The decoder support model SHALL track `DECODE-INTER-MULTIREF-RUNTIME` as a
distinct partial `splot-decode` row named `inter-multiref-runtime`. The row SHALL
cite AV2 § 5.20.7.6, § 5.20.7.10, § 5.20.7.12, § 7.7, § 7.20, § 7.23, and § 8.3.2,
SHALL record the committed `syn-3frame-multiref-64x64.ivf` three-oracle agreement,
and SHALL keep § 7.23 cross-frame CDF save/load, `NumTotalRefs > 2`, a
neighbour-having `single_ref` context, compound references, temporal MV, and the
deferred § 7.12.2 candidates out of scope as named follow-on work.

#### Scenario: Matrix records the partial multi-reference runtime support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `inter-multiref-runtime` appears with Feature ID
  `DECODE-INTER-MULTIREF-RUNTIME`
- **AND** it is marked partial rather than supported for inter decode
- **AND** it does not claim `NumTotalRefs > 2`, compound references, a
  neighbour-having `single_ref` context, or cross-frame CDF save/load

#### Scenario: the multi-reference runtime is proven bit-exact by a 3-oracle fixture
- **WHEN** the committed `syn-3frame-multiref-64x64.ivf` decodes through
  `splot decode`
- **THEN** its whole-stream raw output matches avmdec and dav2d byte-for-byte, and
  the third frame reads the retained inter frame (slot 1), not the key (slot 0)

### MODIFIED Requirement: single_ref entropy-element support row
The decoder support model SHALL track `DECODE-INTER-SINGLE-REF-SYMBOL` as a
partial `splot-decode` row named `inter-single-ref-symbol`. The row SHALL cite AV2
§ 5.20.7.12, § 8.2.6, § 8.3.2, and § 9.3, SHALL record the `SymbolEncoder`
round-trip tests, and SHALL note that the element is now WIRED into the runtime
inter block decode (read when § 7.7 yields `NumTotalRefs == 2`, with the § 8.3.2
context derived from the neighbour `count_refs`) by
`DECODE-INTER-MULTIREF-RUNTIME`. `NumTotalRefs > 2` (multi-decision `single_ref`),
a neighbour-having `single_ref` context, and `read_compound_ref` SHALL remain out
of scope.

#### Scenario: Matrix records the single_ref entropy element as wired
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `inter-single-ref-symbol` appears with Feature ID
  `DECODE-INTER-SINGLE-REF-SYMBOL`
- **AND** its note records that the element is wired into the runtime for
  `NumTotalRefs == 2` by `DECODE-INTER-MULTIREF-RUNTIME`
- **AND** it does not claim `NumTotalRefs > 2`, a neighbour-having context, or
  `read_compound_ref`
