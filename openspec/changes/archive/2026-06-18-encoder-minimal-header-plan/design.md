## Context

`splot-encode` currently has:

- validated borrowed 8-bit YUV420 input frames;
- a deterministic push/pull context lifecycle that intentionally emits no
  packets;
- a private ordered syntax IR for future low-level sequence/frame/tile/block
  decisions.

`splot-core` already owns the AV2 writer surface for sequence headers, frame
headers, tile-group structures, Annex B, IVF, and complete OBU dispatch. This
change does not alter those writers. It creates the next private encoder-side
planning boundary so a later writer-integration PR can translate a checked
encoder plan into concrete `splot-core` writer models.

Relevant AV2 syntax sections named by this plan:

- `§ 5.4` sequence header OBU:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4`
- `§ 5.18` frame header:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18`
- `§ 5.19` tile group OBU:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`
- `§ 5.20.1` tile group payload framing:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`

No rav1e or SVT-AV1 implementation ideas are used for this change.

## Goals / Non-Goals

**Goals:**

- Add private `header_plan` records under `splot-encode` for a minimal
  sequence/frame/tile-group header intent.
- Validate the plan against `EncoderConfig` and `FrameInfo` before a future
  writer can consume it.
- Keep the accepted subset aligned with current input support: 8-bit YUV420
  frame metadata whose visible luma size exactly matches the encoder config.
- Record deterministic single-tile, first-frame, all-intra header intent without
  creating bytes.
- Keep `Context::receive_packet` non-emitting.

**Non-Goals:**

- No `splot-core` writer model construction yet.
- No AV2 byte output, OBU construction, Annex B/IVF output, or packet queuing.
- No coded tile body, coefficient syntax, entropy/CDF selection, reconstruction,
  transform, quantization, mode decision, rate control, or public encode success.
- No public API surface and no dependency graph changes.

## Decisions

1. Private intent records, not `splot-core` header structs.

   The first plan stores the encoder's future header intent: sequence dimensions
   and format, first-frame metadata, a single first tile group covering tile 0,
   and explicit placeholders for the policy choices that will later map to
   concrete writer fields. This keeps the PR useful without inventing every AV2
   bit-level header value before coded-tile integration exists.

2. One-frame construction boundary.

   `MinimalHeaderPlan::new(config, frame_info)` plans the first accepted frame.
   Multi-frame ordering and reference refresh are later mission phases. This
   matches the current input queue and avoids public claims about multi-frame
   output.

3. Reuse existing config/frame validation semantics.

   The header plan rejects zero dimensions, frame/config size mismatch, bit-depth
   mismatch, chroma mismatch, and unsupported current planning formats with typed
   private errors. This mirrors `Context::send_frame` but stays private so future
   writer integration can call it without exposing new API.

4. Determinism by value types and stable ordering.

   Header plan records are `Clone + Debug + Eq + PartialEq`, use no unordered
   collections, and include explicit `TileIndex` values from the syntax IR. Tests
   assert repeated construction yields equal values and stable debug output.

5. No context integration beyond a regression guard.

   The context lifecycle stays as-is. A regression test continues to prove that
   adding header planning does not populate `output_queue` or return a `Packet`.

## Risks / Trade-offs

- Header intent may need additional fields when writer integration starts.
  Mitigation: keep the module private and non-exhaustive by construction; add
  fields in the writer-integration PR with targeted tests.
- Supporting only the current 8-bit YUV420 planning subset defers 10-bit Baseline
  v1 planning. Mitigation: current `Frame` validation is already 8-bit-only; the
  10-bit path should land with the input model that can actually supply 10-bit
  samples.
- Duplicating some `Context::send_frame` validation in private planning can drift.
  Mitigation: tests cover mismatches in both contexts, and the private module
  should become the shared validation source once it is integrated into the
  lifecycle.

## Flight manifest

- Change ID: `encoder-minimal-header-plan`
- Feature IDs: `ENC-MINIMAL-HEADER-PLAN`
- Base commit: `66a941a7dbe14ab1a098a0a628b610e5b784f3d1`
- Depends on merged changes: `ENC-Y4M-INPUT`, `ENC-CONTEXT-STATE-MACHINE`,
  `ENC-SYNTAX-IR`, `ENC-BITSTREAM-WRITER`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/lib.rs`
  - `crates/splot-encode/src/header_plan.rs`
  - `crates/splot-encode/src/header_plan_tests.rs`
  - `crates/splot-encode/src/context.rs` test-only regression if needed
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-minimal-header-plan/**`
- Exact files/directories forbidden to this PR:
  - `crates/splot-core/**`
  - `crates/splot-recon/**`
  - `crates/splot-decode/**`
  - `crates/splot-validate/**`
  - `crates/splot-cli/**`
  - Cargo manifests and lockfiles
  - `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-MINIMAL-HEADER-PLAN`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none at branch creation
- Changed-file intersection with each sibling PR: none
- Semantic overlap with each sibling PR: none
- Can build/test/merge directly onto main without another open PR: yes
