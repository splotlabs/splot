## Why

The encoder mission now needs a reusable AV2 § 8.2 entropy writer before it can
produce any real coded tile bytes. `splot-core::symbol` already provides the
bounded decoder primitive used by decode tests and fuzzing; this change adds the
inverse writer primitive so future encoder tile syntax can prove
`encode -> SymbolDecoder -> original operations` without depending on decoder
crates or external codecs.

## What Changes

- Add a `splot-core` symbol/range encoder primitive tracked by
  `ENC-BITSTREAM-WRITER` and the existing `AV2-8.2-SYMBOL-DECODER` inverse
  surface.
- Support I/O-free writing of `write_bool`, `write_literal`, `write_symbol`,
  and `finish`/`exit_symbol` padding over caller-provided CDF rows.
- Validate CDF rows before encoding and update them with the same AV2 § 8.2.6
  adaptation step used by `SymbolDecoder`.
- Bound both emitted bytes and primitive operation count, so high-skew valid CDF
  rows that produce zero-bit symbols cannot grow the operation log without a
  typed error.
- Prove valid operation streams by decoding the emitted bytes with the in-tree
  `SymbolDecoder`, including boolean/literal/symbol round trips, CDF update
  parity, finalization padding, deterministic byte output, and typed rejection
  of malformed calls.
- Add a bounded fuzz target for symbol encoder operation streams and generated
  CDF rows.
- Remove or supersede the stale `bitio::RangeEncoder` unimplemented stub only if
  doing so is a compatible cleanup; do not change parser error behavior for the
  existing `RangeDecoder` stub in this slice.
- Update the implementation matrix, generated status/coverage docs, writer
  coverage/status docs where applicable, and encoder roadmap/gap-audit notes.

Non-goals:

- No AV2 § 8.3 syntax-element CDF selection, default CDF-bank expansion, tile CDF
  lifecycle, or `decode_tile()`/`encode_tile()` traversal.
- No coefficient tokenization, mode decisions, transforms, quantization, coded
  tile body generation, packets, CLI output, or public `splot encode` success.
- No AVM/dav2d/rav1e/SVT integration, generated external code, unsafe code, new
  dependencies, or thread/scheduler changes.
- No claim that every AV2 entropy-coded syntax element is supported; this is the
  generic § 8.2 writer primitive only.

## Capabilities

### New Capabilities

- `symbol-encoder`: Generic AV2 § 8.2 symbol/range encoder primitive in
  `splot-core`, proven by in-tree decode round trips.

### Modified Capabilities

- `encoder-tools`: Record that `ENC-BITSTREAM-WRITER` includes the § 8.2 generic
  range/symbol writer primitive needed before future coded tile payload work.

## Impact

- Affected code: `crates/splot-core/src/symbol.rs` or sibling symbol-writer
  module/tests, `crates/splot-core/src/write/mod.rs` exports as needed, and the
  stale `crates/splot-core/src/bitio.rs` range-encoder stub if compatible.
- Affected fuzzing: add `fuzz/fuzz_targets/symbol_encoder_bytes.rs` and register
  it in `fuzz/Cargo.toml`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/ENCODER-ROADMAP.md`, `docs/ENCODER-GAP-AUDIT.md`,
  generated status/coverage docs, and writer coverage docs where the xtask
  model includes this primitive.
- APIs: new `splot_core` public API for symbol encoding; no `splot-encode` or
  `splot-cli` behavior change.
- Diagnostics/errors: new typed writer-side errors for malformed CDF rows,
  invalid symbols/literal widths, oversized output bounds, and operation-limit
  exhaustion.
- Dependencies: none.
- Flight note: PR publication is blocked while PR #244 owns the same shared
  generated matrix/status docs; this branch may proceed locally but must rebase
  onto merged `main` and recompute the Flight Manifest before opening a PR.
