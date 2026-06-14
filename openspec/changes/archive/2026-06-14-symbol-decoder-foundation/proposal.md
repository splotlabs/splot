## Why

The decoder roadmap's next missing infrastructure step is AV2 § 8 symbol
decoding: non-empty tile payloads cannot be parsed until `splot-core` has a
bounded, spec-derived entropy decoder primitive. This should land before tile
syntax or reconstruction so future decoder and encoder work share one small,
tested CDF-update implementation.

## What Changes

- Add Feature ID `AV2-8.2-SYMBOL-DECODER`.
- Add a bounded `splot-core` `SymbolDecoder` over caller-provided tile payload
  bytes alongside the existing range-coder stubs.
- Implement only AV2 § 8.2 primitive operations: `init_symbol(sz)`,
  `read_bool()`, `read_literal(n)`, `read_symbol(cdf)`, CDF update, and
  `exit_symbol()` padding/trailing-bit validation.
- Use generated repository-owned § 9 conversion tables (`Prob_Inc` and
  `Para_Adjustment_List`) and caller-supplied mutable CDF rows.
- Add typed `splot-core` errors for invalid CDF rows, invalid symbol state, and
  symbol-exit conformance failures.
- Update decoder roadmap, decoder support matrix/status, implementation matrix,
  and generated feature/spec status.
- Keep § 8.3 syntax-element CDF selection, Tile/Saved CDF banks,
  `decode_tile()`, tile syntax, reconstruction, runtime hash/Y4M output,
  AVM/dav2d execution, and CLI decode success out of scope.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: add the source-backed AV2 § 8.2 symbol decoder foundation
  while keeping runtime `splot decode`, § 8.3 CDF selection, and tile payload
  decode unsupported.

## Impact

- Code/API: new `crates/splot-core/src/symbol.rs` module, small `splot-core`
  exports, and typed core error variants/kinds.
- Dependencies: no new crate dependencies and no `splot-*` dependency edge.
- Docs/status: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder status,
  `docs/IMPLEMENTATION-MATRIX.toml`, and generated feature/spec status as
  required by repo gates.
- OpenSpec: `openspec/specs/decoder-support/spec.md`.
- Boundary: no AVM/dav2d source, snippets, binaries, submodules, dependencies,
  wrappers, build probes, scripts, CI jobs, runtime process execution, local
  absolute paths, or mandatory reference-tool tests.
