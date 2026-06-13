## Context

`splot-recon` currently exists only as a scaffold, while the decoder roadmap
and support spec already define the semantic contract for decoded output
frames and planes. The next decoder mission slice needs the first runtime model
types before decode, frame hashing, Y4M, or encoder roundtrip code can safely
share output-frame storage.

This change is deliberately narrow. It implements AV2-derived output model
types and constructor checks in `crates/splot-recon` without adding a
byte-consuming decode path, reference-frame-store behavior, hashes, Y4M output,
external decoder integration, or new dependencies. The runtime facts come from
the committed AV2 v1.0.0 spec mirror:

- § 6.4.1 defines `bit_depth_idc`, `BitDepth`, `chroma_format_idc`,
  `SubsamplingX`, `SubsamplingY`, `Monochrome`, and `NumPlanes`.
- § 6.17.4.1 defines coded luma frame dimensions.
- § 6.17.4.4 defines crop alignment and positive visible crop dimensions.
- § 7.21.1 and § 7.21.2 define cropped output arrays and chroma output
  dimensions.
- § 7.23 distinguishes padded reference storage from cropped output frames.

## Goals / Non-Goals

**Goals:**

- Add dependency-free public runtime types in `splot-recon` for bit depth,
  pixel format, plane identity, output index, dimensions, visible rectangles,
  immutable owned planes, immutable decoded frames, and typed reconstruction
  errors.
- Enforce checked arithmetic, stride, buffer length, visible-rectangle, crop
  alignment, sample-type, sample-range, plane count, and plane-shape
  invariants in constructors.
- Record `INFRA-RECON-FRAME-PLANE-TYPES` in implementation and decoder support
  status with self-contained tests.
- Keep the API useful for future frame-hash/Y4M/reference work while clearly
  separating cropped output frames from padded reference storage.

**Non-Goals:**

- No byte-consuming decode, reconstruction algorithm, prediction, loop filter,
  film grain, frame hash computation, Y4M output, or CLI behavior.
- No reference-slot manager, `FrameStore` implementation, order-hint logic, or
  film-grain metadata persistence.
- No `splot-core`, `splot-decode`, encoder, dependency graph, CI, workflow, AVM,
  or dav2d integration.
- No resource-limit diagnostic emission. `splot-recon` reports local
  construction errors; future decode planning maps allocation policy failures
  to decoder diagnostics.

## Decisions

1. Keep `splot-recon` independent and dependency-free.

   The error type will implement `Display` and `std::error::Error` manually
   instead of adding `thiserror`. This avoids changing the crate dependency
   graph for a model-only slice and keeps `cargo xtask check-dependency-direction`
   straightforward.

2. Split the API by responsibility.

   `lib.rs` will expose small modules for errors, format, geometry, planes, and
   frames. Private fields and validating constructors keep invariants at the
   boundary while allowing future internal representation changes.

3. Model output frames, not reference storage.

   `Plane<T>` may include storage dimensions, stride, and a visible rectangle
   so it can represent padding, but `DecodedFrame<T>` validates cropped output
   plane shapes from § 7.21.1/§ 7.21.2. Padded § 7.23 reference storage remains
   a future API.

4. Use sealed sample types.

   `ReconSample` will be sealed and implemented only for `u8` and `u16`.
   `u8` is valid only for 8-bit output; `u16` can hold 8-bit and 10-bit output.
   Constructors validate every stored sample is within the active bit depth.

5. Validate frames from luma-visible geometry.

   `DecodedFrame::try_new` will accept coded luma size, visible luma rectangle,
   bit depth, pixel format, output index, and Y/U/V planes. It will validate
   crop alignment for non-monochrome formats and derive visible chroma sizes as
   `((w + subX) >> subX) x ((h + subY) >> subY)`.

## Risks / Trade-offs

- API shape becomes sticky before the decoder exists -> keep types small,
  immutable, and constructor-focused, and avoid reference-store-specific fields
  until those semantics are implemented.
- Runtime type support could be mistaken for decode support -> docs and matrix
  rows explicitly state that no decode, hash, Y4M, or reference-store behavior
  is implemented.
- Full-buffer sample validation is O(n) -> acceptable for constructor safety in
  this foundational slice; future performance-sensitive paths can add internal
  trusted constructors only with proof and tests.
- `splot-recon` cannot emit `decode/resource-limit` without a decode crate
  dependency -> return `ReconError` locally and leave diagnostic mapping to
  future byte-consuming decoder code.
