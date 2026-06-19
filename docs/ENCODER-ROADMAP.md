# Encoder roadmap

`status: active`
`owner: encoder`
`Feature ID: DOC-ENCODER-PROGRAM-CONTRACT`

This roadmap is sequenced for small PRs. Each non-trivial step needs a matrix row,
an OpenSpec change unless trivial, focused tests, and final reviewer acceptance on
the exact HEAD that would merge.

## Phase 0 - Program contract

Status: done.

- Add `docs/ENCODER-GOAL.md`, `docs/ENCODER-ROADMAP.md`, and
  `docs/ENCODER-GAP-AUDIT.md`.
- Replace the stale validator-roadmap encoder fence with an encoder carve-out that
  keeps validator work owned by `docs/VALIDATOR-ROADMAP.md`.
- Mark `toy-intra-encoder-v0` as parked and superseded, not resumed.
- Do not change Rust code or dependency direction.

## Phase 1 - `encoder-recon-dependency`

Status: done.

Decision: explicitly approve and land the `splot-encode -> splot-recon`
dependency edge without adding public encode behavior. This phase owns:

- Private dependency boundary for future encoder frame input, recon
  plane/workspace types, and output proof.
- Deterministic ownership policy alignment with `docs/ZERO_COPY.md`; concrete
  borrowed views and materialized frame APIs stay in later phases.
- Dependency-direction checks, focused compile tests, and API docs.

No encode success path should land in this phase unless it also satisfies the later
legal-stream evidence gates.

## Phase 2 - Input and API model

Status: in progress.

Feature areas: `ENC-Y4M-INPUT`, `ENC-CONTEXT-STATE-MACHINE`,
`ENC-SYNTAX-IR`, `ENC-MINIMAL-HEADER-PLAN`, `ENC-SPEED-PRESETS`.

- Land validated borrowed 8-bit YUV420 input views, then extend the input model
  to 10-bit and Y4M stream adaptation in follow-up PRs.
- Land the context lifecycle state machine before any packet-producing path, so
  callers can test backpressure, flush, and end-of-stream without fake packets.
- Land a private deterministic syntax-planning IR before any writer integration,
  so sequence/frame/tile/block/token decisions can be ordered and inspected
  without mutating an output writer.
- Land a private minimal header plan after the syntax IR, so current first-frame
  sequence/frame/tile-group header intent is typed and rejected before any writer
  is allowed to emit bytes.
- Land a typed runtime speed preset framework, wired from `splot encode --speed`
  into `EncoderRuntimeConfig`, before any preset-specific mode decision or packet
  output exists.
- Keep bitstream-affecting configuration separate from runtime policy.
- Define unsupported-format behavior for 12-bit, monochrome, YUV422, YUV444, and
  non-Y4M inputs.
- Preserve deterministic behavior across worker thread counts.

## Phase 3 - Output writer integration

Status: planned.

Feature area: `ENC-BITSTREAM-WRITER`.

- Drive the existing `splot-core` syntax and container writers from encoder-owned
  models, starting from the private `ENC-SYNTAX-IR` planning records and the
  `ENC-MINIMAL-HEADER-PLAN` header-intent bridge.
- The generic AV2 §8.2 entropy/range encoder primitive now exists in
  `splot-core`; keep coded tile payload generation out until §8.3 token/CDF
  selection and the tile body model exist.
- Prove round trips through parser/writer tests and `splot validate`.

## Phase 4 - First legal all-intra stream

Status: planned, replaces the parked toy bootstrap path.

- Re-propose all-intra work under Baseline Encoder Profile v1 instead of resuming
  `toy-intra-encoder-v0`.
- Land a private residual foundation before forward transform work, so
  source-minus-prediction arithmetic, block geometry, and signed residual
  materialization are proven independently of packet output.
- Land a private forward-transform foundation for the first 4x4 DCT_DCT DC-only
  uniform-residual subset before quantization work, proving the no-op
  quant/dequant inverse handoff without claiming broad transform support.
- Land a private fixed-quantizer v0 for that first 4x4 DCT_DCT DC-only subset,
  proving the `splot-recon` dequant/inverse handoff before coefficient
  tokenization or rate-control work.
- Land a private coefficient-tokenization bridge for the current luma 4x4
  DCT_DCT DC-only top-left neutral-spatial-context quantized subset, proving
  q-context-derived ordered base-tier entropy-token records through the in-tree
  AV2 §8.2 symbol encoder/decoder before tile-body writer integration.
- Land a private minimal closed-loop reconstruction for the current 8-bit luma
  4x4 DCT_DCT DC-only top-left subset, composing the encoder
  residual/forward-transform/quantization stages with the `splot-recon`
  decoder-visible AV2 §7.13.2.10 DC prediction, §7.14.2/§7.14.4 dequantization,
  §7.15.4 inverse transform, and §7.14.3 reconstruct, plus the current-frame
  workspace and decoded-frame hash, and prove the emitted coefficient decisions
  reconstruct identically — before any tile-body or packet path.
- Land private block-symbol emission incrementally before the tile body, starting
  with the luma intra-mode symbols (`y_mode_set`/`y_mode_index` for DC_PRED at the
  tile origin, AV2 §5.20.5.5/§8.3.2), proving each through the in-tree AV2 §8.2
  symbol coder, then extend to chroma `uv_mode` (DC chroma, §5.20.5.6/§8.3.2,
  landed), compose the ordered §5.20.5.3 mode-info prefix (`y_mode_set`,
  `y_mode_index`, `uv_mode`) into one trace (landed), extend it with the first
  `residual()` symbol — the luma `txb_skip` — through one unified §8.2 coder
  (landed), and complete the all-zero block with the chroma U/V `txb_skip`
  symbols in `residual()` order (landed), then add the minimal *coded* block — a
  single luma DC coefficient's `txb_skip=0` + `eob_pt_16` + `coeff_base_eob` +
  `dc_sign` (landed), then the base-range tier — a larger luma DC coefficient's
  `coeff_br` after `coeff_base_eob` (landed), then add the §8.2.5 bypass-literal
  token kind (the foundation for non-luma-DC `sign_bit` and the golomb tail,
  landed), then a coded chroma U DC coefficient whose sign is a `sign_bit` bypass
  literal, with the §8.3.2 chroma contexts and the V `txb_skip` EobU context
  (landed), then the §5.20.7.28 golomb tail for a larger luma DC
  coefficient — the finite-q range (8..=17) and the golomb-prefix range (18..=525,
  the q_length/golomb_length unary codes + a sized `coeff_rem`), both proven by
  range loop tests, landed. The coded intra block symbol sequence now roundtrips
  for luma (the full single-DC magnitude vocabulary 1..=525) and a coded chroma DC.
  The multi-coefficient (eob>1) path has begun: the §8.3.2 `coeff_base`
  low-frequency luma context derivation, the non-EOB `coeff_base` token + CDF row,
  and the reusable multi-coefficient token accessors (coded `all_zero`, a
  parameterized `eob_pt_16`, a parameterized `coeff_base_eob`) all landed — the
  pieces the eob>1 trace composes. The eob>1 trace composition itself, the chroma
  base-range/golomb tiers, partition syntax, and the tile-body byte assembly
  remain.
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
- Consume the typed speed preset framework in later search/scheduling decisions
  only after syntax correctness is proven for the affected output path.
- Record performance evidence only after correctness proof exists.
