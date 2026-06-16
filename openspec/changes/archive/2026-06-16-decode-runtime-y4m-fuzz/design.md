## Context

The fuzz crate currently covers parser/validator entry points, byte-stream
planning, the minimal runtime hash API, pure Y4M serialization, and
source-backed intra prediction primitives. The existing runtime Y4M path is a
separate byte-consuming boundary: `DecodeContext::decode_y4m_bytes` parses raw
Annex B or IVF bytes, applies the minimal runtime tier, reconstructs the
current single-frame fixture shape, serializes a complete Y4M stream through
`splot-recon::Y4mWriter`, and writes that complete stream to a caller-provided
writer.

This change adds no new decode behavior. It adds no-panic fuzz coverage around
the existing public runtime Y4M byte API and keeps CLI filesystem publication
out of scope.

## Goals / Non-Goals

**Goals:**

- Add `decode_runtime_y4m_bytes` to the fuzz crate.
- Exercise `DecodeContext::decode_y4m_bytes` with finite `DecodeLimits`.
- Feed both arbitrary bytes and bounded mutations of the committed
  `syn-flat-intra-64x64-minimal.ivf` fixture.
- Exercise successful in-memory Y4M output, typed unsupported/malformed/limit
  errors, and typed caller-writer output errors.
- Assert only stable current-tier success shape: one 64x64 8-bit 4:2:0 Y4M
  frame payload from the raw-intermediate output path.
- Update implementation matrix, decoder support matrix, testing docs, generated
  status docs, and decoder conformance coverage metadata.

**Non-Goals:**

- Broad runtime decode fuzzing beyond the current minimal Y4M byte path.
- CLI temp-file publication, filesystem I/O, raw output, hash report output, or
  post-film-grain output.
- AVM, dav2d, ffmpeg, network, subprocesses, large corpora, or new
  dependencies.
- New public APIs, diagnostics, syntax support, or runtime conformance claims.

## Decisions

1. Fuzz the public `DecodeContext::decode_y4m_bytes` API directly.

   Rationale: The target is the existing byte-consuming runtime Y4M API. The
   pure writer is already fuzzed by `recon_y4m_output_bytes`, and the CLI file
   publication path is covered by integration tests rather than cargo-fuzz.

2. Reuse the runtime hash fuzz input pattern.

   Rationale: Arbitrary raw bytes keep malformed/parser rejection paths live,
   while bounded mutations of the committed minimal IVF fixture keep execution
   near the current runtime Y4M success path often enough to exercise tile,
   reconstruction, output-limit, and writer handoff behavior.

3. Use a single-thread `DecodeContext` and finite limits.

   Rationale: Thread-policy determinism is already tested at unit/integration
   level. Fuzz iterations should be cheap, deterministic, and bounded by
   explicit caps for input bytes, OBU count, IVF frame records, frames, tile
   count, tile payload bytes, partition steps, decoded-frame bytes, reference
   store bytes, and output bytes.

4. Keep writer behavior in memory.

   Rationale: `decode_y4m_bytes` writes to a generic writer, so the fuzz target
   can use a bounded `Vec<u8>` writer for success shape checks and a deterministic
   failing writer for typed `decode/output-error` paths. No filesystem behavior
   is needed to fuzz this library boundary.

5. Assert only stable output shape on success.

   Rationale: Mutations can affect IVF timing fields while still preserving the
   current minimal tier. The fuzzer should validate structural Y4M properties,
   visible payload length, and neutral sample bytes without freezing incidental
   header text beyond the current output format contract.

## Risks / Trade-offs

- [Risk] The new row is mistaken for broad runtime Y4M or full decode support.
  Mitigation: use a distinct `CONF-DECODE-RUNTIME-Y4M-FUZZ` Feature ID and keep
  notes explicit that the target fuzzes only the current minimal Y4M byte API.

- [Risk] Fuzzing `decode_y4m_bytes` duplicates the runtime hash fuzz path.
  Mitigation: keep the target Y4M-specific by exercising caller writer behavior,
  output byte limits, IVF timebase handling, and Y4M stream shape.

- [Risk] Output allocations grow under hostile input.
  Mitigation: cap input length and all decode/output limits; the current runtime
  path checks the complete Y4M byte budget before caller-visible writes.

- [Risk] A failing writer could hide success assertions.
  Mitigation: select explicit writer modes so the success writer validates the
  resulting bytes and the failing writer only checks typed error return.
