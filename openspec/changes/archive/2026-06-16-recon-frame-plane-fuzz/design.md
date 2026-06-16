## Context

`splot-recon` owns the immutable decoded output model used by later hash, Y4M,
workspace, and reference-store layers: `BitDepth`, `PixelFormat`, `Plane<T>`,
`FramePlanes<T>`, `DecodedFrameInfo`, `DecodedFrame<T>`, borrowed
`PlaneRef`/`FrameRef` views, and explicit `SharedFrame` sharing. Existing unit
tests cover many edge cases, while downstream fuzz targets build valid frames
only as setup for serialization-specific APIs.

This change adds fuzz coverage for the model boundary itself. It remains
source-backed infrastructure coverage only and does not parse AV2 bitstreams or
claim new reconstruction behavior.

## Goals / Non-Goals

**Goals:**

- Add `recon_frame_plane_types_bytes` to the fuzz crate.
- Drive public `splot-recon` frame/plane runtime type APIs using bounded
  arbitrary inputs.
- Cover both accepted and rejected constructor paths for bit depth, chroma
  format, geometry, crop alignment, stride, backing length, plane presence,
  visible size, sample storage type, and sample range.
- Exercise visible-row slicing, borrowed `PlaneRef`/`FrameRef` access, and
  explicit `SharedFrame::share` handle counting on valid frames.
- Keep allocations and operation counts bounded for CI fuzz smoke.

**Non-Goals:**

- Parsing AV2 bitstreams, invoking `splot-decode`, or changing CLI behavior.
- Implementing reconstruction, output scheduling, reference refresh, film grain,
  metadata MD5 verification, raw/Y4M/hash runtime behavior, or resource
  diagnostics.
- Using AVM/dav2d/ffmpeg, filesystem I/O, network I/O, subprocesses, or new
  dependencies.
- Copying AV2 spec text or adding new AV2 semantics beyond exercising existing
  AV2-derived public constructors.

## Decisions

1. Fuzz the frame/plane model as a separate target.

   Rationale: `recon_frame_hash_bytes` and `recon_y4m_output_bytes` already
   generate valid frames, but their assertions are about serialization. A
   separate target can intentionally generate malformed plane sets and geometry
   without coupling those negative cases to downstream serializers.

2. Normalize input into a small model plus targeted mutations.

   Rationale: Valid frame construction needs related Y/U/V geometry. The target
   should first derive a coherent bounded model, then selectively mutate stride,
   backing length, chroma presence, visible size, sample range, and sample type
   to cover typed error paths.

3. Check typed error categories and public invariants, not display strings.

   Rationale: `ReconError` variants and public accessor behavior are the stable
   API contract. Human-readable messages are presentation detail.

4. Keep AV2 claims limited to existing public constructor facts.

   Rationale: `BitDepth::from_av2_bit_depth_idc`,
   `PixelFormat::from_av2_chroma_format_idc`, crop alignment, and chroma visible
   sizes already cite AV2 § 6.4.1 / § 7.21.2 in code and matrix rows. The fuzz
   target supplies robustness evidence for those APIs; it does not expand
   decoder conformance.

## Risks / Trade-offs

- [Risk] The new row is mistaken for byte-consuming decode or reconstruction
  support. Mitigation: name it as `recon` frame/plane model fuzzing and state
  that runtime decode, reconstruction, output scheduling, reference refresh, and
  resource diagnostics remain out of scope.

- [Risk] Invalid cases dominate and valid accessor paths get little coverage.
  Mitigation: derive a valid base model first, then run both valid-frame checks
  and a bounded set of targeted invalid mutations.

- [Risk] Memory use grows with arbitrary dimensions. Mitigation: cap luma
  dimensions, crop padding, stride padding, and generated sample vectors to
  small fixed maxima.
