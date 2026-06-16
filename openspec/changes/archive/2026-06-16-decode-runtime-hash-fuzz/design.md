## Context

The fuzz crate already covers parser, validator, IVF, container auto-detect,
and byte-planning surfaces through five cargo-fuzz targets. The decode-facing
target stops at `DecodeContext::plan_bytes`, so arbitrary byte fuzzing does not
exercise the minimal runtime hash API.

`DecodeContext::decode_hash_report_bytes` now reaches the byte planner, the
minimal tile frontier, supported-subset CDF lifecycle, minimal reconstruction,
and deterministic hash-report serialization for the committed
`syn-flat-intra-64x64-minimal.ivf` fixture. This change adds no-panic fuzz
coverage around that byte-consuming runtime path without broadening AV2 decode
support.

## Goals / Non-Goals

**Goals:**

- Add a self-contained cargo-fuzz target named `decode_runtime_hash_bytes`.
- Exercise `DecodeContext::decode_hash_report_bytes` with finite decode limits.
- Feed both arbitrary bytes and bounded mutations of the committed minimal IVF
  fixture.
- Assert that successful runtime outputs preserve the hash-report shape for the
  current minimal tier.
- Update conformance tracking, decoder support, testing docs, and decoder
  conformance coverage metadata.

**Non-Goals:**

- Full runtime decode fuzzing beyond the minimal hash path.
- Y4M/raw output fuzzing.
- AVM, dav2d, ffmpeg, filesystem I/O, network I/O, or external fixture
  invocation.
- New fuzz dependencies, runtime dependencies, public APIs, diagnostics, or
  spec-derived syntax support.
- Committing a large seed corpus.

## Decisions

1. Keep the fuzz target in the existing `fuzz` crate.

   Rationale: CI and `cargo xtask fuzz` already enumerate cargo-fuzz targets
   from this crate. Adding the target there preserves the existing no-panic
   enforcement path and avoids workflow edits.

2. Use one deterministic single-thread `DecodeContext`.

   Rationale: The runtime hash path does not need scheduler variance for this
   no-panic target. A process-global `OnceLock` context matches the existing
   `decode_plan_bytes` target and keeps fuzzer iterations cheap.

3. Provide two input modes.

   Rationale: Pure arbitrary bytes stress parser and planner rejection paths,
   while bounded fixture mutations keep the fuzzer near the current successful
   runtime tier often enough to exercise tile, CDF, reconstruction, and hash
   paths.

4. Assert only stable minimal-tier success structure.

   Rationale: The fuzzer must not encode broad AV2 expectations. On `Ok`, it
   checks the report contract id/version, one 64x64 8-bit 4:2:0 frame, and the
   presence of a SHA-256 frame hash. On `Err`, a typed `DecodeError` return is
   sufficient.

5. Keep all limits finite and small.

   Rationale: Fuzzing is a hostile-input resource boundary. The target should
   set explicit caps for input bytes, OBU count, IVF frame records, decoded
   frames, tile counts, tile payload bytes, tile partition steps, decoded frame
   bytes, and output bytes.

## Risks / Trade-offs

- [Risk] The new target is mistaken for broad runtime decode support.
  Mitigation: create a distinct conformance Feature ID and decoder support row
  that says it fuzzes only the current minimal hash API.

- [Risk] Fixture mutation masks arbitrary rejection paths.
  Mitigation: keep an explicit raw-arbitrary mode selected directly from fuzz
  input.

- [Risk] Fuzz iterations become too expensive for CI smoke.
  Mitigation: cap limits, reuse a single context, bound fixture mutations, and
  avoid external processes or filesystem access.

- [Risk] Success assertions become brittle if the minimal fixture contract
  legitimately changes.
  Mitigation: assert only the stable public report shape already owned by
  `minimal-intra-8bit420-hash-v1`, not exact digest text or implementation
  internals.
