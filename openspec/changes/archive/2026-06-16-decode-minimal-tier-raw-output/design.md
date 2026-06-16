## Context

The supported runtime tier currently reconstructs one 64x64 8-bit 4:2:0
closed-loop-key IVF fixture and exposes two artifacts:

- `DecodeContext::decode_hash_report_bytes` computes `splot-dfh-sha256-v1` from
  `splot_recon::DecodedFrameHashInput`.
- `DecodeContext::decode_y4m_bytes` serializes the same frame through
  `splot_recon::Y4mWriter`, then the CLI publishes the complete byte stream
  atomically.

The output-equivalence contract in `docs/DECODER-FULL-CONFORMANCE.md` already
defines future raw output as concatenated canonical visible sample bytes for
each output event, using the same `av2-output-samples-v1` byte policy as the
hash input. The relevant spec anchors are AV2 §7.21.1 and §7.21.2 for output
process/intermediate output, and §6.16.13 for decoded sample byte conversion
(`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-21-1`,
`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-21-2`, and
`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13`).

## Goals / Non-Goals

**Goals:**

- Add a `DecodeContext::decode_raw_bytes` API for the existing minimal runtime
  tier.
- Use `DecodedFrameHashInput::write_to` as the canonical raw sample byte
  serializer, avoiding a second sample-order implementation.
- Add `splot decode --output-format raw <input> -o <output.raw>` for the same
  committed minimal IVF tier.
- Preserve output-file safety: decode and serialize complete raw bytes before
  creating or replacing the requested path; write via same-directory temporary
  file; flush, sync, rename, and cleanup like Y4M.
- Add a runtime raw fuzz target over arbitrary raw bytes and bounded mutations
  of the committed minimal fixture.

**Non-Goals:**

- No broad AV2 decode support, raw Annex B runtime success, multi-frame output,
  show-existing/flush ordering, film grain, post-film-grain raw output,
  metadata MD5 verification, reference refresh semantics, or AVM/dav2d
  integration.
- No new public raw container/header format. Raw output is intentionally
  headerless sample bytes governed by the existing output-equivalence contract.
- No new dependencies or dependency-direction changes.

## Decisions

1. Reuse `DecodedFrameHashInput::write_to` for raw serialization.

   The hash input already owns the canonical `av2-output-samples-v1` visible
   sample byte policy and is fuzzed by `recon_frame_hash_bytes`. Reusing it
   keeps raw output aligned with hashes and avoids another plane/sample loop in
   `splot-decode`.

   Alternative considered: add a separate `RawFrameWriter` in `splot-recon`.
   That would be useful once raw output grows variants or metadata, but it is
   unnecessary for this one-frame minimal tier and would duplicate the current
   byte policy.

2. Add a separate raw runtime adapter instead of folding raw into Y4M.

   Raw and Y4M have different contracts: raw output has no header/timebase and
   remains valid for zero IVF timebase, while Y4M requires a nonzero frame-rate
   token. A small `runtime_raw.rs` keeps those preflight rules separate.

3. Generalize CLI file publication helpers to output artifacts.

   The Y4M publication path already implements the required transactional file
   behavior. Raw should reuse the same pattern with stable operation names that
   say `raw` for diagnostics. The CLI stays thin: it selects the output mode,
   calls the library API, and publishes a completed byte buffer.

4. Add a `decode_runtime_raw_bytes` fuzz target.

   `recon_frame_hash_bytes` covers the serializer over structured frames, but
   the runtime API is byte-consuming and should have its own no-panic target
   matching the hash/Y4M runtime fuzz pattern.

## Risks / Trade-offs

- [Risk] Raw output could be mistaken for broad decoder support.
  -> Mitigation: create a dedicated support row whose notes explicitly limit
  support to the committed minimal IVF tier and keep broad output coverage
  partial.

- [Risk] Raw and hash sample bytes could drift.
  -> Mitigation: raw uses `DecodedFrameHashInput::write_to`; tests compare raw
  bytes against the expected sample payload and hash digest path.

- [Risk] Output publication helper changes could regress Y4M.
  -> Mitigation: preserve existing Y4M operation names and tests, add raw tests
  around the same no-touch and replacement behavior, and run the full CLI Y4M
  test file.

- [Risk] Adding a fuzz target without registration could be skipped by CI.
  -> Mitigation: update `fuzz/Cargo.toml`, `.github/workflows/ci.yml` corpus
  seeding, decoder support matrix, and run `cargo xtask check-fuzz-targets`.
