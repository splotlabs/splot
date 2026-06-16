## Context

`crates/splot-core/src/symbol.rs` implements the AV2 § 8.2 symbol-decoder
primitive. An independent line-by-line review against the committed spec mirror
(`docs/spec/av2/1.0.0/08-parsing-process.md` § 8.2.1–§ 8.2.6) and the generated
§ 9.2 `Prob_Inc` / `Para_Adjustment_List` tables found the implementation
spec-exact for `init_symbol`, `read_bool`, `read_literal`, `read_symbol`
(arithmetic renormalization + CDF adaptation), and the bit/padding/exit
conformance portion of `exit_symbol`, with no reachable panic, integer
overflow, or out-of-bounds indexing from arbitrary bytes or malformed CDF rows.

The row was `partial` because the founding change deferred § 8.2.4's CDF
copy/averaging to "future tile-CDF-bank work". That work shipped as
`tile-cdf-save-lifecycle-boundary` (`supported`), which now owns the
Tile→Saved→Frame copy/average half. The primitive's remaining scope is
therefore complete; promotion is an evidence + status-honesty task.

## Goals / Non-Goals

Goals:
- Mark the § 8.2 *primitive* `supported` with proof that covers every arity and
  the arithmetic edges, not just N = 2 / N = 4.
- Keep the matrix honest: the promoted row claims only the § 8.2 primitive.

Non-Goals:
- No production change to the symbol decoder (it is already spec-exact).
- No § 8.3 CDF selection, named Tile/Saved CDF banks, § 9.3 default banks,
  `decode_tile()`/traversal, reconstruction, hashing, or output. Those stay in
  `tile-cdf-selection-boundary`, `tile-cdf-save-lifecycle-boundary`,
  `tile-payload-decode`, and the runtime rows.

## Decisions

### Status honesty: what "supported" claims here

The promoted `symbol-decoder` row asserts exactly the § 8.2 primitive
(§ 8.2.2/8.2.3/8.2.5/8.2.6 in full, and the § 8.2.4 exit/trailing-bit/padding
conformance portion). The § 8.2.4 CDF copy/averaging text and the § 8.2.2
"Tile" CDF-array copy are explicitly attributed to
`tile-cdf-save-lifecycle-boundary`. This avoids double-claiming § 8.2.4 and
keeps `check-decoder-conformance-coverage` consistent: the `symbol-and-cdf-process`
coverage group stays `partial` because it also spans § 8.3 selection and § 9.3
banks, which the coverage gate permits (only a `supported` group requires all
linked rows to be `supported`).

### Test design (no encoder available)

Because the repository has no AV2 entropy *encoder*, exact decoded-symbol
vectors cannot be machine-generated. The evidence instead combines:

- **Hand-verifiable extreme vectors for every arity.** `init_symbol` sets
  `SymbolValue = 0x7FFF` for an all-zero payload and `0x0000` for an all-ones
  15-bit payload. A maximal `SymbolValue` breaks the § 8.2.6 threshold loop at
  symbol 0; a zero `SymbolValue` walks to the last symbol N-1 (whose `cur` is 0
  because `Prob_Inc[N-2][N-1] == 0`). This deterministically pins the first and
  last symbol for every N = 2..8 and exercises each `Prob_Inc` row end-to-end.
- **Hand-verified update extremes.** Two `update_cdf` cases with the minimum
  reachable rate (3) and maximum (8) assert the exact post-update row, pinning
  the `>> rate` arithmetic at both shift extremes including the count cap.
- **Deep-negative `SymbolMaxBits`.** Repeated `read_symbol` on a 2-byte payload
  drives `SymbolMaxBits` well below 0; the test asserts in-range symbols, no
  panic, and determinism across two fresh decoders (covering the implicit
  zero-padding `numBits = 0` path).
- **Random-CDF property test.** Random valid rows of arity N = 2..8 (strictly
  increasing cumulative values in `[1, 32767]`, random adaptation-rate index
  and capped count) over arbitrary payloads assert only invariants that the
  spec update provably preserves: decoded symbol `< N`; post-update entries
  remain in `[1, 32767]` with count `<= 32`; determinism; and an
  update-disabled run leaves the row unchanged.

  Note: the property test deliberately does **not** assert strict monotonicity
  after update, because the § 8.2.6 adaptation can legitimately drive two
  adjacent cumulative entries to equality (e.g. `16000` and `16001` at rate 2
  both become `20192`). Asserting strict monotonicity would be incorrect.

## Risks / Trade-offs

- **Promotion on test evidence rather than new code.** Mitigated: the primitive
  is already byte-stream reachable through the `supported` minimal-tier runtime
  (`decode-minimal-tier-runtime-success`) and the `symbol_decoder_bytes` fuzz
  target, so the "no `supported` on unit tests alone" rule is satisfied by
  pre-existing runtime + fuzz evidence; this change strengthens unit/property
  coverage on top of that.
- **CDF validation strictness (resolved).** Earlier `validate_cdf` rejected
  non-strictly-increasing rows, which the § 8.2.6 update can legitimately
  produce (equal adjacent entries) — a latent conformance gap on re-read of an
  adapted row. This is now fixed (the prerequisite landed): `validate_cdf`
  rejects only a strict decrease (`value < cdf[index - 1]`), so equal adjacent
  entries are accepted and decode correctly, proven by the equal-adjacent
  acceptance/decode tests. No follow-up remains.

## Migration

None. No public API or behavior change; tests and status docs only.
