## Context

`splot-recon` owns the source-backed decoded-frame hash input contract for
already materialized `DecodedFrame<T>` values. `DecodedFrameHashInput` serializes
visible Y, U, and V samples according to AV2 § 6.16.13 sample-byte conversion and
the raw intermediate output sample order described by § 7.21.2, then computes
the repository-owned `splot-dfh-sha256-v1` digest over that byte stream.

Existing unit tests cover exact byte output, row/crop/padding exclusion, bit
depth behavior, Y/U/V ordering, and fixed digest examples. The minimal runtime
hash path has its own byte-consuming fuzz target, but that path intentionally
covers only the supported minimal decode tier. This change fuzzes the
source-backed hash API directly without widening runtime decode claims.

## Goals / Non-Goals

**Goals:**

- Add `recon_frame_hash_bytes` to the fuzz crate.
- Build only valid, small `DecodedFrame<u8>` or `DecodedFrame<u16>` values
  through public `splot-recon` constructors.
- Cover 8-bit and 10-bit sample conversion, monochrome and YUV pixel formats,
  aligned crop origins, stride padding, storage padding, and visible-region
  isolation from non-visible storage.
- Exercise `DecodedFrameHashInput::byte_len`, `write_to`, and `compute_hash`
  along with `DecodedFrameHash` display/hex/raw-byte helpers.
- Keep allocations, serialized byte buffers, and failing-writer budgets bounded.

**Non-Goals:**

- Fuzzing AV2 bitstream decode, `DecodeContext`, IVF, Annex B, tile payloads, or
  CLI hash publication.
- Implementing or verifying AV2 decoded-frame-hash metadata MD5 syntax from
  § 5.17.12.
- Applying film grain, output ordering, show-existing-frame/flush behavior,
  reference refresh, or motion-field storage.
- Generating or committing a corpus.
- Filesystem publication, AVM/dav2d/ffmpeg, networking, subprocesses, or new
  dependencies.

## Decisions

1. Fuzz `splot-recon` directly.

   Rationale: `DecodedFrameHashInput` is a library serialization boundary over
   typed decoded frames. Driving it through `DecodeContext` would duplicate the
   existing minimal runtime hash fuzz target and would not vary crop, storage,
   stride, and pixel format broadly.

2. Normalize arbitrary bytes into valid frames.

   Rationale: The target is for hash serialization and digest behavior, not for
   spending most fuzz iterations on constructor errors. The target should align
   non-monochrome crop origins to § 6.4.1 subsampling factors and derive chroma
   geometry with existing `PixelFormat` helpers.

3. Test padding isolation with paired frames.

   Rationale: The hash input contract is visible-sample-only. A paired frame
   with identical visible samples, mutated non-visible padding, and a different
   `OutputIndex` verifies the digest does not accidentally include backing
   padding or output-index metadata.

4. Avoid a direct SHA-256 dependency in the fuzz crate.

   Rationale: `sha2` is already used inside `splot-recon`, and unit tests check
   known digest values. The fuzz target can still assert byte length,
   deterministic serialization, deterministic digest computation, stable
   contract identifiers, hex formatting, and padding isolation without changing
   the fuzz crate dependency graph.

## Risks / Trade-offs

- [Risk] The new row is mistaken for AV2 metadata hash verification.
  Mitigation: name it as `recon`/frame-hash-input fuzz and state that
  § 5.17.12 decoded-frame-hash metadata verification remains out of scope.

- [Risk] Structured generation filters too much input before hash code runs.
  Mitigation: normalize inputs into valid bounded geometry and sample values
  instead of rejecting most byte sequences.

- [Risk] Fuzz buffers grow too large.
  Mitigation: cap visible luma dimensions, crop origins, storage padding, stride
  padding, and failing-writer byte budgets.

- [Risk] The target asserts overly specific digest values for arbitrary inputs.
  Mitigation: assert invariants that must hold for every valid frame rather than
  fixed digest strings.
