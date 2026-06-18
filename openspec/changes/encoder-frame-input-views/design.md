## Context

`splot-encode` currently exposes a `Frame` placeholder with no plane data. The
previous `encoder-recon-dependency` change added the approved
`splot-encode -> splot-recon` dependency edge, and `splot-recon` already exposes
validated `PlaneRef` / `FrameRef` borrowed views plus `SharedFrame` for explicit
frame sharing. This PR uses those primitives to make the encoder input surface
real while keeping all encode operations unavailable until the state-machine and
coded-frame work land.

This is non-normative encoder API infrastructure. It cites AV2 § 6.4.1 only for
the existing `splot-recon` bit-depth and YUV420 chroma-format facts; it does not
parse or emit AV2 syntax and does not use AVM as an oracle.

### Flight Manifest

- Branch: `codex/encoder-frame-input-views`
- Baseline: `origin/main` at
  `0ddea9b6b2377e0f99b493720f1e7a10a9a973bb`
- Sibling PR audit: `gh pr list --state open` returned no open PRs before this
  change started.
- Owned implementation paths:
  - `crates/splot-encode/src/frame.rs`
  - `crates/splot-encode/src/context.rs`
  - `crates/splot-encode/src/error.rs`
  - `crates/splot-encode/src/lib.rs`
  - `crates/splot-cli/src/commands/encode.rs`
  - `fuzz/Cargo.toml`
  - `fuzz/Cargo.lock`
  - `fuzz/fuzz_targets/encoder_frame_input_views_bytes.rs`
- Owned docs/spec/status paths:
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `docs/ENCODER-GOAL.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ARCHITECTURE.md`
  - `docs/SPEC-MAPPING.md`
  - `docs/TESTING.md`
  - `docs/ZERO_COPY.md`
  - `AGENTS.md`
  - `.github/workflows/ci.yml`
  - `openspec/changes/encoder-frame-input-views/**`
- Forbidden paths unless a reviewer explicitly expands scope:
  - `crates/splot-core/**`
  - `crates/splot-recon/**`
  - `crates/splot-decode/**`
  - `crates/splot-validate/**`
  - dependency manifests outside the fuzz-target registration already listed

## Goals / Non-Goals

**Goals:**

- Replace the empty `Frame` stub with validated borrowed 8-bit YUV420 frame
  input views.
- Model frame identity, optional timestamp ticks, visible luma size, bit depth,
  chroma layout, plane stride, plane visible rectangles, and backing buffer
  lengths with typed API values.
- Reject invalid plane counts, unsupported formats, stride/visible geometry
  errors, derived chroma-size mismatches, odd-size edge cases, and arithmetic
  overflow before callers can access rows through the encoder frame.
- Add an explicit retained/shared input path for future lookahead without
  deriving `Clone` or hiding pixel copies.
- Keep `send_frame`, `receive_packet`, and `flush` returning
  `splot_core::Error::Unimplemented`.

**Non-Goals:**

- No Y4M parser or input file reader.
- No context lifecycle/state-machine replacement.
- No coded packet, bitstream writer, entropy writer, reconstruction, lookahead,
  or encode decision path.
- No support claim for 10-bit input, monochrome, YUV422, YUV444, 12-bit, alpha,
  RGB, live capture, or non-Y4M input.

## Decisions

1. **Use `splot_recon::PlaneRef` as the borrowed plane validator.**
   `Frame<'a>` will hold `PlaneRef<'a, u8>` for Y, U, and V. This reuses the
   existing stride, visible-rectangle, length, and row-iteration validation
   instead of duplicating media-buffer rules in `splot-encode`.

   Alternative considered: create encode-local slice/stride validators. That
   would avoid exposing recon view types, but it would duplicate the zero-copy
   policy surface and create two places for plane-boundary bugs.

2. **Expose an encode-local frame contract around recon views.**
   `FrameInfo`, `FrameId`, and `FrameTimestamp` remain encode-owned metadata.
   The frame input API uses the existing encoder `BitDepth` and
   `ChromaSubsampling` enums, then maps the supported subset to recon's
   `BitDepth::Eight` and `PixelFormat::Yuv420` only for validation.

   Alternative considered: replace encoder config format enums with recon
   enums. That would be larger API churn and is not required to validate the
   first input surface.

3. **Retained input is explicit and shared, not implicit.**
   `RetainedFrame` wraps a caller-provided `splot_recon::SharedFrame<u8>` after
   validating that it is 8-bit YUV420. Sharing is spelled
   `RetainedFrame::share()`, delegating to `SharedFrame::share()`. This PR will
   not materialize a borrowed frame into owned storage; a future lookahead PR must
   add a specifically named and marked materialization boundary if it needs one.

   Alternative considered: make `Context::send_frame` copy the input internally.
   That would hide a large media copy behind a normal API call before lookahead
   policy exists.

4. **Keep lifecycle behavior unchanged.**
   `Context::send_frame` will accept the real frame type but still returns
   `Unimplemented`. This keeps this PR focused on input validation and avoids
   combining it with `encoder-context-state-machine`.

## Risks / Trade-offs

- **Public `splot-recon` view exposure** -> This deliberately makes recon view
  types part of the first encoder input API. The benefit is one validation model;
  the trade-off is that later recon-view changes must consider encoder API
  compatibility.
- **8-bit-only implementation while the program target includes 10-bit** -> The
  constructor rejects unsupported bit depths today. The matrix row records partial
  progress, and 10-bit remains future work under `ENC-Y4M-INPUT`.
- **No materialization helper in this PR** -> Callers that only have short-lived
  borrowed buffers cannot retain them yet. This is intentional; the first
  materialization boundary should land with lookahead or state-machine code that
  proves why the copy is needed.
- **No AVM differential proof** -> The feature does not produce decoder-visible
  syntax. Unit/property tests and fuzz coverage are the relevant evidence.
