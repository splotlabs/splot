## Context

`docs/DECODER-ROADMAP.md` defines the repository-owned decoded-frame hash
contract:

```text
algorithm_id = "splot-dfh-sha256-v1"
byte_stream_id = "av2-output-samples-v1"
variant_id = "raw_intermediate_output"
```

`splot-recon` already owns validated decoded-frame and plane model types plus
`DecodedFrameHashInput<'_, T>`, which serializes the canonical cropped visible
sample byte stream. The missing piece is the digest result that future decoder
fixtures, CLI hash output, and encoder roundtrip tests can compare without
requiring Y4M output.

This change is still reconstruction infrastructure. It does not read AV2
bitstreams, schedule decode work, reconstruct pixels, select output order, write
Y4M, or invoke local reference decoders. `splot-recon` must remain independent
of `splot-decode`, `splot-cli`, `splot-parallel`, and AVM/dav2d integration.

## Goals / Non-Goals

**Goals:**

- Compute `splot-dfh-sha256-v1` for a caller-supplied
  `DecodedFrame<T>` using the existing `DecodedFrameHashInput` semantics.
- Expose stable identifiers for algorithm, byte stream, and variant.
- Expose a small digest value type that can return raw bytes and lowercase hex.
- Pin digest behavior with self-contained `splot-recon` unit tests covering
  byte-stream identity, padding exclusion, 8-bit and 10-bit sample encoding, and
  lowercase hex formatting.
- Update matrix/docs/OpenSpec so the deterministic-frame-hash row no longer
  overclaims runtime decode, AV2 metadata MD5 verification, film grain, Y4M, or
  AVM/dav2d execution.

**Non-Goals:**

- No CLI `splot decode --output-format hash` success path.
- No byte-consuming decode, tile payload parsing, symbol decoding, or
  reconstruction algorithm.
- No AV2 `METADATA_TYPE_DECODED_FRAME_HASH` MD5 verification.
- No film-grain synthesis or post-film-grain hash variant.
- No output ordering, show-existing, flush output, or reference refresh process.
- No AVM/dav2d source, snippets, binaries, wrappers, scripts, build probes,
  Cargo dependencies, CI jobs, runtime process execution, or mandatory tests.

## Decisions

1. Add digest computation to `crates/splot-recon/src/hash_input.rs`.

   The digest consumes exactly the existing canonical hash-input byte stream.
   Keeping both APIs together prevents divergent byte order, visible-region, or
   bit-depth handling. A separate module was considered, but it would either
   duplicate traversal helpers or expose internals that are currently local to
   `hash_input.rs`.

2. Add a public `DecodedFrameHash` value type.

   The type wraps `[u8; 32]` and exposes:

   - `CONTRACT_ID = "splot.decoded_frame_hash"`;
   - `CONTRACT_VERSION = 1`;
   - `ALGORITHM_ID = "splot-dfh-sha256-v1"`;
   - `BYTE_STREAM_ID = DecodedFrameHashInput::<T>::BYTE_STREAM_ID` by contract;
   - `VARIANT_ID = "raw_intermediate_output"`;
   - `as_bytes() -> &[u8; 32]`;
   - `to_hex() -> String`;
   - `Display` as lowercase hex.

   Returning a concrete value type is clearer for fixture manifests and future
   CLI output than returning a bare byte array or string. A borrowed hex view is
   unnecessary because the value is only 32 bytes and formatting is not a hot
   decode loop.

3. Add `DecodedFrameHashInput::compute_hash() -> DecodedFrameHash`.

   The method streams visible samples into SHA-256 directly instead of calling
   `write_to` through an allocation-backed writer. This avoids row-buffer
   allocation failures and keeps digest computation infallible for already
   validated frame models. `write_to` remains available for tests and future
   tooling that needs raw canonical bytes.

4. Use the existing workspace `sha2` crate.

   `sha2` is already a workspace dependency used by `xtask` and covered by the
   repository's cargo-deny policy. This change adds a direct dependency from
   `splot-recon` to the existing workspace dependency; it does not introduce a
   new third-party crate or any `splot-*` dependency edge. Hand-rolling SHA-256
   was considered and rejected because it would add avoidable cryptographic
   implementation risk and maintenance burden. This dependency-edge decision is
   explicit in the OpenSpec change, agent log, and PR body for maintainer review.

5. Keep `splot-recon` scheduler-free.

   Digest computation is serial over one already materialized frame. Future
   parallel decode or multi-frame hash orchestration belongs in `splot-decode`
   under its context-owned `splot_parallel::WorkerPool`, not in `splot-recon`.

## Risks / Trade-offs

- [Risk] The digest API could be mistaken for runtime decode support. →
  Mitigation: docs and matrix say it hashes caller-supplied decoded frames only;
  CLI hash output, byte-consuming decode, and reconstruction stay unsupported.
- [Risk] `splot-dfh-sha256-v1` could be confused with AV2 decoded-frame-hash
  metadata MD5. → Mitigation: API/docs use the `splot-dfh-sha256-v1` algorithm
  ID and explicitly leave AV2 metadata MD5 verification as future work.
- [Risk] A new direct dependency from `splot-recon` broadens the crate graph. →
  Mitigation: use the existing workspace `sha2` dependency only, run
  `cargo xtask check-dependency-direction`, `cargo machete`, and
  `cargo deny check`, and document the edge in the PR. If maintainer review
  rejects this direct dependency, block this change rather than hand-roll
  SHA-256.
- [Risk] Digest traversal could diverge from `write_to`. → Mitigation: tests
  compare `compute_hash()` to `sha2::Sha256` over bytes emitted by `write_to`
  for representative frames.
