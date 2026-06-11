# Validator implementation roadmap

`status: active`  
`owner: validator`  
`scope: AV2 validator/parser/inspector, not encoder`

| Phase | Scope | Status | Key open Feature IDs |
|---|---|---|---|
| 0 | Matrix and OpenSpec hygiene | done | — |
| 1 | Descriptor and payload-boundary foundation | done | `AV2-5.2.3-TRAILING-BITS`, `AV2-5.2.4-BYTE-ALIGNMENT` |
| 2 | OBU payload dispatch and input containers | done | `AV2-IVF-CONTAINER` |
| 3 | Sequence header parser (§ 5.4) | partial | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` |
| 4 | Activated sequence state (§ 6.2.2) | partial | `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`, `AV2-7.3.8-HLS-AVAILABILITY` |
| 5 | OBU ordering and temporal-unit state machine | partial | `AV2-7.3.2-CMVS-BOUNDARIES`, `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` |
| 6 | High-level syntax OBUs | partial | `AV2-5.8-LAYER-CONFIG-RECORD`, `AV2-5.10-OPERATING-POINT-SET` |
| 7 | Non-HLS payload OBUs | partial | `AV2-5.13-QUANTIZATION-MATRIX`, `AV2-5.14-FILM-GRAIN`, `AV2-5.17-METADATA` |
| 8 | Frame header child features | partial | `AV2-5.18-FRAME-HEADER` |
| 9 | Tile group and arithmetic payload boundaries | partial | `AV2-5.19-TILE-GROUP`, `AV2-5.20-TILE-GROUP-PAYLOAD` |
| 10 | Conformance vectors and AVM differential harness | todo | `CONF-AVM-DIFF-HARNESS`, `CONF-PUBLIC-VECTORS` |

This is the single forward-looking validator planning document: it owns phase
sequencing and rationale. The earlier validator planning docs were executed
and folded in here; their details live in git history. Status lines below are
coarse snapshots — per-row detail defers to the generated docs and is not
re-edited here for every matrix change. Canonical sources:

- per-feature status: [`IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml),
  rendered as the generated [`FEATURE-STATUS.md`](./FEATURE-STATUS.md)
- per-spec-section view: the generated [`SPEC-COVERAGE.md`](./SPEC-COVERAGE.md)
- emitted diagnostics: [`VALIDATOR-DIAGNOSTICS.md`](./VALIDATOR-DIAGNOSTICS.md)

## Guiding rule

Every validator feature flows through the workflow in
[`AGENTS.md`](../AGENTS.md) and [`FEATURE-TRACKING.md`](./FEATURE-TRACKING.md):
OpenSpec change -> `docs/IMPLEMENTATION-MATRIX.toml` -> code/tests/diagnostics
-> xtask proof -> generated `docs/FEATURE-STATUS.md`. No matrix stage `done`
without proof; no bare `TODO(spec)`.

## Current focus and guardrails

**Highest-leverage next work:** deepen sequence/HLS semantics and the
temporal-unit state machine before frame headers and tile groups. Sequence
state drives later parser branches; §6.2.2 activated limits and §7.3.8 HLS
availability are prerequisites for meaningful frame/tile validation; frame
header and tile group syntax depend on sequence-level dimensions, layer
limits, tool flags, dependency maps, and timing fields. The open gaps, in
dependency order:

| Gap | Feature IDs |
|---|---|
| Sequence semantics | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` (the §6.4.1 multistream slice — distinct-mlayer-count, SWITCH/RAS dependency self-containment, monotonic-output-order agreement — landed; operating-point/same-output-time residuals remain) |
| Activated sequence state | `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` |
| HLS availability | `AV2-7.3.8-HLS-AVAILABILITY` |
| Temporal-unit ordering completion | `AV2-7.3.7-TEMPORAL-UNIT-ORDER`, then §7.3.2–§7.3.6 children as parse dependencies allow (the minimal §7.3.2 CMVS tracker landed) |
| Deeper HLS semantics | the `validate = partial` HLS rows (LCR, atlas, OPS/BRT, metadata); the §6.10.7/§6.8.9/§7.3.8.7 dependency-map agreement checks, the §6.4.13/§6.10.5 signaled buffer-delay sum-constancy checks (`decoder-model/*`), and the **static** Annex A profile/level/tier value-space subset (`annex-a/*`: Table A.1 profile + Table A.7/A.8/A.9 level/tier value-space and static level limits) are landed, with the Table A.4 interoperability-point OBU-presence checks deferred to the `msdo-global-lcr-agreement` backlog change (see the Planned diagnostics backlog) and the **rate-based** Annex A/E operating-point *semantics* (decoder-schedule simulation, buffer model) still future |
| Frame-header continuation | the Phase 8 remaining work below |

**Do not start yet** as a primary task: a full tile-group payload parser,
entropy/range coding, a decoder, an encoder, a bitstream writer, or the AVM
differential harness. The active OpenSpec changes `add-bitstream-writer`,
`toy-intra-encoder-v0`, and `avm-differential-harness` are recorded intent
behind this fence, not started work — none has implementation tasks checked.
Prepare hooks and fixtures, but keep the core work focused on the gaps above.

## Phase 0 — matrix and OpenSpec hygiene

**Status:** done — matrix child rows for the large features, generated status
docs, and drift gates exist; the OpenSpec change is archived
(`2026-06-07-validator-coverage-roadmap`). Nothing open.

## Phase 1 — descriptor and payload-boundary foundation

**Status:** done — the planned descriptor and boundary rows plus
`AV2-4.11.7-SU` and `AV2-4.11.4-SVLC` landed with proptest proof. Still open:
`AV2-5.2.3-TRAILING-BITS` / `AV2-5.2.4-BYTE-ALIGNMENT` `validate` stays
`partial` until every payload parser calls the boundary helpers.

## Phase 2 — `open_bitstream_unit(sz)` payload dispatch

**Status:** done — dispatch, `PayloadStatus`, and the `inspect --json`
`payload_status` object landed with tests (`AV2-5.2.1-OBU-DISPATCH`). Raw Annex B
and IVF-wrapped Annex B inputs are accepted through the shared
`AV2-IVF-CONTAINER` stream layer. The dispatch row's `parse = partial` is the
declared honest end-state until every payload variant exists.

## Phase 3 — sequence header parser, split by §5.4 child rows

**Status:** partial — all thirteen §5.4 child rows parse with tests; remaining
work is the deeper `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` validation (the
umbrella and most child `validate` stages are `partial`).

**Goal:** implement the first real OBU payload parser and unlock
sequence-activated validation.

Umbrella: `AV2-5.4-SEQUENCE-HEADER`, plus `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`
for the semantics. Per-child status lives in the generated
[`SPEC-COVERAGE.md`](./SPEC-COVERAGE.md) and
[`FEATURE-STATUS.md`](./FEATURE-STATUS.md).

The parser lives in `crates/splot-core/src/headers/sequence.rs` (no AV1 names;
every field maps directly to AV2 syntax or an AV2-derived variable). The
`sequence-header/*` checks proposed by this phase are landed and listed in
[`VALIDATOR-DIAGNOSTICS.md`](./VALIDATOR-DIAGNOSTICS.md).

## Phase 4 — activated sequence state and remaining §6.2.2 checks

**Status:** partial — activated layer-id limits and the core HLS availability
checks landed; full §7.3.8 availability modeling remains open.

**Goal:** the validator remembers activated sequence headers and uses them to check OBU layer IDs.

Feature IDs:

- `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`
- `AV2-7.3.8-HLS-AVAILABILITY`

The state shape shipped as the crate-private `ValidatorContext` in
`crates/splot-validate/src/context.rs`, which tracks activated sequence
headers per extended layer, HLS availability, and temporal-unit state.

Landed: the activation-limit and availability checks listed under
`sequence-state/*` and `hls/*` in
[`VALIDATOR-DIAGNOSTICS.md`](./VALIDATOR-DIAGNOSTICS.md).

Landed (sequence-multistream-semantics): the §7.3.6 single-active-sequence-header
rule `hls/multiple-active-sequence-headers` (cited at §7.3.6, tracked by
`AV2-7.3.8-HLS-AVAILABILITY`), firing on a frame-confirmed activation of a
different `seq_header_id` within the same coded video sequence.

Remaining:

- full §7.3.8 availability modeling (MSDO/OPS availability records and the
  global atlas reference remain deferred).

Acceptance:

- Unit tests with one sequence header followed by a violating OBU.
- Unit tests with parseable prefix + later error retaining both stateful and parse diagnostics.
- No global mutable state.

## Phase 5 — OBU ordering and temporal-unit state machine

**Status:** partial — the core §7.3.7 temporal-unit ordering and §7.3.6
extended-layer ordering landed; a minimal §7.3.2 coded-multistream-video-sequence
(CMVS) begin/end tracker landed (no `cmvs/*` diagnostics yet — it scopes the
§6.4.1 `monotonic_output_order_flag` agreement check); the §7.3.3–§7.3.5
coded-frame-unit rows and §7.3.9 long-term-reference availability are not
started.

**Goal:** enforce temporal-unit and coded-extended-layer presence order enough for validator-first conformance.

Feature IDs:

- `AV2-7.3-OBU-ORDERING` umbrella, already present.
- Child rows (all in the matrix):
  - `AV2-7.3.2-CMVS-BOUNDARIES`
  - `AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT`
  - `AV2-7.3.4-CODED-NONOUTPUT-FRAME-UNIT`
  - `AV2-7.3.5-CODED-FRAME-UNIT`
  - `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`
  - `AV2-7.3.7-TEMPORAL-UNIT-ORDER`
  - `AV2-7.3.8-HLS-AVAILABILITY`
  - `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY`

Landed: the §7.3.7 ordering checks listed under `obu-order/*` in
[`VALIDATOR-DIAGNOSTICS.md`](./VALIDATOR-DIAGNOSTICS.md) — delimiter presence
and duplication, global-HLS position, ascending xlayer order, and padding
globality.

Remaining:

- global metadata prefix/suffix positions (pending frame/tile parsing; see
  the backlog table below);
- the `cmvs/*` boundary-ordering diagnostics on `AV2-7.3.2-CMVS-BOUNDARIES`
  (the minimal begin/end tracker landed; only the monotonic-output-order check
  consumes it so far) and the coded-frame-unit rows
  (`AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT`,
  `AV2-7.3.4-CODED-NONOUTPUT-FRAME-UNIT`, `AV2-7.3.5-CODED-FRAME-UNIT`).

Follow-up in this phase: §7.4 random access decoding. Validate random access
points enough to support HLS availability, coded-video-sequence boundaries, and
long-term-reference preconditions (§7.4.2 covers random access with and without
long-term reference frames). Closely coupled to
`AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` and the `AV2-5.6-MSDO`
random-access-point detection bound.

Acceptance:

- Small synthetic streams for valid/invalid ordering.
- Inspector output groups OBUs by temporal unit where possible.
- Matrix child rows prevent the umbrella from pretending complete coverage.

## Phase 6 — high-level syntax OBUs

**Status:** partial — every Phase 6 row parses with tests and a dedicated
parser module, the §6.10.7/§6.8.9/§7.3.8.7 layer-dependency-map agreement
checks are landed (`ops/*-dependency-missing`, `lcr/*-dependency-missing`,
`frame-header/mfh-*-dependency-missing`), and the §6.4.13/§6.10.5 buffer-delay
sum-constancy checks landed as a resolved two-tier outcome
(`decoder-model/buffer-delay-sum-changed` error for the intra-CVS OPS case that is
non-conforming under every "video sequence" reading, plus the advisory
`decoder-model/buffer-delay-sum-changed-across-cvs` warning for the
broad-reading-only seq-header / cross-boundary cases — AVM parses but never
enforces these values, so there is no differential oracle); the **static** Annex A
profile/level/tier subset has also landed (`annex-a/*`: Table A.1 profile
value-space, Table A.7/A.9 level/tier value-space, and the Table A.8/A.9 static
level-limit checks on the parsed intra frame path; the Table A.4
interoperability-point OBU-presence checks are deferred to the
`msdo-global-lcr-agreement` backlog change), so the table data in
`crates/splot-validate/src/annex_a.rs` is transcribed verbatim from the mirror;
remaining work is deeper semantic validation across the board — the **rate-based**
Annex A/E operating-point semantics (MaxDisplayRate/MaxDecodeRate, buffer model)
and decoder-schedule simulation are still future.

**Goal:** parse HLS OBUs referenced by sequence/frame validation and OBU ordering.

Feature IDs:

- `AV2-5.6-MSDO`
- `AV2-5.7-MULTI-FRAME-HEADER`
- `AV2-5.8-LAYER-CONFIG-RECORD`
  - `AV2-5.8.1-LCR-GLOBAL-INFO`
  - `AV2-5.8.2-LCR-LOCAL-INFO`
  - `AV2-5.8.3-LCR-AGGREGATE-INFO`
  - `AV2-5.8.4-LCR-SEQ-PTL-INFO`
  - `AV2-5.8.5-LCR-GLOBAL-PAYLOAD`
  - `AV2-5.8.6-LCR-XLAYER-INFO`
  - `AV2-5.8.7-LCR-REP-INFO`
  - `AV2-5.8.8-LCR-EMBEDDED-LAYER-INFO`
  - `AV2-5.8.9-LCR-XLAYER-COLOR-INFO`
- `AV2-5.9-ATLAS-SEGMENT`
- `AV2-5.10-OPERATING-POINT-SET`
  - `AV2-5.10-OPS-SYNTAX-ELEMENTS`
- `AV2-5.11-OPERATING-POINT-PAYLOAD`
  - `AV2-5.11.1-OPS-AGGREGATE-INFO`
  - `AV2-5.11.2-OPS-SEQ-PTL-INFO`
  - `AV2-5.11.3-OPS-DECODER-MODEL-INFO`
  - `AV2-5.11.4-OPS-COLOR-INFO`
  - `AV2-5.11.5-OPS-MLAYER-INFO`
- `AV2-5.12-BUFFER-REMOVAL-TIMING`

Prioritize HLS availability and layer mapping:

1. MSDO: substream/xlayer map and random-access availability.
2. LCR: global/local ids, layer maps, sequence-header references.
3. OPS: maps and payload size consistency.
4. Atlas: id and LCR relationship checks.
5. Buffer removal timing: decoder-model hooks.
6. Multi-frame header: prerequisites for frame header reuse.

Acceptance:

- Each OBU has a dedicated parser module and validator checks.
- Every check has a stable diagnostic ID and spec section.
- External availability is modeled explicitly but disabled by default unless the CLI/API supplies external HLS OBUs.

## Phase 7 — non-HLS payload OBUs

**Status:** partial — `AV2-5.15-CONTENT-INTERPRETATION` and `AV2-5.16-PADDING`
have parse/validate/tests done (their write/encode stages are encoder scope,
tracked with the `ENC-*` matrix rows); quantizer matrix, film grain, and the
§5.17 metadata family parse with tests. The §6.16.3 metadata lifetime store
and the §6.16.10 scan-type CVS-consistency checks landed
(`metadata-semantic-validation`; `AV2-5.17.3-METADATA-GROUP` and
`AV2-5.17.10-METADATA-SCAN-TYPE` are `validate = done`), while the metadata
umbrella and the remaining family rows keep `validate = partial`
(decoded-frame-hash is decoder-blocked, in-frame-unit placement is
frame-parsing-blocked).

**Goal:** parse and validate payload OBUs that are not the full frame/tile syntax yet.

Feature IDs:

- `AV2-5.13-QUANTIZATION-MATRIX`
- `AV2-5.14-FILM-GRAIN`
- `AV2-5.15-CONTENT-INTERPRETATION`
- `AV2-5.16-PADDING`
- `AV2-5.17-METADATA`
- metadata child rows for §5.17.1 through §5.17.13

Landed: the locally-decidable checks listed under `padding/*`, `metadata/*`,
`film-grain/*`, `qm/*`, and `content-interpretation/*` in
[`VALIDATOR-DIAGNOSTICS.md`](./VALIDATOR-DIAGNOSTICS.md).

Remaining: the stateful/cross-OBU semantics behind the `validate = partial`
rows (`AV2-5.13-QUANTIZATION-MATRIX`, `AV2-5.14-FILM-GRAIN`,
`AV2-5.17-METADATA` and its child rows). The metadata portion that was
validator-achievable landed via the archived `metadata-semantic-validation`
change; the residuals are decoder-blocked (§6.16.13 decoded-frame-hash) or
frame-parsing-blocked (§7.3.3/§7.3.4 in-frame-unit placement) and stay
tracked by their matrix rows.

Acceptance:

- synthetic fixtures for each OBU type;
- `inspect` can show parsed fields in JSON;
- strict validation fails on unsupported payload syntax once corresponding matrix row is `partial`/`done`.

## Phase 8 — frame header child features

**Status:** partial — beyond the activation skeleton,
`AV2-5.18.3-FRAME-CONFIGURATION` and `AV2-5.18.4-FRAME-SIZE` parse with tests,
and the intra tail through `AV2-5.18.6-QUANTIZATION` and
`AV2-5.18.7-SEGMENTATION-TILING` is partial (`AV2-5.18.7.3-TILE-PARAMS` done);
`AV2-5.18.5-FILTERING` and the §5.18.8–§5.18.10 child rows are todo.

**Goal:** split the large frame header into implementable chunks.

- **Landed** (archived OpenSpec `frame-activation-hls-skeleton` and
  `frame-tiling-quant-segmentation`): the prefix-only frame-activation
  skeleton plus the intra tail through tile/quantization/segmentation/QM/
  delta-q parameters, stopping before § 5.18.5.2; this enables
  `hls/unavailable-sequence-header`, `hls/unavailable-multi-frame-header`,
  `frame-header/tile-cols-out-of-range`, `frame-header/tile-rows-out-of-range`,
  `frame-header/context-update-tile-id-out-of-range`, and
  `frame-header/qm-plane-count-mismatch`.
- **Remaining:** inter frame-header paths, the MFH-gated branches
  (`cur_mfh_id > 0` stops as unsupported), the § 5.18.7.4 non-uniform
  sequence-reuse branch, § 5.18.5 filtering onward, and the § 6.17.6.2
  layer-dependency constraints (the §5.4.1 dependency maps are now exposed by
  the sequence-header model; the checks themselves are not implemented yet).
  `AV2-5.18-FRAME-HEADER` and `AV2-5.19-TILE-GROUP` therefore stay `partial`,
  not `done`.

Umbrella: `AV2-5.18-FRAME-HEADER`. The §5.18.1–§5.18.10 child rows and the
matching §6.17 semantics rows live in the matrix; see the generated
[`SPEC-COVERAGE.md`](./SPEC-COVERAGE.md) and
[`FEATURE-STATUS.md`](./FEATURE-STATUS.md) for per-child status.

Rules:

- Frame header implementation must depend on parsed sequence header state.
- Avoid introducing a decoder unless a check truly requires it.
- Use AVM traces/differential testing as soon as fixture streams are available.

## Phase 9 — tile group and arithmetic payload boundary validation

**Status:** partial — only the `tile_group_obu()` prefix from the Phase 8
activation skeleton landed; `AV2-5.20-TILE-GROUP-PAYLOAD` and the
arithmetic-boundary targets are untouched.

**Goal:** validate tile-group structure without prematurely promising a complete decoder.

Feature IDs:

- `AV2-5.19-TILE-GROUP`
- `AV2-5.20-TILE-GROUP-PAYLOAD`
- child rows for §5.20.1-§5.20.10 as needed.

Initial target:

- validate tile group header/size fields;
- validate arithmetic coder entry/exit boundaries;
- validate `exit_symbol` / trailing-bit interactions;
- leave pixel-reconstruction-dependent checks as explicit child rows.

Acceptance:

- malformed tile payloads return diagnostics, not panics;
- valid AVM-generated streams pass header/payload-boundary checks;
- incomplete decoding constraints are tracked as child rows, not hidden.

## Phase 10 — conformance vectors and AVM differential harness

**Status:** todo — mapping-only; `cargo xtask conformance` is an explicit
stub. An active OpenSpec change (`avm-differential-harness`) plans the
harness.

**Goal:** turn validator confidence into reproducible external proof.

Feature IDs:

- `CONF-AVM-DIFF-HARNESS`
- `CONF-PUBLIC-VECTORS`
- `CONF-INSPECT-SNAPSHOTS`

Work items:

- `cargo xtask conformance --avm-bin <path> --input <stream>`;
- parser trace comparison mode;
- store failing bitstreams under an ignored local corpus and optionally add minimized redistributable fixtures;
- document where public AV2 vectors can be fetched and their license status;
- never require AVM in normal CI until the maintainer opts in.

## Planned diagnostics backlog

Diagnostics proposed in earlier phase plans that have **not** landed yet. The
canonical registry of emitted IDs is
[`VALIDATOR-DIAGNOSTICS.md`](./VALIDATOR-DIAGNOSTICS.md); when one of these
lands, add it to the registry tables there (the CI gate will require it).

| Planned ID | Severity | Section | Feature | Trigger |
|---|---|---|---|---|
| `obu-order/global-hls-after-metadata-suffix` | error | §7.3.7 | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | Global HLS appears after suffix metadata. Needs global suffix-metadata classification, which is pending frame/tile parsing. |
| `obu-order/non-global-hls-before-coded-layer` | error | §7.3.7 | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | Non-global HLS appears in an invalid temporal-unit region. |
| `msdo/sub-xlayer-duplicate` | error | §6.6 | `AV2-5.6-MSDO` | Duplicate `sub_xlayer_id` where uniqueness is required. Add only after confirming the exact §6.6 wording in the spec mirror (spec honesty). |
| `brt/global-ordering-position` | error | §7.3.7 | `AV2-7.3-OBU-ORDERING` | A global BRT in an invalid temporal-unit position. Deferred: §7.3.7 does not list BRT among the global prefix OBUs, so a hard ordering error needs the §7.3.8 decoder-model / random-access state (tracked by a spec TODO under `AV2-7.3-OBU-ORDERING` in `splot-validate`). |
| `annex-a/msdo-required-for-iop` | error | §A.2 | `AV2-A-PROFILES` | A Table A.4 interoperability-point row requires an OBU_MSDO (or the IOP2 MSDO-or-global-LCR either-or) for a multi-extended-layer coded video sequence and none is present. Deferred from `annex-a-profile-level-skeleton`; re-lands with `msdo-global-lcr-agreement` once MSDO aggregate-profile (`multistream_profile_idc`) state, LCR activation state, and §7.3.6-correct per-TU window attribution exist (the PR #46 codex threads record the requirements). |
| `annex-a/msdo-prohibited-for-iop` | error | §A.2 | `AV2-A-PROFILES` | A Table A.4 interoperability-point row prohibits an OBU_MSDO (a single-extended-layer coded video sequence) but one is present. Deferred from `annex-a-profile-level-skeleton`; re-lands with `msdo-global-lcr-agreement` once MSDO aggregate-profile (`multistream_profile_idc`) state, LCR activation state, and §7.3.6-correct per-TU window attribution exist (the PR #46 codex threads record the requirements). |
| `annex-a/lcr-required-for-iop` | error | §A.2 | `AV2-A-PROFILES` | A Table A.4 interoperability-point row requires a local/global OBU_LAYER_CONFIGURATION_RECORD (or the MSDO-plus-local-LCR / global-LCR either-or) that is absent in the coded video sequence. Deferred from `annex-a-profile-level-skeleton`; re-lands with `msdo-global-lcr-agreement` once MSDO aggregate-profile (`multistream_profile_idc`) state, LCR activation state, and §7.3.6-correct per-TU window attribution exist (the PR #46 codex threads record the requirements). |
| `annex-a/frame-exceeds-ops-level` | error | §A.4/§6.10.4 | `AV2-A-LEVELS-TIERS` | A frame's size / tile geometry exceeds the Annex A.4 Table A.8/A.9 limits for an *operating-point-advertised* level: §6.10.4 requires the operating point's bitstream to satisfy A.4 with `seq_level_idx` set to the OPS-signaled `ops_level_idx`, so the static level-limit checks must also run against OPS levels, not only the activated `seq_level_idx`. Deferred from `annex-a-profile-level-skeleton`: blocked on operating-point-to-frame mapping (which frames belong to which operating point), which the validator does not model yet — `check_ops_level_tier_value_space` currently only checks the OPS-carried value space (reserved-level / high-tier), not per-frame conformance against the advertised level. |

Naming rules live in [`FEATURE-TRACKING.md`](./FEATURE-TRACKING.md) § 12
(Diagnostic-ID convention); never rename a landed ID without a migration note.

## Done criteria for the umbrella validator goal

The validator can be called “full syntax validator” only when:

- every §5 syntax row is `parse = done` and `tests = done`;
- every locally checkable §6 semantic row is `validate = done`;
- every stateful §7.3/HLS availability row has either `validate = done` or an explicitly documented blocked dependency;
- Annex A/E conformance rows are represented and implemented to the extent their required syntax exists;
- malformed data never panics under unit tests, proptests, and fuzzing;
- AVM/public-vector proof exists for representative valid and invalid streams.
