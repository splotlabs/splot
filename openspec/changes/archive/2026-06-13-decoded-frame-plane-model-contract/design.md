## Context

The decoder roadmap requires shared decoded frame, plane, pixel-format, and hash
types before supported decode output can land. Crate scaffolding still requires
explicit maintainer approval, so this change documents the contract those future
types must satisfy without adding Rust APIs or workspace members.

This change is docs/OpenSpec only. It does not add `splot-recon`,
`splot-decode`, `DecodedFrame`, `Plane<T>`, `PixelFormat`, `BitDepth`, runtime
allocation, hashing, Y4M output, fixtures, dependencies, or external decoder
integration.

## Goals / Non-Goals

**Goals:**

- Define the future decoded-frame and plane data model in
  `docs/DECODER-ROADMAP.md`.
- Ground the contract in AV2 § 6.4.1, § 6.17.4.1, § 6.17.4.4, § 7.21.1,
  § 7.21.2, and § 7.23.
- Mark `decoded-frame-plane-model` as a contract-only `partial` support row.
- State allocation, stride, and checked-arithmetic constraints before runtime
  code exists.

**Non-Goals:**

- No dependency graph change.
- No `splot-recon` or `splot-decode` crate creation.
- No public or internal Rust type definitions.
- No runtime decoded-frame allocation, hashing, Y4M output, or reference-frame
  storage.
- No emitted decode diagnostic change.
- No AVM/dav2d source, snippets, binaries, wrappers, scripts, tests, `xtask`
  commands, Cargo dependencies, build probes, or CI jobs.

## Decisions

1. Treat a future `DecodedFrame` as an AV2 output-frame record, not just a pixel
   buffer.

   AV2 § 7.21.1 defines output as `OutY`, `OutU`, and `OutV` at `bitDepth`.
   The future model must carry the output index/order, bit depth, pixel format,
   visible luma crop rectangle, plane dimensions, and source/crop metadata.
   AV2 § 7.1 and § 7.21 define the output processes; `splot` assigns a
   repository-owned zero-based emission index over frames emitted by those
   processes after supported stream/layer selection. Pixel data alone is not
   enough for encoder closed-loop reuse.

2. Distinguish output frames from reference-frame storage.

   AV2 § 7.21.2 output arrays are cropped output samples: current output copies
   from `LrFrame` through `CropLeft`/`CropTop`, while show-existing output copies
   from `FrameStore` through `RefCropLeft`/`RefCropTop`. AV2 § 7.23 reference
   storage keeps loop-restored `LrFrame` over coded/padded frame dimensions plus
   reference metadata (`RefFrameWidth`, `RefCropWidth`, `RefSubsamplingX`,
   `RefBitDepth`, `RefNumPlanes`, and related fields). The future model should
   not collapse those two shapes into one implicit buffer.

3. Keep `Plane<T>` storage rectangular and stride-aware, while making visible
   samples explicit.

   The contract permits stride for efficient future storage, but requires an
   explicit sample stride (`stride_samples`), `stride_samples >= storage_width`,
   checked `required_samples = stride_samples * storage_height`, and checked
   `allocation_bytes = required_samples * bytes_per_sample` before allocation.
   The full backing allocation, including padding, is charged against
   `DecodeLimits`. Hashes, Y4M, and fixtures must consume only the visible output
   rectangle. Padding must never become observable output.

4. Derive `PixelFormat` from AV2 sequence format variables.

   AV2 § 6.4.1 maps `chroma_format_idc` to `SubsamplingX`,
   `SubsamplingY`, `Monochrome`, and `NumPlanes`, and maps `bit_depth_idc` to
   `BitDepth`. The future model should expose both the friendly format
   (`Yuv420`, `Yuv422`, `Yuv444`, `Monochrome`) and the exact subsampling and
   plane-count facts needed by reconstruction code. AV2 v1.0.0 permits 8-bit
   and 10-bit output samples, and future sample storage must reject values
   outside `0..=(1 << bit_depth) - 1`.

5. Use checked arithmetic for all derived sizes.

   Plane dimensions, stride products, total allocation size, hash byte length,
   and reference-store byte accounting are derived from hostile bitstreams and
   caller limits. Future runtime code must compute them with checked arithmetic
   and reject overflow as `decode/resource-limit`.

6. Make emitted output immutable.

   A future output frame must remain valid after emission. Reference slots may
   own or share reconstructed buffers, but overwriting a reference slot must not
   mutate an already emitted output frame. Borrowed or shared views are
   acceptable only when backing samples are immutable for the output view, the
   output owns an independent copy, or copy-on-write / unique ownership is
   proven before reference-slot mutation.

7. Keep this row partial until runtime types and tests exist.

   The support matrix row can become `partial` because docs and OpenSpec define
   the contract. It cannot become `supported` until actual decoded-frame/plane
   types exist and are tested without external reference tools.

## Risks / Trade-offs

- API lock-in: naming future types too concretely could constrain crate design.
  Mitigation: document semantic contracts and conceptual names, not Rust module
  placement or exact signatures.
- Storage ambiguity: allowing stride could hide padding or byte-accounting bugs.
  Mitigation: require visible dimensions, checked sample and byte allocation
  arithmetic, limit charging for full backing allocations, and state that
  hashes/Y4M/tests exclude padding.
- Security: future allocation code can overflow on derived dimensions.
  Mitigation: make checked arithmetic and `DecodeLimits` gating part of the
  contract before implementation.
- Security: mutable aliasing could let a reference-slot overwrite change a
  previously emitted output frame. Mitigation: require immutable sharing, owned
  output copies, or copy-on-write / unique ownership before mutation.
- Encoder compatibility: an output-only record might be reused incorrectly as a
  reference-store entry. Mitigation: require the contract to distinguish cropped
  output frames from padded loop-restored reference storage and to preserve the
  § 7.23 metadata needed for future reference-frame management.
