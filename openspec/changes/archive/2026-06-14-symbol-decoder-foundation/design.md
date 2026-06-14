## Context

`splot-core` currently has direct AV2 descriptor readers in `bitio.rs` and a
stub `RangeDecoder` that returns `Error::Unimplemented`. The decoder is
otherwise plan-only: `splot-decode` can bound and plan raw Annex B/IVF sources,
but it does not traverse tile payload syntax or reconstruct pixels.

AV2 § 8.2 defines a tile-bounded symbol decoder used by `decode_tile()` and
future CDF-coded syntax. That primitive is needed before constrained intra tile
syntax can land, but it should be implemented independently from tile traversal
so it can be tested with small synthetic byte/CDF cases and later reused by the
encoder-facing reconstruction path.

## Goals / Non-Goals

**Goals:**

- Add a new `crates/splot-core/src/symbol.rs` module for AV2 § 8.2 symbol
  decoding over a caller-provided tile-data byte slice.
- Implement `init_symbol(sz)`, `read_bool()`, `read_literal(n)`,
  `read_symbol(cdf)`, optional CDF update, and `exit_symbol()` validation.
- Use generated `splot-core` § 9 conversion tables for `Prob_Inc` and
  `Para_Adjustment_List`.
- Validate mutable CDF rows before indexing or updating them.
- Return typed `splot-core::Error` variants for invalid CDF rows, invalid symbol
  decoder state, and exit-padding/trailing-bit failures.
- Keep the implementation serial, deterministic, allocation-free on the hot
  path, and independent of `splot-decode`, `splot-recon`, and `splot-cli`.

**Non-Goals:**

- No `decode_tile()` or § 5.20.2-§ 5.20.10 block syntax.
- No § 8.3 syntax-element CDF selection.
- No Tile/Saved CDF bank structs, default-CDF initialization, copyback,
  averaging, frame-context save/load, or `frame_end_update_cdf()`.
- No runtime `splot decode` success path, deterministic hash output, Y4M output,
  reconstruction, output scheduling, reference refresh, or film-grain synthesis.
- No `RangeEncoder` implementation.
- No AVM/dav2d source inspection requirement, runs, wrappers, scripts, build
  probes, Cargo dependencies, CI jobs, runtime process execution, local absolute
  paths, or mandatory reference-tool tests.

## Decisions

1. Add `symbol.rs` instead of growing `bitio.rs`.

   `bitio.rs` already owns direct bitstream descriptors and is close to the
   repository's 1000-line soft budget. A separate module keeps the § 8.2 state
   machine testable and prevents entropy-decoder complexity from obscuring the
   direct descriptor readers.

2. Keep the primitive in `splot-core`.

   The symbol decoder is AV2 syntax/parsing infrastructure and depends only on
   core bit reading, offsets, errors, and generated tables. `splot-decode` will
   orchestrate future tile work through `DecodeContext` and `WorkerPool`, but
   the primitive itself must remain scheduler-free and dependency-direction
   neutral.

3. Use caller-supplied CDF rows.

   A full AV2 tile CDF context requires § 8.3 syntax-element selection, CDF bank
   initialization, tile-local copies, frame-context save/load, and averaging.
   This change intentionally exposes a row-oriented primitive such as
   `read_symbol(&mut [i32])`, matching the generated CDF table value type, and
   validates the row shape before use. Full CDF context ownership remains future
   work.

4. Model `SymbolMaxBits` as signed state.

   AV2 § 8.2.2 explicitly permits negative `SymbolMaxBits` to represent implicit
   zero padding, and § 8.2.4 requires `SymbolMaxBits >= -14` at exit. The
   implementation will use signed arithmetic and avoid `sz * 8` overflow by
   deriving `numBits` and `SymbolMaxBits` with widened or branch-based checked
   arithmetic.

5. Validate exit without consuming external state.

   `finish()` / `exit_symbol()` will validate the trailing one bit and zero
   padding inside the tile-data slice based on the decoder's current bit
   position and `SymbolMaxBits`. It returns a summary for tests/future callers
   but does not update frame CDF banks or output diagnostics directly.

6. Leave runtime decode behavior unchanged.

   `splot decode` still plans raw bytes and emits structured unsupported
   diagnostics after planning. The decoder-support matrix row becomes `partial`
   because § 8.2 primitives exist while § 8.3 CDF selection and tile decode do
   not.

## Risks / Trade-offs

- [Risk] A generic symbol decoder could be mistaken for tile payload support. ->
  Mitigation: keep `symbol-decoder` `partial`, leave `tile-payload-decode`
  `todo`, and document all runtime decode non-goals in roadmap/matrix/PR text.
- [Risk] Off-by-one errors in `SymbolMaxBits` or exit padding. -> Mitigation:
  add direct unit tests for `sz` boundaries, implicit padding, `-14` vs `-15`,
  trailing-one validation, and zero-padding validation.
- [Risk] Invalid CDF rows can cause unchecked indexing. -> Mitigation: validate
  CDF length, monotonic cumulative entries, probability range, adaptation-rate
  index, and count range before using generated tables.
- [Risk] Future encoder APIs may need rate estimation and snapshot/restore. ->
  Mitigation: keep this primitive small and row-oriented; do not expose a
  public tile-CDF-bank API that would constrain future RDO design.
