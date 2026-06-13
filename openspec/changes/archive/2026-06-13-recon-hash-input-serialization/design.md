## Context

The decoder roadmap defines `splot-dfh-sha256-v1` as SHA-256 over canonical
decoded output sample bytes. `splot-recon` already owns validated
`DecodedFrame<T>` and `Plane<T>` types with visible-row iteration, but there is
no source API that produces the canonical sample-byte stream. A digest API would
require wiring a hashing dependency into a crate that currently has no
dependencies, so the next safe slice is the dependency-free serialization layer.

## Goals / Non-Goals

**Goals:**

- Add a `splot-recon` API that serializes visible decoded output samples from
  `DecodedFrame<T>` in the roadmap's `av2-output-samples-v1` order.
- Follow AV2 § 6.16.13 sample-byte conversion and § 7.21.2 visible output
  arrays for caller-supplied raw pre-film-grain output frames.
- Exclude stride padding, backing allocation padding, output metadata, OBU
  bytes, container metadata, and decoded-frame-hash metadata.
- Keep the API dependency-free, no-panic, and reusable by future decode and
  encoder roundtrip code.
- Update decoder docs, support matrix, feature tracking, generated status, and
  OpenSpec artifacts.

**Non-Goals:**

- No SHA-256 digest computation or digest result type.
- No AV2 metadata MD5 verification for `metadata_decoded_frame_hash()`.
- No byte-consuming decode, resource-limit diagnostic emission, Y4M output, or
  film-grain synthesis.
- No output ordering, show-existing handling, implicit output, flush output, or
  frame-buffer output process implementation.
- No AVM/dav2d integration, fixtures, scripts, wrappers, CI hooks, or local path
  metadata.
- No new dependencies or crate dependency graph changes.

## Decisions

1. The serializer lives in `crates/splot-recon/src/hash_input.rs`.

   `splot-recon` already owns decoded frame and plane types, so it is the right
   dependency-free home for the canonical byte stream. Future `splot-decode`
   code can feed the bytes into a digest without making `splot-recon` depend on
   the decode driver or CLI.

2. The public API is `DecodedFrameHashInput<'a, T>`.

   The wrapper borrows a `DecodedFrame<T>` and exposes the contract identifiers
   `BYTE_STREAM_ID = "av2-output-samples-v1"` and
   `VARIANT_ID = "raw_intermediate_output"`. It is not named as a SHA-256 hash
   type because this slice produces digest input only. A free function was
   considered, but a wrapper gives the byte-stream identifiers and checked
   `byte_len()` a stable home without exposing plane-only serialization.

3. The API writes to a caller-provided `std::io::Write`.

   `write_to` avoids allocating a full byte buffer and lets future callers stream
   directly into a digest adapter or test buffer. It returns `std::io::Result<()>`
   because writer failure is an output-sink problem, not a reconstruction model
   invariant. A writer error may leave a partial sink.

4. Byte length is computed separately with checked arithmetic.

   `byte_len()` returns `splot-recon::Result<usize>` and reports
   `ReconError::ArithmeticOverflow` if visible sample counts or byte totals
   overflow. Normal constructed frames are already validated for plane presence,
   sample range, and visible geometry.

5. Serialization follows the existing validated frame shape.

   The writer emits the caller-provided frame's modeled visible rows: Y, then U,
   then V when those planes are present. Monochrome frames carry only Y by
   construction. 8-bit output writes one byte per sample even if the backing
   Rust sample type is `u16`; 10-bit output writes little-endian two-byte values.

## Risks / Trade-offs

- [Risk] A serializer could be mistaken for a completed frame hash. →
  Mitigation: docs, matrix notes, and API names explicitly say this is hash
  input serialization only and no digest is computed.
- [Risk] `std::io::Write` exposes partial-write failure behavior. →
  Mitigation: document that `write_to` propagates writer errors and may leave a
  partially written sink.
- [Risk] Future film-grain output needs a separate variant. → Mitigation:
  `VARIANT_ID` is fixed to `raw_intermediate_output`; post-film-grain output
  must be a separate explicitly named future API.
- [Risk] A caller might pass frames in decode order instead of output order. →
  Mitigation: this API only serializes one already materialized frame; output
  ordering remains a future decode-driver responsibility.
