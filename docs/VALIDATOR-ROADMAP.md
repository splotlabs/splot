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
| Deeper HLS semantics | the `validate = partial` HLS rows (LCR, atlas, OPS/BRT, metadata); the §6.10.7/§6.8.9/§7.3.8.7 dependency-map agreement checks, the §6.8.5 LCR PTL-ceiling and §6.8.8 LCR rep-info equality checks against the activated sequence header (`lcr/ptl-*-exceeds-max`, `lcr/rep-info-mismatch`; `lcr-ptl-activated-sequence-agreement`), the §6.4.13/§6.10.5 signaled buffer-delay sum-constancy checks (`decoder-model/*`), and the **static** Annex A profile/level/tier value-space subset (`annex-a/*`: Table A.1 profile + Table A.7/A.8/A.9 level/tier value-space and static level limits) are landed, with the Table A.4 interoperability-point OBU-presence checks deferred to the `msdo-global-lcr-agreement` backlog change (see the Planned diagnostics backlog), the LCR-declared PTL maxima Annex A value-space range checks (`lcr_seq_profile_idc[i]`/`lcr_max_level_idx[i]`/etc.) still on the Annex A table backlog, and the **rate-based** Annex A/E operating-point *semantics* (decoder-schedule simulation, buffer model) still future |
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
`AV2-IVF-CONTAINER` stream layer. Under `obu-dispatch-frame-payloads` the 11
frame-carrying types (the tile-group family and the SEF/TIP/bridge family) no
longer return a blanket `Unimplemented`: the stateless dispatcher parses their
state-free §5.18.2 / §5.19 activation prefix and returns
`PayloadStatus::PrefixParsed` with `blocked_on = "active sequence header state"`,
surfaced by `inspect --json` as `prefix_parsed_awaiting_state` (the richer
state-aware surface is the inspector's stateful frame-header views and the
validator's direct-call path). The dispatch row's `parse = partial` is the
declared honest end-state until the deeper §5.18 / §5.19 / §5.20 syntax beyond
the prefix exists (owned by the frame-header / tile-group rows).

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

Landed (rap-availability-replay): the §7.3.8.1 random-access-point HLS
availability replay `hls/unavailable-at-random-access-point` (tracked by
`AV2-7.3.8-HLS-AVAILABILITY`), firing when a sequence header / multi-frame
header / operating point set referenced at or after a §7.4.1 random access point
(CLK/OLK/RAS temporal unit) was not (re)sent in or after that point's temporal
unit — with LEADING_* temporal-unit resends disqualified, undecidable leading
frames left qualifying (sound under-approximation), per-(object, random access
point) dedup, and per-key external-HLS suppression: for a declarable kind
(sequence header / operating point set) the `ExternalHlsSet` is authoritative, so
suppression requires the *exact* referenced key to be declared external; for a
kind the set cannot express (multi-frame header / LCR / atlas) any `Provided` mode
keeps the blanket suppression (`options.rs`). This
removes the `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` "blocked on §7.3.8
availability" framing (the §6.2.2 post-activation-window precision follow-up
stays on that row).

Remaining:

- the §7.3.8.1 film-grain / quantization-matrix frame references await
  frame-header parsing; the §7.3.8.2 first-sentence MSDO availability-at-RAP is
  interop-point-gated (Annex A.2, tracked on `AV2-5.6-MSDO`); the global atlas
  reference (§7.3.8.4 "can be available") and the `ExternalHlsSet`
  MFH/LCR/atlas declaration keys remain named non-goals.

Acceptance:

- Unit tests with one sequence header followed by a violating OBU.
- Unit tests with parseable prefix + later error retaining both stateful and parse diagnostics.
- No global mutable state.

## Phase 5 — OBU ordering and temporal-unit state machine

**Status:** partial — the core §7.3.7 temporal-unit ordering and §7.3.6
extended-layer ordering landed; a §7.3.2 coded-multistream-video-sequence
(CMVS) begin/end tracker landed (it scopes the §6.4.1
`monotonic_output_order_flag` agreement check) and the §7.3.2 boundary-set
identity check landed as the first `cmvs/*` diagnostic
(`cmvs/boundary-set-mismatch`, decidable-disagreement-only); the §7.3.3–§7.3.5
coded-frame-unit **segmentation** landed under `frame-unit-segmentation`
(`crates/splot-validate/src/frame_unit.rs`): each `(xlayer, mlayer, tlayer)`
triple's OBUs partition into coded frame units with the §7.3.3/§7.3.4 region
order, output/non-output classification from the parsed output flags (Unknown
→ silent), and the eager structural `frame-unit/*` presence-order diagnostics
plus the §7.3.8.10 first-coded-frame-unit CI rule; the two formerly-backlogged
§7.3.7/§7.3.6 ordering rows (`obu-order/global-hls-after-metadata-suffix`,
`obu-order/non-global-hls-before-coded-layer`) landed with it. The §7.3.6
coded-extended-layer-unit constraint family and the §7.3.7/§7.4.6
display-order-hint (DOH) constraints landed under `celu-orderhint-constraints`
(`crates/splot-validate/src/celu.rs`, the `CodedExtendedLayerTracker` above the
segmenter, keyed per `obu_xlayer_id`): the `celu/*` in-unit ordering,
output-frame presence, same-OrderHint, CLK/OLK first-unit and lowest-layer,
no-CLK+OLK-mix, all-leading-or-none, and CELU-scoped CI rules, plus the
flag-gated `celu/doh-order-hint-mismatch` / `celu/doh-order-hint-bits-mismatch`
checks. §7.3.9 long-term-reference availability is partial
(`reference-state-and-random-access`): the per-slot `RefLongTermId` model and the
§6.17.2 RAS `long_term_id_in_use` check (`frame-header/ras-ref-long-term-id-not-in-use`)
landed; the §7.3.9.1 general availability + the RAP-CELU CLK-then-OLK ordering remain
residuals (need a long-term RAP-replay key).

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
and duplication, global-HLS position, ascending xlayer order, padding
globality, global-HLS-after-metadata-suffix, and non-global-HLS-before-coded-layer
— plus the §7.3.3–§7.3.5 coded-frame-unit segmentation and its `frame-unit/*`
diagnostics, and the §7.3.6 coded-extended-layer-unit constraint family and
§7.3.7/§7.4.6 DOH OrderHint/OrderHintBits checks under `celu/*`. The `celu/*`
predicates are disjoint from `obu-order/non-global-hls-before-coded-layer` (which
owns the HLS-header-after-frame-region case) and from
`frame-unit/ci-not-in-first-frame-unit` (the §7.3.8.10 temporal-unit CI form vs
the §7.3.6 CELU-scoped `celu/content-interpretation-not-in-first-unit`).

Remaining:

- the remaining `cmvs/*` boundary-ordering diagnostics on
  `AV2-7.3.2-CMVS-BOUNDARIES` that depend on the §7.3.3–§7.3.5 frame-unit
  segmentation (the §7.3.2 boundary-set identity check landed as
  `cmvs/boundary-set-mismatch`; the segmentation foundation they need now
  exists in `frame_unit.rs`, but the ordering diagnostics themselves are not
  yet wired);
- the Unknown-path residual of the coded-frame-unit rows: a coded frame whose
  output classification is undecidable (an unsupported `FrameHeaderParseStatus`
  stop, blocked on `AV2-5.18-FRAME-HEADER`) drops its output-class-derived
  judgment (the grammar branch and the §7.3.4 BRT bound), narrowing as
  frame-header parsing lands;
- the §7.3.6 monotonic_output_order_flag==0 OrderHint regression rule's full
  form (mirror lines 579–584) stays a named residual on
  `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`: it needs output-order and MSB-extended
  OrderHint state (`AV2-5.18.2-FRAME-HEADER-INFO`); the §7.3.6 bit-identity
  cross-layer fingerprint baseline ships as
  `hls/repeated-sequence-header-not-identical` and the §6.4.1 same-output-time /
  operating-point-consistency residuals are documented-blocked on
  `AV2-E-DECODER-MODEL` / `AV2-5.18.2-FRAME-HEADER-INFO` (see
  `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`).

Follow-up in this phase: §7.4 random access decoding. Validate random access
points enough to support HLS availability, coded-video-sequence boundaries, and
long-term-reference preconditions (§7.4.2 covers random access with and without
long-term reference frames). Closely coupled to
`AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` and the `AV2-5.6-MSDO`
random-access-point detection bound. **Partial** (`reference-state-and-random-access`,
umbrella `AV2-7.4-RANDOM-ACCESS`): the §7.4.5 RAS reference restriction landed (via the
§6.17.2 `long_term_id_in_use` check + the per-slot `RefLongTermId` model) and the
§7.3.8.9 `reset_qm()` QmProtected discipline the §7.4.3/.4/.5 processes invoke landed
(`frame-header/qm-level-unavailable`). Residuals: the §7.4.2 preconditions, the §7.4.4
OLK `ref_long_term_id` iff-conditions, the §7.3.9.1 RAP-CELU ordering, and the
§7.4.4/.5 `OrderHint < (1<<OrderHintBits)` bound (not header-decidable — the unwrapped
`get_disp_order_hint()`).

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
and the intra tail through `AV2-5.18.6-QUANTIZATION`,
`AV2-5.18.7-SEGMENTATION-TILING` (`AV2-5.18.7.3-TILE-PARAMS` done; `gdf_params()`
§5.18.7.9, `cdef_params()` §5.18.7.10, `lr_params()` §5.18.7.11, and
`ccso_params()` §5.18.7.12 parse on the intra path),
`AV2-5.18.5-FILTERING` (`deblocking_filter_params()` §5.18.5.2 on the intra path),
and the §5.18.8–§5.18.10 tail (`AV2-5.18.8-TRANSFORM-CODING-MODES`,
`AV2-5.18.9-GLOBAL-MOTION` intra arm, `AV2-5.18.10-FILM-GRAIN-STRUCTURES`) all
parse — the **complete** intra frame header reaches the `IntraHeaderComplete`
terminal and the show-existing-frame path reaches `ShowExistingFrameComplete`.
The inter-path arms of every child remain partial/todo.

**Goal:** split the large frame header into implementable chunks.

- **Landed** (archived OpenSpec `frame-activation-hls-skeleton` and
  `frame-tiling-quant-segmentation`): the prefix-only frame-activation
  skeleton plus the intra tail through tile/quantization/segmentation/QM/
  delta-q parameters, stopping before § 5.18.5.2; this enables
  `hls/unavailable-sequence-header`, `hls/unavailable-multi-frame-header`,
  `frame-header/tile-cols-out-of-range`, `frame-header/tile-rows-out-of-range`,
  `frame-header/context-update-tile-id-out-of-range`, and
  `frame-header/qm-plane-count-mismatch`.
- **Landed** (OpenSpec `mfh-frame-header-state`): the intra `cur_mfh_id > 0`
  path now resolves the in-band multi-frame header's parsed § 5.7 state (carried
  on `MultiFrameHeaderRecord`) into the core parse, so § 5.18.4.1 default
  dimensions come from `mfh_frame_width/height_minus_1[ cur_mfh_id ]` (with the
  § 5.18.2 omitted-size inference) and § 5.18.7.1 `segmentation_params()` parses
  its MFH-gated arm — the same intra tail the `cur_mfh_id == 0` path reaches. A
  `cur_mfh_id > 0` frame whose in-band MFH is unresolvable still routes to
  Unknown.
  The § 5.18.7.4 non-uniform sequence-reuse branch is now wired: § 5.4.2 parsing
  records `SeqSbColStarts` / `SeqSbRowStarts`, so a frame reusing a non-uniform
  sequence tile layout parses through `tile_info()` instead of stopping
  unimplemented.
- **Landed** (OpenSpec `frame-filtering-deblocking-gdf-cdef`): the intra-path
  stop advances past the § 5.18.2 tail loop-filter cluster —
  `deblocking_filter_params()` (§ 5.18.5.2, including the `cur_mfh_id > 0`
  `mfh_deblocking_filter_update` / `mfh_apply_deblocking_filter` arm),
  `gdf_params()` (§ 5.18.7.9), and `cdef_params()` (§ 5.18.7.10) — so the core
  stop status was then `StoppedBeforeLoopRestorationParams`. A payload that ends
  *inside* the loop-filter cluster is reported through the dedicated
  `StoppedInsideFilterParams` status: the already-parsed control-region facts
  (frame size, output flags, tile / quant / segmentation) are preserved so the
  validator's state-supported checks (e.g.
  `frame-header/frame-size-exceeds-sequence-max`) still fire on a truncated frame,
  rather than the EOF failing the whole core parse and silently skipping them.
  §6.17.5.2 / §6.17.7.5 / §6.17.7.6 state no requirement of bitstream conformance
  on the parsed fields, so no diagnostic was added.
- **Landed** (OpenSpec `frame-loop-restoration-ccso-params`): the intra-path stop
  advances past `lr_params()` (§ 5.18.7.11) and `ccso_params()` (§ 5.18.7.12), so
  the terminal intra stop status is now `StoppedBeforeReadTxMode` (the next
  unparsed structure is `read_tx_mode()`, § 5.18.8.1). On the intra path
  `NumTotalRefs == 0`, so `lr_params()`'s temporal-prediction arm and
  `ccso_params()`'s reuse arm are dead. When an `lr_params()` plane signals a
  frame-level Wiener filter, the parser stops honestly with
  `StoppedBeforeWienerNsFilter` before the unmodeled `read_wienerns_filter()` bank
  decode. § 6.17.7.8 yields two locally decidable diagnostics:
  `frame-header/ccso-ext-filter-reserved` (`ccso_ext_filter != 7`) and
  `frame-header/ccso-max-band-out-of-range` (`1 << ccso_max_band_log2 <=
  CCSO_BAND_NUM`). The § 6.17.7.7 lr size / RU-divisibility bounds and the
  reference-state CCSO requirements remain deferred (the reuse/ref-state arm is
  dead on the intra path).
- **Landed** (OpenSpec `frame-header-intra-tail-completion`): the intra path now
  parses to completion. After `ccso_params()` the § 5.18.2 tail
  (`crates/splot-core/src/headers/frame/tail.rs`) reads `read_tx_mode()`
  (§ 5.18.8.1, `tx_mode_select` gated on the derived `CodedLossless`), the no-bit
  intra inferences of `frame_reference_mode()` (§ 5.18.8.3) / `skip_mode_params()`
  (§ 5.18.8.2) / `allow_bawp` / `allow_warpmv_mode`, `reduced_tx_set` `f(2)`, the
  no-bit intra arm of `global_motion_params()` (§ 5.18.9.1), and
  `film_grain_config()` (§ 5.18.10.1 — `apply_grain` / `fgm_id` / `grain_seed`;
  `load_grain_model()` reads no bits per § 6.17.10.1, so the § 5.14 model parser is
  not invoked here). The terminal status is `IntraHeaderComplete`; the
  show-existing-frame path reaches `ShowExistingFrameComplete` after its
  `film_grain_config()`. A payload that ends inside the tail reports the dedicated
  `StoppedInsideIntraTail` status, preserving the loop-filter-cluster facts. The
  `frame-header-core.av2` / `frame-header-core-mfh.av2` fixtures now parse to
  `intra_header_complete`. No new diagnostic was added: § 5.18.8 / § 5.18.9 /
  § 5.18.10.1 state no requirement of bitstream conformance on the parsed intra
  fields, and the fgm_id HLS-availability checks need the cross-OBU film-grain
  model store (deferred).
- **Landed** (OpenSpec `frame-header-copy-bit-accounting`): the § 5.18.1
  `NumFrameHeaderBits` accounting and the `frame_header( isFirst == 0 )` ==
  `frame_header_copy()` path are now modeled for completed-intra first headers. When a
  first tile group's `frame_header_info()` reaches `IntraHeaderComplete`, the core's
  `consumed_bits` is `NumFrameHeaderBits` (mirror :3924), and `splot-core` records the
  exact first-header bits (`RecordedFrameHeaderBits`, starting after the
  `tile_group_obu()` `is_first_tile_group` flag per § 6.17.1 mirror :4303-4305) and
  parses a non-first tile group's `frame_header_copy()` as exactly that many
  `header_bit` `f(1)` reads, comparing bit-for-bit (`parse_frame_header_copy`,
  `crates/splot-core/src/headers/tile_group.rs`). The validator
  (`observe_frame_header_copy` / `check_frame_header_copy`,
  `crates/splot-validate/src/context.rs`) pairs a non-first tile group with ITS coded
  frame's first tile group using the `FrameUnitSegmenter` boundary authority
  (`OpensNewUnit` records, `ContinuesUnit` compares, `Ambiguous` drops), keyed per
  `(xlayer, mlayer, tlayer)` and cleared at each temporal delimiter. § 6.17.1 makes
  bit-identity a requirement of bitstream conformance (mirror :4299-4300), surfaced as
  `frame-header/copy-bits-mismatch`; a copy region shorter than `NumFrameHeaderBits` is
  a § 5.18.1 / § 6.2.1 truncation, `frame-header/copy-bits-truncated`. An incomplete /
  coverage-stopped / unresolvable first header records nothing (the copy region stays
  unparsed, Unknown routing). This gives non-first tile groups an EXACT header/tile-data
  boundary. The FIRST tile group's header/tile-data boundary is now EXACT on the
  INTRA-COMPLETE path too (`tile-group-structure-completion`, Phase 9): once the first
  tile group reaches `IntraHeaderComplete`, `use_bru`/`bru_inactive` are the § 5.18.2
  derived constant 0 (mirror :4127-4129 / :4653), so `parse_tile_group_structure` consumes
  the § 5.19 remainder (tg range, `byte_alignment()`, the `headerBytes` payload boundary).
  `inspect` surfaces the copy region's presence on non-first tile groups
  (`frame_header_copy` view) and the § 5.19 structure on the intra-complete first tile group
  (`tile_group_structure` view).
- **Landed** (OpenSpec `frame-reference-state-model`,
  `AV2-7.23-REFERENCE-FRAME-UPDATE`): the § 7.23 reference-frame buffer state is now
  MODELED for intra streams, replacing the always-unknown `FrameReferenceStateView`
  placeholder. A per-extended-layer `ReferenceStateTracker`
  (`crates/splot-validate/src/reference_state.rs`) holds `NUM_REF_FRAMES` slots, each
  `Unknown` / `ProvenInvalid` / `Valid{RefOrderHint, dims}`, updated at each completed
  frame's segmenter-authoritative coded-frame boundary (deferred per § 7.23's
  decode-frame-wrapup ordering, with the end-of-bitstream flush) from the parsed
  `refresh_frame_flags` / `OrderHint` / dimensions and the § 7.23 key/switch `first`
  RefValid rule. A CLK at `FirstPictureInTU` grounds the § 5.18.2 reset
  (`RefValid[i] = 0` over `0..NumRefFrames`, mirror :4449-4455) then re-applies its
  refresh; a frame whose refresh mask is not parsed (inter / TIP / bridge / truncated /
  ambiguous-boundary) honestly poisons ALL slots; a mid-stream join starts all-`Unknown`
  (sound under-approximation). The modeled buffer gives the § 6.17.2
  show-existing-frame slot-validity check
  (`frame-header/show-existing-frame-invalid-slot`): a SEF (`refresh_frame_flags = 0`,
  no update, but displays `frame_to_show_map_idx`) referencing a slot the buffer PROVES
  invalid fires; a poisoned (Unknown) slot drops to silence (the Unknown invariant). The
  modeled view is threaded into the core parse via `FrameReferenceStateView::from_slots`
  — forward plumbing for the § 5.18 INTER reference paths (no intra branch consumes it
  today). No external-HLS suppression: reference buffers are written only in-band by the
  § 7.23 process.
- **Remaining:** the § 5.18 INTER reference-path CONSUMERS of the modeled reference state
  (the `explicit_ref_frame_map` / `ref_frame_idx` `RefValid` checks § 6.17.2 :4605-4607,
  `frame_size_with_refs`, `primary_ref_frame` range, the `use_bru` OrderHint/dim
  constraints § 6.17.2 :4587-4596) await inter-path parsing; the § 7.3.9 long-term
  reference availability (`AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY`) is now **partial** —
  `reference-state-and-random-access` added the per-slot `RefLongTermId` to `SlotFacts` (from
  a KEY frame's `long_term_id_plus_1 - 1`) and the § 6.17.2 RAS `long_term_id_in_use` check;
  the § 7.3.9.1 general availability + the § 7.4.4 OLK `ref_long_term_id` iff-conditions
  remain residuals. The § 6.17.2
  `derive_sef_order_hint` already-shown / `RefImplicitOutputFrame` /
  `RefImmediateOutputFrame` SEF constraints (mirror :4375-4380) need output-frame-buffer
  / shown state this phase does not model.
- **Remaining:** the § 5.20 `tile_group_payload()` body (`AV2-5.20-TILE-GROUP-PAYLOAD`),
  the INTER-path BRU arms of `tile_group_obu()` (the `bru_inactive` `headerBits` /
  `remainingBits` `trailing_bits()` early-return and the `use_bru` `bru_tile_active` loop,
  reachable only once the inter frame-header path derives `use_bru`/`bru_inactive`), and the
  cross-tile-group continuity / last-group `tg_end == NumTiles - 1` § 6.18 clauses (need
  prior-tile-group state threaded through the segmenter). Also remaining: the
  `read_wienerns_filter()` frame-level Wiener bank decode,
  the inter frame-header paths (including the inter § 5.18.8 coding-mode arms and the inter
  `cur_mfh_id > 0` arms; the § 5.18.9 inter global-motion arm — `use_global_motion`, the
  `our_ref ns(NumTotalRefs+1)` base selection, and the full § 5.18.9.2-.6 subexp decode
  chain — is now modeled as the standalone `global_motion.rs` structure parser
  (`AV2-5.18.9-GLOBAL-MOTION`), but its per-reference warp loop stops honestly on the
  cross-frame `OrderHints` / `RefNumTotalRefs` / `SavedGmParams` state and the structure is
  not yet invoked from the production parse, which stops at `InterStop::ReachedSharedTail`
  before the shared tail), the bridge-frame remainder, `frame_header_copy()` for
  an INTER first header (the gate extends when the inter path completes), and the
  § 6.17.6.2 layer-dependency constraints (the §5.4.1 dependency maps are now exposed by
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

**Status:** partial — the `tile_group_obu()` § 5.19 STRUCTURE landed
(`tile-group-structure-completion`): on the intra-complete first-tile-group path
`parse_tile_group_structure` reads `tile_start_and_end_present_flag`, `tg_start`/`tg_end`,
`byte_alignment()`, and the `headerBytes`/payload boundary (the BRU arms are dead on intra
— `use_bru`/`bru_inactive` derive to the § 5.18.2 constant 0). The validator emits the
locally-decidable § 6.18 tg-range diagnostics (`tile-group/first-tg-start-not-zero`,
`tile-group/tg-end-before-tg-start`, `tile-group/tg-end-out-of-range`,
`tile-group/truncated-structure`, `tile-group/byte-alignment-zero-bit`) and `inspect`
surfaces the `tile_group_structure` view. `AV2-5.20-TILE-GROUP-PAYLOAD` (the payload body),
the INTER-path BRU arms, and the arithmetic-boundary targets are still untouched.

**Goal:** validate tile-group structure without prematurely promising a complete decoder.

Feature IDs:

- `AV2-5.19-TILE-GROUP`
- `AV2-5.20-TILE-GROUP-PAYLOAD`
- child rows for §5.20.1-§5.20.10 as needed.

Initial target:

- ~~validate tile group header/size fields~~ — landed for the intra-complete first tile
  group (the § 5.19 structure, tg range, and `headerBytes`/payload boundary);
- validate arithmetic coder entry/exit boundaries;
- validate `exit_symbol` / trailing-bit interactions;
- the INTER-path `tile_group_obu()` BRU arms and the cross-tile-group continuity § 6.18
  clauses remain (the latter needs prior-tile-group state);
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
| `brt/global-ordering-position` | error | §7.3.7 | `AV2-7.3-OBU-ORDERING` | A global BRT in an invalid temporal-unit position. Deferred: §7.3.7 does not list BRT among the global prefix OBUs, so a hard ordering error needs the §7.3.8 decoder-model / random-access state (tracked by a spec TODO under `AV2-7.3-OBU-ORDERING` in `splot-validate`). |
| `annex-a/frame-exceeds-ops-level` | error | §A.4/§6.10.4 | `AV2-A-LEVELS-TIERS` | A frame's size / tile geometry exceeds the Annex A.4 Table A.8/A.9 limits for an *operating-point-advertised* level: §6.10.4 requires the operating point's bitstream to satisfy A.4 with `seq_level_idx` set to the OPS-signaled `ops_level_idx`, so the static level-limit checks must also run against OPS levels, not only the activated `seq_level_idx`. Deferred from `annex-a-profile-level-skeleton`: blocked on operating-point-to-frame mapping (which frames belong to which operating point), which the validator does not model yet — `check_ops_level_tier_value_space` currently only checks the OPS-carried value space (reserved-level / high-tier), not per-frame conformance against the advertised level. |

Naming rules live in [`FEATURE-TRACKING.md`](./FEATURE-TRACKING.md) § 12
(Diagnostic-ID convention); never rename a landed ID without a migration note.

### Struck backlog entries

- `msdo/sub-xlayer-duplicate` (`AV2-5.6-MSDO`, §6.6): **struck** by
  `msdo-substream-constraint-checks`. Re-verification of the §6.6 MSDO semantics
  in the spec mirror
  ([`06-syntax-structures-semantics.md#s-6-6`](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-6),
  lines 1330–1398) found **no** `sub_xlayer_id` uniqueness requirement:
  `sub_xlayer_id[i]` only "specifies the value of obu_xlayer_id ... for the i-th
  independent sub-bitstream" (line 1359), with no constraint that the values
  differ. Spec honesty (AGENTS.md §6) forbids inventing the constraint, so the
  planned diagnostic is removed rather than implemented.

## Done criteria for the umbrella validator goal

The validator can be called “full syntax validator” only when:

- every §5 syntax row is `parse = done` and `tests = done`;
- every locally checkable §6 semantic row is `validate = done`;
- every stateful §7.3/HLS availability row has either `validate = done` or an explicitly documented blocked dependency;
- Annex A/E conformance rows are represented and implemented to the extent their required syntax exists;
- malformed data never panics under unit tests, proptests, and fuzzing;
- AVM/public-vector proof exists for representative valid and invalid streams.
