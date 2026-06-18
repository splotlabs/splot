## Context

`crates/splot-core/src/symbol.rs` implements the AV2 v1.0.0 § 8.2 symbol
decoder primitive over a bounded tile-payload byte slice:
`init_symbol(sz)`, `read_bool()`, `read_literal(n)`, `read_symbol(cdf)`, CDF
adaptation, checkpoints, and `exit_symbol()` padding validation. The decoder is
already runtime/fuzz exercised and marked supported for the generic § 8.2
primitive.

The encoder mission cannot produce real § 5.20 coded tile bytes until the
inverse primitive exists. Future tile syntax must be able to write booleans,
literals, and CDF symbols to an owned tile payload, then prove the emitted bytes
decode back through the existing `SymbolDecoder`. This change is only that
generic § 8.2 primitive; § 8.3 CDF-row selection and tile traversal remain
separate work.

This branch was created from `origin/main` at squash merge
`e1eff50a07138e2ed276e01b8747bf0f802cf1bf`. At proposal time, open PR #244
(`codex/decode-coeff-loop-foundation`) owns shared generated matrix/status docs,
so this change cannot open a PR until that overlap is removed or #244 lands.

## Goals / Non-Goals

**Goals:**

- Provide a public, I/O-free `splot-core` AV2 § 8.2 symbol encoder primitive.
- Encode `write_bool`, `write_literal`, and `write_symbol` operation streams so
  `SymbolDecoder` recovers the exact values with matching CDF updates.
- Finalize tile-payload bytes with `exit_symbol()`-valid trailing and padding
  bits.
- Bound output growth, primitive operation count, allocation, literal width,
  symbol arity, and CDF row shape with typed writer errors.
- Add unit/property/fuzz evidence and update the matrix/status/docs proof.

**Non-Goals:**

- No § 8.3 syntax-element CDF selection or default-bank expansion.
- No Tile/Saved/Frame CDF lifecycle changes.
- No tile traversal, coefficient tokenization, transforms, quantization, mode
  decisions, packets, CLI output, or public encoder success path.
- No external codec integration, generated external tables, unsafe code,
  dependencies, or scheduler/threading changes.

## Decisions

### New sibling module, shared CDF helpers

Add a new `crates/splot-core/src/symbol_encoder.rs` module and export it from
`splot-core`. Keep the existing decoder API in `symbol.rs` to avoid a broad
module move. Factor the decoder's CDF-shape validation and § 8.2.6 adaptation
step into crate-private helpers so encoder and decoder use the same generated
§ 9.2 `Prob_Inc` / `Para_Adjustment_List` tables and cannot drift.

Alternative considered: put `SymbolEncoder` directly in `symbol.rs`. That keeps
private helper access simple but risks pushing one file toward the repository
source-line budget and makes the decoder module harder to review. A sibling
module with narrow crate-private helper exposure is cleaner.

### Writer-side errors stay in the writer family

`SymbolEncoder` returns `write::WriteError` / `WriteResult`, not parser
`Error`. Invalid CDF rows may reuse the existing `SymbolCdfErrorKind` as a
shared shape description, but encode-time failures such as a symbol outside the
row arity, use-after-finish, or output-limit exhaustion are writer errors.

Alternative considered: add `Error::InvalidSymbolEncoderState` to
`splot-core::error`. That would mix encode-side programming failures into the
parser/conformance error family, which existing writer code deliberately avoids.

### Encode proof is decode round trip, not external reference equality

The implementation must prove each accepted operation stream by feeding the
finished payload to `SymbolDecoder` with the same operation sequence and CDF
rows. The assertions are:

- decoded booleans/literals/symbols equal the encoded values;
- decoder and encoder CDF rows are byte-for-byte equal after each symbol when
  updates are enabled, and unchanged when disabled;
- `finish()` produces bytes accepted by `SymbolDecoder::finish()`;
- the same operation stream produces byte-identical payload bytes.

This is the right proof for the generic primitive: AV2 specifies the decoder
process, and an encoder may choose any valid bitstream that decodes to the same
symbols. No AVM byte-equality claim is made.

### Bounded owned output and operation log

`SymbolEncoder` records primitive range steps and finalizes them into owned
tile-payload bytes. Its configuration accepts caller-supplied output byte and
operation-count limits. Every accepted operation checks both limits before
mutating committed state or caller CDF rows, and fallible reservations map to
typed writer errors. The operation limit is required because valid high-skew CDF
rows can produce zero-bit symbols: emitted bytes alone do not bound memory or
finalization cost. `finish(self)` consumes the encoder and returns an owned
output summary plus bytes, matching `BitWriter::into_bytes` style; a failed
write before finish leaves prior committed state valid.

Future tile/payload writers can embed these bytes into § 5.20.1 tile-group
framing without retaining borrowed state. This is not a media frame copy
boundary and does not require a zero-copy pixel materialization marker.

### Stale `bitio::RangeEncoder` stub handling

The old `bitio::RangeEncoder::new() -> Error::Unimplemented` stub predates the
real `splot-core::symbol::SymbolDecoder`. This change may either remove the
stub if no downstream code depends on it, or leave it as a deprecated
compatibility alias that points readers to `symbol_encoder::SymbolEncoder`.
The `RangeDecoder` stub is not in scope because the real decoder is already
`symbol::SymbolDecoder` and parser error behavior should not churn in this PR.

### Flight manifest

- Change ID: `range-encoder-complete`
- Feature IDs: `ENC-BITSTREAM-WRITER`, `AV2-8.2-SYMBOL-DECODER` inverse evidence
- Base commit: `e1eff50a07138e2ed276e01b8747bf0f802cf1bf`
- Depends on merged changes: `encoder-program-contract`,
  `encoder-recon-dependency`, `encoder-frame-input-views`,
  `encoder-context-state-machine`
- Exact files/directories owned by this PR:
  `crates/splot-core/src/symbol.rs`, `crates/splot-core/src/symbol_encoder.rs`,
  `crates/splot-core/src/symbol_encoder_tests.rs`,
  `crates/splot-core/src/symbol_encoder_proptests.rs`,
  `crates/splot-core/src/write/error.rs`, `crates/splot-core/src/lib.rs`,
  `crates/splot-core/src/write/mod.rs` if exporting through writer docs,
  `crates/splot-core/src/bitio.rs` only for stale stub cleanup,
  `fuzz/fuzz_targets/symbol_encoder_bytes.rs`, `fuzz/Cargo.toml`,
  `docs/IMPLEMENTATION-MATRIX.toml`, generated feature/spec/status docs,
  `docs/ENCODER-ROADMAP.md`, `docs/ENCODER-GAP-AUDIT.md`,
  `docs/spec-coverage-writer.md` if regenerated by writer coverage, and
  `openspec/changes/range-encoder-complete/**`
- Exact files/directories forbidden to this PR:
  `crates/splot-encode/**`, `crates/splot-decode/**`,
  `crates/splot-recon/**`, `crates/splot-validate/**`, `crates/splot-cli/**`,
  workspace manifests, `Cargo.lock`, external-reference docs except status
  citations, AV2 spec mirror files, and any active sibling PR files
- Public APIs/types owned: `SymbolEncoder`, `SymbolEncoderConfig`,
  `SymbolEncoderOutput` or equivalent summary, writer-side symbol error variants
- Matrix rows owned: `ENC-BITSTREAM-WRITER` note/test additions; no decoder row
  promotion unless the matrix schema requires inverse proof references
- Generated files owned: feature/status/spec/writer coverage docs regenerated
  from this branch after #244 is no longer overlapping
- Open sibling PRs audited: #244 `codex/decode-coeff-loop-foundation`
- Changed-file intersection with sibling PR #244: currently shared docs overlap
  in `docs/IMPLEMENTATION-MATRIX.toml`, `docs/FEATURE-STATUS.md`, and
  `docs/SPEC-COVERAGE.md`
- Semantic overlap with sibling PR #244: none in code/API; generated status
  documents overlap operationally
- Can build/test/merge directly onto main without another open PR: no while #244
  remains open with shared generated-doc ownership; yes after #244 lands and
  this branch rebases/regenerates

## Risks / Trade-offs

- **Range-coder inverse mistakes** -> Mitigation: every accepted operation stream
  is decoded back through `SymbolDecoder`; property tests and fuzzing compare
  symbols, CDF rows, final padding, and deterministic bytes.
- **CDF helper refactor regresses decoding** -> Mitigation: keep behavior
  byte-for-byte equivalent and run existing `symbol_decoder_bytes` fuzz target,
  symbol unit/property tests, and decoder tile-payload tests.
- **Output growth, zero-bit symbol loops, or allocation pressure** -> Mitigation:
  require explicit output byte and operation-count limits, use fallible
  reservations, and test exhaustion before mutation.
- **Generated-doc conflict with PR #244** -> Mitigation: do not open the PR until
  #244 is merged or updated to remove overlap; after that, rebase/merge main and
  regenerate all affected status docs.
- **Overclaiming entropy coverage** -> Mitigation: spec/matrix language says
  generic § 8.2 primitive only; § 8.3 selection, CDF banks, tile traversal, and
  coded tile body work stay partial/future.

## Migration Plan

1. Land the new symbol encoder API and tests without changing existing decoder
   call sites.
2. If the stale `bitio::RangeEncoder` stub is removed or deprecated, adjust only
   direct tests/docs that referenced the stub as an unimplemented placeholder.
3. Future encoder tile syntax work consumes `SymbolEncoder` directly from
   `splot-core`; no dependency-graph change is required.
