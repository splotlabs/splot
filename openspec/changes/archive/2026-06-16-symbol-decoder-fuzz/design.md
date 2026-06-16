## Context

`splot_core::symbol::SymbolDecoder` is the public AV2 §8.2 arithmetic symbol
decoder foundation. Unit and property tests already cover known vectors,
malformed CDF rows, and broad random CDF rows, while cargo-fuzz currently starts
at OBU/container planning and runtime output surfaces. Phase 9 needs the symbol
decoder's byte-consuming API to have its own bounded fuzz target so future tile
payload work has a stable no-panic proof at the primitive layer.

## Goals / Non-Goals

**Goals:**

- Add a cargo-fuzz target named `symbol_decoder_bytes`.
- Use only the public `splot-core` symbol API.
- Bound payload length, operation count, CDF row length, and generated row
  values before calling the decoder.
- Exercise `read_bool`, `read_literal`, `read_symbol`, and `exit_symbol`.
- Treat all typed `splot_core::Error` returns as acceptable fuzz outcomes.
- Assert only local invariants that follow from successful public API returns.

**Non-Goals:**

- No §8.3 syntax-element CDF selection.
- No default §9.3 Tile/Saved CDF bank initialization.
- No tile payload traversal, partition decoding, block syntax, reconstruction,
  runtime hashes, runtime Y4M, or reference refresh.
- No AVM/dav2d/ffmpeg invocation, filesystem I/O, network I/O, subprocesses,
  dependency changes, public API changes, or CI target hardcoding.

## Decisions

1. Fuzz `splot_core::symbol::SymbolDecoder` directly.
   - Rationale: the API is already public and byte-consuming, so the target can
     be self-contained in the existing `fuzz` crate without exposing
     `splot-decode` internals.
   - Alternative considered: fuzz crate-private tile-payload CDF selectors.
     That would require exposing internal APIs or using test-only hooks, which
     is not needed for the §8.2 primitive proof.

2. Use an operation stream rather than a single fixed decode path.
   - Rationale: a compact byte grammar can mix bool reads, literal reads,
     symbol reads, malformed CDF validation, and finish paths within a single
     target.
   - Alternative considered: one target per symbol operation. A single target is
     simpler and keeps CI smoke growth modest.

3. Generate valid and invalid CDF rows inside the harness.
   - Rationale: valid rows reach arithmetic update paths; malformed rows keep
     validation and typed error paths covered. Both are bounded by the AV2 §8.2
     supported row arity in the implementation.
   - Alternative considered: only use generated §9.3 rows. That would overstate
     default CDF-bank coverage and would not cover malformed caller-supplied
     rows as directly.

4. Register the target in `fuzz/Cargo.toml` and let CI enumerate targets.
   - Rationale: existing CI creates corpus directories from `cargo fuzz list`;
     target registration is enough for smoke execution. Corpus seeding can add
     tiny symbol-specific seeds without hardcoding the executable list.

## Risks / Trade-offs

- [Risk] Harness assertions become false-positive crashers for valid but
  unexpected decoder behavior. -> Mitigation: assert only public invariants such
  as symbol values staying inside CDF arity and successful summary positions
  being byte-aligned and within payload bounds.
- [Risk] Excessive operation counts slow CI fuzz smoke. -> Mitigation: clamp raw
  payload bytes and operation count to small constants.
- [Risk] Docs imply full §8.3 or tile-payload support. -> Mitigation: matrix and
  support notes explicitly exclude CDF selection, CDF banks, tile traversal, and
  runtime decode behavior.
- [Risk] Generated CDF rows accidentally use invalid probability shapes for the
  valid path. -> Mitigation: construct monotonic cumulative rows with bounded
  rates/counts, and keep a separate malformed-row mode for validation coverage.
