# Encoder roadmap

`status: active`  
`owner: encoder`  
`Feature ID: DOC-ENCODER-PROGRAM-CONTRACT`

This roadmap is sequenced for small PRs. Each non-trivial step needs a matrix row,
an OpenSpec change unless trivial, focused tests, and final reviewer acceptance on
the exact HEAD that would merge.

## Phase 0 - Program contract

Status: this PR.

- Add `docs/ENCODER-GOAL.md`, `docs/ENCODER-ROADMAP.md`, and
  `docs/ENCODER-GAP-AUDIT.md`.
- Replace the stale validator-roadmap encoder fence with an encoder carve-out that
  keeps validator work owned by `docs/VALIDATOR-ROADMAP.md`.
- Mark `toy-intra-encoder-v0` as parked and superseded, not resumed.
- Do not change Rust code or dependency direction.

## Phase 1 - `encoder-recon-dependency`

Status: next exclusive encoder change after Phase 0.

Decision: explicitly approve and design the `splot-encode -> splot-recon`
dependency edge, or choose a different closed-loop boundary. This phase owns:

- Public boundary between encoder frame input, recon plane/workspace types, and
  output proof.
- Deterministic ownership model for borrowed views and materialized frames under
  `docs/ZERO_COPY.md`.
- Dependency-direction checks, focused compile tests, and API docs.

No encode success path should land in this phase unless it also satisfies the later
legal-stream evidence gates.

## Phase 2 - Input and API model

Status: planned.

Feature areas: `ENC-Y4M-INPUT`, `ENC-SPEED-PRESETS`.

- Replace the empty `Frame` placeholder with a strong input model for 8/10-bit
  YUV420 Y4M pictures.
- Keep bitstream-affecting configuration separate from runtime policy.
- Define unsupported-format behavior for 12-bit, monochrome, YUV422, YUV444, and
  non-Y4M inputs.
- Preserve deterministic behavior across worker thread counts.

## Phase 3 - Output writer integration

Status: planned.

Feature area: `ENC-BITSTREAM-WRITER`.

- Drive the existing `splot-core` syntax and container writers from encoder-owned
  models.
- Keep coded tile payload generation out until the entropy/range encoder and tile
  body model exist.
- Prove round trips through parser/writer tests and `splot validate`.

## Phase 4 - First legal all-intra stream

Status: planned, replaces the parked toy bootstrap path.

- Re-propose all-intra work under Baseline Encoder Profile v1 instead of resuming
  `toy-intra-encoder-v0`.
- Use closed-loop reconstruction before public success.
- Emit only syntax the writer can produce and the validator accepts.
- Record fixtures, hashes, and matrix proof before marking any encode stage done.

## Phase 5 - Differential and decode evidence

Status: planned.

- Add self-contained fixtures and checks that do not require the network.
- Use AVM and dav2d as supplemental local evidence when available, recording exact
  versions and commands in the matrix or PR text.
- Keep live differential harness work separate from the validator roadmap fence
  until `CONF-AVM-DIFF-HARNESS` is explicitly implemented.

## Phase 6 - Basic inter support

Status: planned.

- Start only after all-intra output is legal, deterministic, and closed-loop.
- Add reference-state ownership, inter frame-header writer support, and conformance
  evidence in separate scoped PRs.
- Do not infer AV2 syntax from AV1 or external encoder source.

## Phase 7 - Rate control and speed presets

Status: planned.

Feature areas: `ENC-RATE-CONTROL-V0`, `ENC-SPEED-PRESETS`.

- Keep rate control policy separate from bitstream syntax.
- Add speed presets as runtime policy with deterministic output for a fixed preset.
- Record performance evidence only after correctness proof exists.
