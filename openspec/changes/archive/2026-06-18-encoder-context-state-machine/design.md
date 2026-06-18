## Context

`encoder-frame-input-views` made `Context::send_frame` accept a real borrowed
`Frame<'_>`, but all three lifecycle calls still return
`splot_core::Error::Unimplemented`. That keeps callers from testing normal
push/pull behavior such as receive-before-input, backpressure, flush, and
end-of-stream. This change is non-normative API plumbing tracked by
`ENC-CONTEXT-STATE-MACHINE`; it emits no AV2 syntax and therefore has no AV2
section citation.

Reference gate:

- Decoder-visible behavior: no.
- AV2 spec sections read: none needed; no bitstream syntax, reconstruction,
  reference state, or layer behavior changes.
- AVM files/tests/streams used as oracle: none; no emitted stream.
- Matrix row: add `ENC-CONTEXT-STATE-MACHINE`.
- Reference docs read: `ENCODER-RESEARCH-NOTES.md`,
  `THIRD-PARTY-NOTICES.md`, `RAV1E-SOURCE-MAP.md`; SVT-AV1 not relevant.
- rav1e concepts used: high-level safe Rust push/pull API inspiration only.
- Third-party material copied: none.

## Goals / Non-Goals

**Goals:**

- Expose an explicit `EncoderState` with accepting, draining, finished, and
  failed states.
- Return typed `SendFrameStatus`, `ReceivePacketStatus`, and `FlushStatus`
  values instead of using `Unimplemented` as lifecycle flow control.
- Use typed encoder errors for invalid state transitions.
- Keep bounded input/output queue accounting deterministic and testable.
- Preserve zero-copy input semantics by not retaining borrowed sample data.
- Add state-transition unit/property tests and a bounded command-sequence fuzz
  target.

**Non-Goals:**

- No legal AV2 packet production and no fake packet bytes.
- No Y4M reader, output container, CLI success path, RangeEncoder, coded tile
  body, reconstruction, reference storage, RDO, rate control, or speed-preset
  work.
- No dependency graph, workspace manifest, or `splot-core` changes.

## Decisions

### Queue metadata, not borrowed samples

`Frame<'_>` cannot be stored by `Context` without either carrying lifetimes
through the context or copying/materializing media samples. For this phase,
`send_frame` will validate the call state and push only `FrameInfo` into a bounded
internal queue. `receive_packet` will retire queued frame metadata from the
no-output frontier and report that no packet is ready.

Alternative considered: make `Context` lifetime-parameterized over borrowed
frames. Rejected because it would force a public API shape around a temporary
pre-encode queue and would not match future explicit lookahead/retained-frame
ownership.

### Separate state and operation status types

The public API will expose:

- `EncoderState`: current lifecycle state.
- `EncoderOperation`: the operation used in state errors.
- `SendFrameStatus`: accepted or backpressure without throwing.
- `ReceivePacketStatus`: packet-ready placeholder shape, need-more-data, or
  finished.
- `FlushStatus`: draining or already finished.

`Error::State` will represent invalid transitions such as send-after-flush,
send-after-finished, or calls after failed state. Queue-full is a normal status,
not an error, so callers can implement backpressure loops without exception
control flow.

Alternative considered: reuse the existing `EncoderStatus` enum. Rejected because
one enum is too vague for all operations and still cannot distinguish packet
readiness from send backpressure cleanly.

### No-output receive semantics

Until a real encode core exists, `receive_packet` must never return
`PacketReady` with invented data. It may consume one queued input metadata entry
to advance the lifecycle, then return `NeedMoreData`. After `flush`, once the
input queue is empty and no output exists, it transitions to `Finished` and
returns `Finished`.

Alternative considered: make every receive return `NeedMoreData` without
consuming queued input. Rejected because flush would never be able to reach a
deterministic end-of-stream state when frames were accepted before flush.

### Current queue limits are fixed constants

This PR will use small fixed queue limits in `Context` and expose accessors for
tests and future callers. They are not new runtime configuration because the
first supported encode path needs a broader resource-limit design.

Alternative considered: add queue sizes to `EncoderRuntimeConfig`. Rejected to
avoid public resource-limit API churn before Y4M/lookahead/resource-limit work.

## Flight Manifest

- Change ID: `encoder-context-state-machine`
- Feature IDs: `ENC-CONTEXT-STATE-MACHINE`
- Base commit: `a57c0c68aed795e379b1c7af1a4a115fabe0b977`
- Depends on merged changes: `encoder-program-contract`,
  `encoder-recon-dependency`, `encoder-frame-input-views`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/context.rs`
  - `crates/splot-encode/src/core_boundary.rs`
  - `crates/splot-encode/src/error.rs`
  - `crates/splot-encode/src/lib.rs`
  - `crates/splot-cli/src/commands/encode.rs`
  - `fuzz/Cargo.toml`
  - `fuzz/fuzz_targets/encoder_context_state_machine_bytes.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/SPEC-MAPPING.md`
  - `docs/ARCHITECTURE.md`
  - `docs/CONCURRENCY.md`
  - `docs/ENCODER-GOAL.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/specs/encoder-api/spec.md`
  - `openspec/changes/encoder-context-state-machine/**`
- Exact files/directories forbidden to this PR:
  - `crates/splot-core/**`
  - `crates/splot-recon/**`
  - `crates/splot-decode/**`
  - `crates/splot-validate/**`
  - workspace dependency manifests and lockfiles except `fuzz/Cargo.toml`
  - AV2 spec mirror files under `docs/spec/av2/**`
- Public APIs/types owned:
  - `Context::{send_frame, receive_packet, flush, state}`
  - `EncoderState`
  - `EncoderOperation`
  - `SendFrameStatus`
  - `ReceivePacketStatus`
  - `FlushStatus`
  - encoder `Error::State`
- Matrix rows owned: `ENC-CONTEXT-STATE-MACHINE`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none (`gh pr list --state open` returned `[]`)
- Changed-file intersection with each sibling PR: none
- Semantic overlap with each sibling PR: none
- Can build/test/merge directly onto main without another open PR: yes

## Risks / Trade-offs

- Queueing metadata without pixels could be mistaken for a successful encode.
  Mitigation: docs, statuses, matrix notes, and CLI continue to state that no
  packet/output success path exists.
- Public status names can churn before the full encoder lands. Mitigation: keep
  names small, operation-specific, and marked as the pre-1.0 API surface.
- The no-output receive loop is temporary. Mitigation: tests assert no packet is
  emitted; later packet-producing changes must update the same tests and matrix
  proof.
