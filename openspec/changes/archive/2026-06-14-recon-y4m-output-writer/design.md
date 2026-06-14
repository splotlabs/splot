## Context

`splot-recon` owns immutable decoded frame and plane model types plus
`DecodedFrameHashInput<'_, T>` / `DecodedFrameHash` for visible-sample identity.
The decoder roadmap still marks Y4M output as planned, and the CLI remains a
byte-planning entry point that emits structured diagnostics instead of decoded
output.

This change is reconstruction output infrastructure only. It serializes already
materialized caller-supplied `DecodedFrame<T>` values and does not read AV2
bitstreams, decode tile payloads, reconstruct pixels, select output order, apply
film grain, refresh references, or invoke AVM/dav2d.

## Goals / Non-Goals

**Goals:**

- Add a `crates/splot-recon/src/y4m.rs` API for writing Y4M stream headers and
  frames from validated `DecodedFrame<T>` values.
- Use visible luma size and AV2-derived `BitDepth` / `PixelFormat` to derive a
  stable frame format.
- Require caller-supplied frame rate metadata instead of inventing timing from
  decoded frames.
- Serialize visible rows only, excluding storage stride and coded padding.
- Pin output bytes with self-contained tests for headers, plane order, bit
  depth, chroma formats, crop/stride exclusion, multi-frame output, mismatch
  errors, and writer error propagation.
- Keep `splot-recon` scheduler-free and dependency-free beyond existing
  standard-library I/O.

**Non-Goals:**

- No CLI `splot decode -o` success path.
- No byte-consuming decode, tile payload parsing, symbol decoding, prediction,
  inverse transform, loop filtering, or reconstruction algorithm.
- No output scheduling, presentation-order sorting, show-existing/flush output,
  reference refresh, or film-grain synthesis.
- No AV2 `METADATA_TYPE_DECODED_FRAME_HASH` verification.
- No AVM/dav2d source inspection requirement, runs, wrappers, scripts, build
  probes, Cargo dependencies, CI jobs, runtime process execution, or mandatory
  tests.
- No parser changes and no duplication of the PR #113 byte/container traversal
  fixes already present on main.

## Decisions

1. Put the writer in `splot-recon`.

   The writer consumes `DecodedFrame<T>`, `Plane::visible_rows()`, `BitDepth`,
   and `PixelFormat`, all of which are owned by `splot-recon`. `splot-cli`
   remains thin, and future byte-consuming decode orchestration belongs in
   `splot-decode` above this API.

2. Add a dedicated Y4M error type.

   `ReconError` is clone/equality-friendly construction error state. Y4M writer
   failures include `std::io::Error`, so `Y4mError` will be separate and expose
   typed configuration/format/mismatch variants plus `Io { source }`.

3. Use caller-supplied stream metadata.

   `DecodedFrame<T>` does not contain frame rate, sample aspect ratio, colorimetry,
   or chroma siting. The initial writer pins progressive `Ip` output and requires
   a validated nonzero `Y4mFrameRate`. Sample aspect ratio defaults to `A0:0`
   unless a future change adds an explicit API.

4. Pin Y4M chroma tags as repository policy.

   Y4M is outside the AV2 specification. The writer maps AV2-derived formats to
   repository-owned output tags: `Cmono`, `Cmono10`, `C420`, `C420p10`, `C422`,
   `C422p10`, `C444`, and `C444p10`. AV2 citations justify the underlying
   decoded sample and chroma facts, not the Y4M container syntax itself.

5. Match frame payload bytes to the existing visible-sample policy.

   Non-monochrome frames write Y, then U, then V. Monochrome writes only Y. Rows
   are emitted left-to-right and top-to-bottom. 8-bit samples write one byte;
   10-bit samples write little-endian `u16` bytes with no scaling. This mirrors
   the existing hash-input byte policy so hashes and Y4M payloads stay aligned
   over visible samples.

6. Preserve the PR #101 concurrency boundary.

   `splot-recon` must not construct worker pools, depend on Rayon/crossbeam, own
   queues, spawn threads, or sort by worker completion. Future multi-frame Y4M
   output must be committed in emission order by the decode driver through its
   `DecodeContext` / `splot_parallel::WorkerPool` boundary before calling this
   writer.

## Risks / Trade-offs

- [Risk] Marking `output-y4m` source-backed could be mistaken for runtime
  `splot decode -o` support. -> Mitigation: matrix/docs/OpenSpec explicitly say
  this is a library writer over caller-supplied frames and CLI/runtime decode
  output remains unsupported.
- [Risk] Y4M chroma tag spelling can drift. -> Mitigation: expose stable tag
  methods and exact header tests for every supported modeled format.
- [Risk] Stream mismatch checks could write partial frames. -> Mitigation:
  validate frame format against the stream header before writing `FRAME\n` or
  payload bytes.
- [Risk] Y4M byte traversal could diverge from decoded-frame hash traversal. ->
  Mitigation: add tests for the same visible-row, plane-order, and bit-depth
  cases already covered by hash input.
