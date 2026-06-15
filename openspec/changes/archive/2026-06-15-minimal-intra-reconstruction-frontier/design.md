## Context

The minimal runtime currently parses and validates the committed
`minimal-intra-8bit420-hash-v1` IVF shape, derives one tile work unit, consumes
the traced root partition and flat block-symbol sequence, then constructs a
decoded frame with `build_flat_yuv420_frame()`. That helper allocates planes
filled with sample value 128 directly in `splot-decode`.

Existing `splot-recon` primitives already provide the pieces this narrow tier
can honestly use today: `CurrentFrameWorkspace`, `DecodedFrameInfo`, luma DC
intra prediction, checked plane geometry, frame freeze, hash input, and Y4M
serialization. AV2 §5.20.5.5 identifies the traced luma intra mode syntax, while
§7.13.2.10 defines DC intra prediction. AV2 §5.20.5.6 maps the current traced
chroma symbol to a non-DC H/V-style path, and `splot-recon` does not yet expose a
horizontal/vertical chroma prediction primitive, so this change must not claim
faithful chroma prediction. It keeps chroma neutral through checked workspace
storage to preserve the existing minimal output contract.

## Goals / Non-Goals

**Goals:**
- Route the supported 64x64 flat minimal fixture through a crate-private
  reconstruction handoff using `splot-recon` workspace primitives and luma DC
  intra prediction.
- Preserve byte-identical hash and Y4M output for the existing minimal fixture.
- Preserve the current closed unsupported-feature behavior for every out-of-tier
  stream and mutation.
- Track the work with Feature ID
  `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` and matrix/docs evidence.

**Non-Goals:**
- No broad `decode_block()`, recursive `decode_tile()`, mode-info arrays, MiSizes
  mutation, residual syntax, inverse quant/transform, loop filters, reference
  refresh, film grain, raw output, or full intra reconstruction claim.
- No new public API, CLI flag, crate dependency, AVM/dav2d integration, or
  validator behavior change.
- No support for additional fixtures, chroma formats, bit depths, dimensions,
  chroma H/V prediction, or non-DC luma intra modes in this PR.

## Decisions

1. Keep the reconstruction handoff inside `splot-decode`.

   The runtime already owns byte-stream parsing, tier validation, and tile trace
   consumption. A small crate-private helper can translate the already-validated
   minimal trace into calls on `splot-recon` without making `splot-recon`
   byte-consuming or aware of decoder tiers. The alternative, adding a
   fixture-specific constructor to `splot-recon`, would move tier policy into the
   reconstruction crate and weaken the existing dependency boundary.

2. Use workspace reconstruction operations instead of direct filled-plane construction.

   The helper should create a checked `DecodedFrameInfo`, allocate a
   `CurrentFrameWorkspace<u8>` with an inert initial sample, run top-left DC
   prediction for the 64x64 Y plane, materialize neutral 32x32 U/V planes through
   checked workspace writes, then `freeze()` the workspace into a
   `DecodedFrame<u8>`. This keeps geometry, sample validation, and frame
   invariants in `splot-recon`. Direct `Plane::from_vec` construction is the
   current shortcut this change removes. The chroma part is intentionally not
   described as faithful `H_PRED` reconstruction; that remains future work.

3. Keep the tier facts explicit.

   The minimal runtime will still validate profile 0, 8-bit YUV420, 64x64, one
   tile, no filters/grain/tools, traced luma mode, traced chroma mode, all-zero
   transform symbols, and the `exit_symbol()` boundary before reconstruction.
   The reconstruction helper will not derive those conditions from syntax again;
   it receives an already-validated minimal trace recipe for this tier.

4. Preserve output identity as the main behavioral proof.

   Existing hash/Y4M tests are the public runtime contract. Add focused tests
   around the handoff and keep the existing expected digest/Y4M bytes unchanged.
   Any digest or Y4M byte change is a regression unless the OpenSpec and matrix
   explicitly change the output contract, which this PR does not.

## Risks / Trade-offs

- Runtime behavior could silently remain synthetic if the helper only fills a
  workspace with 128. Mitigation: initialize the workspace with a non-output
  sample, use luma DC prediction, and materialize chroma with checked neutral
  workspace fills; tests assert all visible samples and output bytes.
- A narrow luma DC handoff can be mistaken for broad intra support. Mitigation:
  matrix notes, OpenSpec requirements, and docs must state the non-claims
  explicitly, including the chroma H/V gap, and keep `intra-reconstruction`
  partial.
- Geometry or allocation changes could bypass limits. Mitigation: keep
  `ensure_runtime_limits()` before workspace allocation and rely on checked
  `DecodedFrameInfo`/workspace constructors.
- Output file safety could regress indirectly through Y4M changes. Mitigation:
  keep CLI Y4M atomic publication tests in the targeted gate set and avoid
  changing `crates/splot-cli/src/commands/decode.rs`.
