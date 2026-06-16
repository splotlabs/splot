## Why

Phase 3 of the decoder conformance program is gated by the symbol decoder. The
`symbol-decoder` row (`AV2-8.2-SYMBOL-DECODER`) has been `partial` since the
foundation landed, with the stated reason that the same AV2 § 8.2.2 / § 8.2.4
text also covers Tile/Saved CDF copies and CDF averaging that "belong with
future tile-CDF-bank work". That work has since shipped: the
`tile-cdf-save-lifecycle-boundary` row is `supported` and now owns the § 8.2.4
copy/average half. The remaining § 8.2 *symbol-decoder primitive* in
`crates/splot-core/src/symbol.rs` is spec-complete (`init_symbol`, `read_bool`,
`read_literal`, `read_symbol` with exact arithmetic and CDF adaptation, and the
`exit_symbol()` trailing/padding conformance portion), is byte-stream reachable
through the `supported` minimal-tier runtime and the `symbol_decoder_bytes`
fuzz target, and is verified spec-exact by independent review.

What blocks calling the primitive `supported` is *evidence*, not code: the unit
tests only pin arities N=2 and N=4, the property test exercises a single fixed
CDF, and the CDF-update rate extremes and deep-negative `SymbolMaxBits` padding
path are not asserted by value. This change adds that evidence and promotes the
row, scoping the broader § 8.3 / CDF-bank / `decode_tile()` work to its own
already-tracked rows.

## What Changes

- Promote the `symbol-decoder` row in `docs/DECODER-SUPPORT-MATRIX.toml` from
  `partial` to `supported`, rewriting the notes so the deferred work (§ 8.3
  CDF selection → `tile-cdf-selection-boundary`; Tile/Saved CDF banks and the
  § 8.2.4 copy/average half → `tile-cdf-save-lifecycle-boundary`; § 9.3 default
  banks; `decode_tile()`/traversal → `tile-payload-decode`; reconstruction /
  hash / Y4M → the runtime rows) is described as separately-tracked scope, not
  as a reason this primitive is incomplete.
- Advance the `AV2-8.2-SYMBOL-DECODER` feature stages (`parse`, `decode_check`,
  `tests`) from `partial` to `done` in `docs/IMPLEMENTATION-MATRIX.toml`, with
  recorded proof, and update its notes for the same reason.
- Add § 8.2 primitive test evidence in `crates/splot-core/src/symbol.rs`:
  - extreme-value vectors decoding the first and last symbol for every arity
    N = 2..8 (exercises every `Prob_Inc[N-2]` row end-to-end);
  - exact CDF-update results at the minimum and maximum adaptation rates
    (pins the `>> rate` shift extremes by value);
  - a deep-negative-`SymbolMaxBits` run asserting deterministic implicit-zero
    padding and no panic;
  - a property test over **random valid CDF rows of every arity** asserting the
    decoded symbol is in range, the updated row stays in the valid probability
    range with a capped count, decoding is deterministic, and disabled updates
    leave the row byte-for-byte unchanged.
- No production code changes to the symbol decoder (it is already spec-exact)
  and no new public API.

## Capabilities

### Modified Capabilities

- `decoder-support`: Mark the `AV2-8.2-SYMBOL-DECODER` § 8.2 symbol-decoder
  primitive `supported`, with strengthened arity/rate/edge/random-CDF
  evidence, while keeping § 8.3 CDF selection, default CDF banks, runtime tile
  decode, reconstruction, and output tracked as `partial`/`unsupported` in
  their own rows.

## Impact

- Affected code: `crates/splot-core/src/symbol.rs` (tests only).
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated `docs/DECODER-SUPPORT-STATUS.md`,
  `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`,
  `docs/DECODER-SPEC-COVERAGE.md`, and `docs/DECODER-ROADMAP.md`.
- APIs: no public API changes.
- Diagnostics: no new diagnostic rule IDs.
- Dependencies: no new third-party dependencies and no AVM/dav2d integration.
