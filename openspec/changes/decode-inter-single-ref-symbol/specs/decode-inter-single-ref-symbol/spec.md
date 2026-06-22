## ADDED Requirements

### Requirement: Read the § 5.20.7.12 single_ref entropy element bit-exact
The decoder SHALL provide a crate-private `read_single_ref` that reads the AV2
§ 5.20.7.12 `single_ref` entropy element and returns the selected `RefFrame[0]`.
It SHALL loop `ref` from `0` to `NumTotalRefs - 2`, read one binary `single_ref`
symbol per `ref` over `TileSingleRefCdf[ctx][ref]` (the AV2 § 8.3.2 CDF row, with
`ctx` caller-supplied because the neighbour derivation is deferred), return `ref`
on the first symbol that decodes to `1`, and return `NumTotalRefs - 1` when every
symbol decodes to `0` — matching the spec loop and AVM `read_single_ref`. The
reader SHALL use typed errors only and SHALL NOT panic.

The element SHALL be proven bit-exact by a `SymbolEncoder` <-> `read_single_ref`
round-trip: for the full range of selectable `RefFrame[0]` values and distinct
per-decision contexts, the symbols encoded with `SymbolEncoder` over the same
`TileSingleRefCdf[ctx][ref]` rows SHALL decode back to the encoded selection, and
the § 8.2.4 `exit_symbol()` bit count SHALL be consistent. The selections and
contexts SHALL be asymmetric so a transposed tree decision or a wrong CDF-row
index would change the decoded selection.

This element SHALL be loaded-but-unwired: it SHALL NOT be wired into the runtime
decode path and the `NumTotalRefs == 1` runtime gate SHALL NOT be relaxed, so
every existing inter and intra fixture SHALL continue to decode byte-identically.
The § 8.3.2 neighbour-derived `single_ref` context derivation
(`av2_get_ref_pred_context`), the runtime wiring (the § 7.7 two-valid-slot
reference feed and the multi-frame reference-retention loop that make
`NumTotalRefs >= 2` reachable), and `read_compound_ref` SHALL remain out of scope
as the named follow-on (the multi-reference runtime brick).

#### Scenario: single_ref selections round-trip through SymbolEncoder
- **WHEN** the `single_ref` symbol sequence for a target `RefFrame[0]` selection
  is encoded with `SymbolEncoder` over `TileSingleRefCdf[ctx][ref]` rows
- **THEN** `read_single_ref` decodes the same `RefFrame[0]` selection over an
  identical CDF subset
- **AND** every selectable selection (0 through `NumTotalRefs - 1`) round-trips
  with `exit_symbol()` consistency

#### Scenario: a transposed CDF-row index changes the decode
- **WHEN** the per-decision CDF row is indexed `[ref][ctx]` instead of
  `[ctx][ref]`
- **THEN** the asymmetric round-trip no longer recovers the encoded selection or
  fails `exit_symbol()`, so the tree shape and `[ctx][ref]` indexing are pinned

#### Scenario: NumTotalRefs == 1 returns the only reference with no symbol read
- **WHEN** `read_single_ref` is called with `NumTotalRefs == 1`
- **THEN** it returns `0` (`NumTotalRefs - 1`) without consuming any symbol bit,
  because the § 5.20.7.12 loop is empty (the legal one-reference case; § 6.19.7.11
  only requires `NumTotalRefs > 0`)

#### Scenario: NumTotalRefs == 0 is a typed error before any read
- **WHEN** `read_single_ref` is called with `NumTotalRefs == 0`
- **THEN** it returns a typed error before consuming any symbol bit (§ 6.19.7.11
  requires `NumTotalRefs > 0`; the spec computes `NumTotalRefs - 1`, which would
  underflow at 0)

#### Scenario: invalid inputs are typed errors, not panics
- **WHEN** `read_single_ref` is given fewer contexts than decisions, an
  out-of-range context, or a short tile payload
- **THEN** it returns a typed error (or, for a short payload that the § 8.2
  decoder reads from implicit padding, a valid in-range selection) and never
  panics

#### Scenario: existing fixtures are unchanged
- **WHEN** `splot decode` is given the existing inter and general-intra fixtures
- **THEN** each decodes to its previously-recorded bit-exact output, because the
  element is loaded-but-unwired
