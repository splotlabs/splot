## Context

`splot` remains validator-first, but the repository now has enough writer and
reconstruction foundation to plan the encoder track honestly. The current encoder
crate is still an API shell: `send_frame`, `receive_packet`, and `flush` return
`Error::Unimplemented`, the public `Frame` type has no data model, and the CLI
surfaces the unimplemented state.

The writer foundation in `splot-core` can emit many parsed syntax OBUs and container
framing, but it is not an entropy encoder. The tile-group writer still takes
caller-supplied coded tile bytes, `RangeEncoder` returns `Unimplemented`, and inter
frame-header composition is not complete. `splot-recon` exposes useful closed-loop
building blocks, but `splot-encode` does not depend on it yet.

## Goals / Non-Goals

**Goals:**

- Define Baseline Encoder Profile v1 as the program target before code changes.
- Record current writer, reconstruction, API, conformance, and PR ownership gaps.
- Make validator-roadmap language compatible with a scoped encoder program.
- Keep the first PR docs-only and tracked by `DOC-ENCODER-PROGRAM-CONTRACT`.
- Make the next exclusive encoder change explicit: `encoder-recon-dependency`.

**Non-Goals:**

- No Rust production behavior changes.
- No `splot-encode -> splot-recon` dependency yet.
- No RangeEncoder, entropy coding, rate control, speed preset, Y4M reader, or public
  encoder success path.
- No 12-bit encode promise; Baseline Encoder Profile v1 is limited to 8/10-bit
  YUV420 input.
- No external codec integration, copied third-party code, copied tables, copied
  constants, or copied prose.

## Decisions

1. **Use a docs Feature ID for the first PR.** The first flight is
   `DOC-ENCODER-PROGRAM-CONTRACT` because it changes planning, status, and
   OpenSpec contracts, not encoder behavior. Using an `ENC-*` implementation ID
   would imply a code or API milestone that this PR deliberately avoids.

2. **Define the profile before reviving the toy encoder.** The parked
   `toy-intra-encoder-v0` change predates the current writer/recon baseline and was
   scoped as a bootstrap experiment. Future all-intra work must be re-proposed under
   the Baseline Encoder Profile v1 contract instead of resuming the parked tasks.

3. **Keep validator ownership intact.** The validator roadmap remains the planning
   document for validator/parser/inspector work. Encoder work can proceed only under
   `docs/ENCODER-GOAL.md`, `docs/ENCODER-ROADMAP.md`, OpenSpec change artifacts,
   and implementation-matrix rows.

4. **Defer dependency-graph changes to their own PR.** The next encoder change is
   `encoder-recon-dependency`, where the maintainer can explicitly approve the
   `splot-encode -> splot-recon` edge, public boundary, and tests. This PR records
   that dependency as a gap and does not add it.

5. **Gate public encode success on evidence.** A future successful encode path must
   have matrix proof, self-contained CI coverage, `splot validate` evidence, and
   decode/differential evidence appropriate for the phase before it stops returning
   unimplemented status.

## Risks / Trade-offs

- Same-file matrix churn with other work -> keep edits to encoder/docs rows, avoid
  decoder support files, and rebase before review if another matrix-touching PR
  lands first.
- Roadmap over-promises implementation -> write every profile claim as a target or
  acceptance gate unless the source audit proves it exists today.
- Reusing recon too early creates dependency ambiguity -> isolate that decision in
  `encoder-recon-dependency`.
- External reference contamination -> keep rav1e and SVT-AV1 as architecture
  inspiration only and derive AV2 behavior from the AV2 spec mirror and AVM.

## Flight Manifest

- Change ID: `encoder-program-contract`
- Feature IDs: `DOC-ENCODER-PROGRAM-CONTRACT`; existing encoder rows referenced but
  not advanced: `ENC-BITSTREAM-WRITER`, `ENC-Y4M-INPUT`, `ENC-INTRA-TOY-V0`,
  `ENC-RATE-CONTROL-V0`, `ENC-SPEED-PRESETS`
- Base commit: `543eb6db`
- Depends on merged changes: current `main` through `543eb6db`
- Exact files/directories owned by this PR:
  - `docs/ENCODER-GOAL.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `docs/VALIDATOR-ROADMAP.md`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `openspec/changes/encoder-program-contract/**`
  - `openspec/changes/toy-intra-encoder-v0/tasks.md`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - `crates/**`
  - `Cargo.toml`
  - `Cargo.lock`
  - `AGENTS.md`
  - `docs/ARCHITECTURE.md`
  - `docs/CONCURRENCY.md`
  - `docs/ZERO_COPY.md`
  - `docs/DECODER-SUPPORT-*`
  - `xtask/**`
  - `fuzz/**`
  - `tests/**`
- Public APIs/types owned: none
- Matrix rows owned: `DOC-ENCODER-PROGRAM-CONTRACT`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open as of the final review-readiness pass
- Changed-file intersection with each sibling PR: none
- Semantic overlap with each sibling PR: none
- Can build/test/merge directly onto main without another open PR: yes

## Migration Plan

This PR adds documentation and OpenSpec artifacts only. Rollback is deleting the new
docs/change files and restoring the old roadmap text; no runtime migration exists.

## Open Questions

- The precise public boundary for `splot-encode` to borrow `splot-recon` frame,
  plane, and workspace types belongs to `encoder-recon-dependency`.
- The first implementation Feature IDs for all-intra Baseline Encoder Profile v1
  should be created in the implementation PR that owns that code, not in this docs
  contract.
