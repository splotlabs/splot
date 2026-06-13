# Agent Log: decoded-frame-plane-runtime-types

## Orchestrator

Objective: implement the next decoder mission slice after
`decode/unsupported-feature` ownership landed. Scope is the first
source-backed decoded output frame and plane model in `splot-recon`, tracked by
`INFRA-RECON-FRAME-PLANE-TYPES`.

Baseline: `origin/main` at `5eca41b` (`feat(decoder): move unsupported
diagnostic to decode crate (#95)`).

## Planning Subagents

### @architect

Agent id: `019ec211-84d8-7ed1-bf9d-dd8626ce1d07`

Findings:

- Implement this as `decoded-frame-plane-runtime-types` with Feature ID
  `INFRA-RECON-FRAME-PLANE-TYPES`.
- Keep the change inside `crates/splot-recon`, plus docs/status/OpenSpec
  updates.
- Use small modules for format, geometry, planes, frames, and errors.
- Add immutable output-frame types and constructor invariants before any decode,
  hash, Y4M, or reference-store behavior.
- Avoid overclaiming: this is not byte-consuming decode and not a reference-slot
  manager.

Decision: follow the module split and scope guidance, but avoid adding
`thiserror` so the change does not modify the dependency graph.

### @spec-reader

Agent id: `019ec211-87c8-7770-b5fb-fad355b013de`

Findings:

- AV2 § 6.4.1 maps `chroma_format_idc = 0` to 4:2:0, `1` to monochrome,
  `2` to 4:4:4, and `3` to 4:2:2; values above `3` are non-conformant.
- AV2 § 6.4.1 maps `bit_depth_idc = 0` to 10-bit and `1` to 8-bit; values
  above `1` are reserved.
- § 6.17.4.1 gives coded luma dimensions; § 6.17.4.4 gives positive crop
  dimensions and chroma crop-origin alignment.
- § 7.21.1/§ 7.21.2 define cropped output arrays and chroma output dimensions
  `((w + subX) >> subX) x ((h + subY) >> subY)`.
- § 7.23 reference storage uses padded `LrFrame`/`FrameStore` dimensions and
  must not be conflated with visible output.

Decision: implement only AV2 facts from the committed spec mirror and avoid
runtime claims for hashes, Y4M, film grain, or reference storage.

### @api-designer

Agent id: `019ec211-8a87-7482-8b51-8ec2a51a3d4d`

Findings:

- Provide `BitDepth::{Eight, Ten}`, `PixelFormat::{Monochrome, Yuv420, Yuv422,
  Yuv444}`, `PlaneId`, `PlaneSize`, `PlaneRect`, `OutputIndex`, `Plane<T>`,
  `DecodedFrame<T>`, and typed `ReconError`.
- Seal `ReconSample` to `u8` and `u16`; `u8` supports only 8-bit output while
  `u16` supports 8-bit and 10-bit output.
- Keep fields private and validate all public constructors.
- Validate stride, exact buffer length, checked arithmetic, visible rectangle,
  crop alignment, plane presence, plane shapes, and sample ranges.

Decision: use this shape as the implementation target with manually implemented
errors.

### @reference-oracle

Agent id: `019ec211-8d75-7cc2-9a5a-bb6e14d92b09`

Findings:

- AVM/dav2d source reads or command runs are unnecessary for this model-only
  change.
- The committed AV2 spec mirror, decoder roadmap/support matrix, and
  self-contained unit tests are sufficient evidence for the slice.
- Do not update `docs/LOCAL-REFERENCE-EVIDENCE.toml`.

Decision: no external decoder evidence will be added or required.

## Local Reference Boundary

No AVM or dav2d command was run for planning. No AVM/dav2d source, snippets,
binaries, submodules, dependencies, build probes, wrappers, CI jobs, required
scripts, required `xtask` commands, or mandatory tests are proposed.

## Implementation

Implemented in:

- `crates/splot-recon/src/lib.rs`
- `crates/splot-recon/src/error.rs`
- `crates/splot-recon/src/format.rs`
- `crates/splot-recon/src/geometry.rs`
- `crates/splot-recon/src/plane.rs`
- `crates/splot-recon/src/frame.rs`
- `docs/DECODER-ROADMAP.md`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `docs/IMPLEMENTATION-MATRIX.toml`
- Generated status docs:
  `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`,
  `docs/SPEC-COVERAGE.md`

`splot-recon` now exposes immutable decoded output model types:
`BitDepth`, `PixelFormat`, `PlaneId`, `OutputIndex`, `PlaneSize`,
`PlaneRect`, `Plane<T>`, `FramePlanes<T>`, `DecodedFrameInfo`,
`DecodedFrame<T>`, sealed `ReconSample`, and `ReconError`.

The implementation validates AV2-derived bit-depth/chroma mappings, positive
dimensions, crop bounds and chroma alignment, stride, checked sample/byte
accounting, exact backing buffer length, visible rows excluding padding,
monochrome/non-monochrome plane presence, AV2-derived chroma visible sizes,
sample-type compatibility, and sample-range limits. It does not consume bytes,
reconstruct pixels, compute hashes, write Y4M, store reference frames, invoke
external decoders, or change the dependency graph.

## Local Verification

Passed:

- `openspec validate decoded-frame-plane-runtime-types --strict`
- `cargo test -p splot-recon --locked`
- `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`
- `cargo xtask check-dependency-direction`
- `cargo xtask check-decoder-support`
- `cargo xtask feature-status --format markdown --output /tmp/splot-feature-status.md`
- `cargo xtask check-feature-status`
- `cargo xtask ci`

## Final Review Sign-offs

### Code/test review

Agent id: `019ec227-32e4-7411-bbfa-551375c40d3b`

Result: no findings. The reviewer checked `crates/splot-recon/src/*`, unit
tests, and OpenSpec task claims. Focused tests, clippy, OpenSpec validation,
dependency-direction, decoder-support, feature-status, and `cargo xtask ci`
were reported passing. Residual risk is limited to stated non-goals: no runtime
decode, hash, Y4M, or reference-store behavior.

### AV2/spec/status review

Agent id: `019ec227-36a7-72b2-9956-bde02011825e`

Result: no findings. The reviewer confirmed the §6.4.1 bit-depth/chroma
mappings, §6.17.4.1 frame-size framing, §6.17.4.4 crop/alignment claims,
§7.21 output-shape claims, and §7.23 output-vs-reference-store separation are
consistent with the local AV2 mirror. No decode/hash/Y4M/reference-store
overclaiming was found.

### Dependency and external-reference boundary review

Agent id: `019ec227-3ad1-7040-832e-55aa4c656898`

Result: no findings. The reviewer confirmed no Cargo dependency graph change,
no workflow/script/`xtask`/`build.rs` changes, no AVM/dav2d/ffmpeg invocation
or source inclusion, and no executable local reference evidence entries.
