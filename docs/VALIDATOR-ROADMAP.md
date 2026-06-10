# Validator implementation roadmap

`status: active`  
`owner: validator`  
`scope: AV2 validator/parser/inspector, not encoder`

This is the single forward-looking validator planning document. The earlier
phase plans, design sketches, and the dated status snapshot
(`VALIDATOR-GAP-ANALYSIS.md`, `VALIDATOR-NEXT-PHASE.md`,
`VALIDATOR-SEQUENCE-HEADER-COVERAGE.md`, `VALIDATOR-HLS-AVAILABILITY-STATE.md`,
`VALIDATOR-IMPLEMENTATION-MATRIX-EXPANSION.md`, `VALIDATOR-NEXT-DIAGNOSTICS.md`,
`OPS-BRT-DIAGNOSTICS.md`, and `CURRENT-VALIDATOR-STATE.md`) were executed and
folded into this roadmap, the canonical matrix, and the diagnostics registry;
their content lives in git history. Canonical per-feature status remains
[`IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml) — see the
generated [`SPEC-COVERAGE.md`](./SPEC-COVERAGE.md) for the per-spec-section
view. Each phase below carries a coarse status line as of 2026-06-10.

## Guiding rule

Every validator feature must flow through the existing five-layer workflow:

```text
OpenSpec change -> docs/IMPLEMENTATION-MATRIX.toml -> code/tests/diagnostics -> xtask proof -> generated docs/FEATURE-STATUS.md
```

Do not mark a matrix stage `done` without proof. Do not add a bare `TODO(spec)`. Use `TODO(spec: FEATURE-ID): ...` and make sure the Feature ID exists in the matrix.

## Current focus and guardrails

Before any code edit, orient with:

```bash
git status --short
cargo xtask feature-status --format table
cargo xtask spec-coverage
cargo xtask ci
```

**Highest-leverage next work:** deepen sequence/HLS semantics and the
temporal-unit state machine before frame headers and tile groups. Sequence
state drives later parser branches; §6.2.2 activated limits and §7.3.8 HLS
availability are prerequisites for meaningful frame/tile validation; frame
header and tile group syntax depend on sequence-level dimensions, layer
limits, tool flags, dependency maps, and timing fields. The open gaps, in
dependency order:

| Gap | Feature IDs |
|---|---|
| Sequence semantics | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` |
| Activated sequence state | `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` |
| HLS availability | `AV2-7.3.8-HLS-AVAILABILITY` |
| Temporal-unit ordering completion | `AV2-7.3.7-TEMPORAL-UNIT-ORDER`, then §7.3.2–§7.3.6 children as parse dependencies allow |
| Deeper HLS semantics | the `validate = partial` HLS rows (LCR, atlas, OPS/BRT, metadata) and the deferred §6.10.7 dependency-map checks |
| Frame-header continuation | the Phase 8 child rows below |

**Do not start yet** as a primary task: a full tile-group payload parser,
entropy/range coding, a decoder, an encoder, a bitstream writer, or the AVM
differential harness. Prepare hooks and fixtures, but keep the core work
focused on the gaps above.

## Phase 0 — matrix and OpenSpec hygiene

**Status: done.** The roadmap is linked from `SPEC-MAPPING.md`,
`FEATURE-TRACKING.md`, and `README.md`; the OpenSpec change is archived
(`2026-06-07-validator-coverage-roadmap`); the matrix carries child rows for
the large features and the generated docs are regenerated and drift-gated.

**Goal (executed):** make missing validator work visible before code expands —
matrix child rows for large features, OpenSpec hygiene, generated status docs.

## Phase 1 — descriptor and payload-boundary foundation

**Status: done.** All five rows (`AV2-4.11.3-UVLC`, `AV2-4.11.5-LE`,
`AV2-4.11.8-NS`, `AV2-5.2.3-TRAILING-BITS`, `AV2-5.2.4-BYTE-ALIGNMENT`) have
`parse`/`tests`/`decode_check` done with proptest proof; `AV2-4.11.7-SU` and
`AV2-4.11.4-SVLC` landed beyond the original scope. Trailing-bits and
byte-alignment `validate` stays `partial` until every payload parser calls the
boundary helpers.

**Goal (executed):** make `splot-core` able to parse payload syntax without
panics or overreads — `BitReader` descriptors in `crates/splot-core/src/bitio.rs`,
trailing-bits/extension handling in `crates/splot-core/src/obu.rs`, typed EOF
errors, property tests over arbitrary byte slices.

## Phase 2 — `open_bitstream_unit(sz)` payload dispatch

**Status: done** for this phase's scope: dispatch, `PayloadStatus`, and the
`inspect --json` `payload_status` object landed with tests
(`AV2-5.2.1-OBU-DISPATCH`). The row's `parse = partial` is the declared honest
end-state until every payload variant exists.

**Goal (executed):** parse the OBU payload selected by `obu_type` instead of
treating every payload as opaque bytes. `ParsedObu` stays `#[non_exhaustive]`;
reserved OBU payloads stay `PayloadStatus::Opaque` (AV2 §5.3 defines no syntax
for them); unimplemented payloads return `PayloadStatus::Unimplemented` with
the owning Feature ID. Strict validation should fail on unparsed normative
payloads once the feature is marked as required.

## Phase 3 — sequence header parser, split by §5.4 child rows

**Status: partial.** All thirteen §5.4 child rows are `parse = done` /
`tests = done`; a valid sequence header parses in full. Remaining work is
deeper `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` validation — the umbrella and most
child `validate` stages are `partial`.

**Goal:** implement the first real OBU payload parser and unlock sequence-activated validation.

Umbrella:

- `AV2-5.4-SEQUENCE-HEADER`

Child rows (all landed; `validate` depth varies):

- `AV2-5.4.1-SEQUENCE-HEADER-GENERAL`
- `AV2-5.4.2-SEQUENCE-TILE-CONFIG`
- `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG`
- `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG`
- `AV2-5.4.5-SEQUENCE-INTRA-CONFIG`
- `AV2-5.4.6-SEQUENCE-INTER-CONFIG`
- `AV2-5.4.7-SEQUENCE-SCC-CONFIG`
- `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG`
- `AV2-5.4.9-SEGMENT-INFO`
- `AV2-5.4.10-SEQUENCE-FILTER-CONFIG`
- `AV2-5.4.11-USER-QM`
- `AV2-5.4.12-TIMING-INFO`
- `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO`
- `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`

The parser lives in `crates/splot-core/src/headers/sequence.rs` (no AV1 names;
every field maps directly to AV2 syntax or an AV2-derived variable). The local
`sequence-header/*` checks proposed by this phase are landed and listed in the
enforced registry tables of
[`VALIDATOR-DIAGNOSTICS.md`](./VALIDATOR-DIAGNOSTICS.md). Remaining work is
the deeper `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` validation noted in the status
line.

## Phase 4 — activated sequence state and remaining §6.2.2 checks

**Status: partial.** Activated `max_tlayer_id`/`max_mlayer_id` limits and the
core HLS availability checks landed with tests
(`AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` and
`AV2-7.3.8-HLS-AVAILABILITY` are `validate = partial`, `tests = done`); full
§7.3.8 availability modeling remains open.

**Goal:** the validator remembers activated sequence headers and uses them to check OBU layer IDs.

Feature IDs:

- `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`
- `AV2-7.3.8-HLS-AVAILABILITY`

State shape:

```rust
pub struct ValidatorContext {
    pub sequence_headers: SequenceHeaderStore,
    pub active_sequence_by_xlayer: BTreeMap<ExtendedLayerId, SequenceHeaderId>,
    pub temporal_unit: TemporalUnitState,
    pub diagnostics_mode: DiagnosticsMode,
}
```

First stateful checks:

- after activation, reject `obu_tlayer_id > max_tlayer_id`;
- after activation, reject `obu_mlayer_id > max_mlayer_id`;
- reject frame/tile OBUs before an available/activated sequence header once enough activation rules are known;
- preserve a partial-validation warning for payloads that cannot yet activate a sequence header.

Acceptance:

- Unit tests with one sequence header followed by a violating OBU.
- Unit tests with parseable prefix + later error retaining both stateful and parse diagnostics.
- No global mutable state.

## Phase 5 — OBU ordering and temporal-unit state machine

**Status: partial.** All eight child rows exist in the matrix;
temporal-delimiter, duplicate-delimiter, and ascending-xlayer ordering landed
with tests (`AV2-7.3.7-TEMPORAL-UNIT-ORDER` and
`AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT` are `validate = partial`,
`tests = done`). `AV2-7.3.2-CMVS-BOUNDARIES` and the coded-frame-unit rows
(`AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT`, `AV2-7.3.4-CODED-NONOUTPUT-FRAME-UNIT`,
`AV2-7.3.5-CODED-FRAME-UNIT`) are not started.

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

Initial checks:

- temporal unit starts with exactly one global temporal delimiter;
- global HLS OBUs precede coded extended layer units;
- coded extended layer units appear in ascending non-global `obu_xlayer_id` within a temporal unit;
- padding can appear anywhere, but outside coded extended layer units it must be global;
- global metadata prefix/suffix positions once metadata parsing exists.

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

**Status: partial.** Every Phase 6 row is `parse = done` / `tests = done` with
a dedicated parser module. Remaining work is deeper semantic validation
(`validate = partial` across the board), including the deferred MFH/OPS
§6.10.7 layer-dependency-map checks.

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

**Status: partial.** `AV2-5.15-CONTENT-INTERPRETATION` and `AV2-5.16-PADDING`
are fully done; quantizer matrix, film grain, and the §5.17 metadata family
parse with recorded tests but keep `validate = partial`.

**Goal:** parse and validate payload OBUs that are not the full frame/tile syntax yet.

Feature IDs:

- `AV2-5.13-QUANTIZATION-MATRIX`
- `AV2-5.14-FILM-GRAIN`
- `AV2-5.15-CONTENT-INTERPRETATION`
- `AV2-5.16-PADDING`
- `AV2-5.17-METADATA`
- metadata child rows for §5.17.1 through §5.17.13

Initial checks:

- padding payload bytes are zero where required by syntax/semantics;
- metadata type parsing and layer-specific/global rules;
- film-grain update flags and chroma idc ranges;
- quantization matrix non-zero entries and delta range once syntax exists;
- content-interpretation field bounds.

Acceptance:

- synthetic fixtures for each OBU type;
- `inspect` can show parsed fields in JSON;
- strict validation fails on unsupported payload syntax once corresponding matrix row is `partial`/`done`.

## Phase 8 — frame header child features

**Status: partial** — see the status notes below. Beyond the activation
skeleton, `AV2-5.18.3-FRAME-CONFIGURATION` and `AV2-5.18.4-FRAME-SIZE` parse
with tests, the intra tail through `AV2-5.18.6-QUANTIZATION` and
`AV2-5.18.7-SEGMENTATION-TILING` is `partial` (with `AV2-5.18.7.3-TILE-PARAMS`
done); `AV2-5.18.5-FILTERING` and the §5.18.8–§5.18.10 child rows are todo.

**Goal:** split the large frame header into implementable chunks.

> **Status (OpenSpec `frame-activation-hls-skeleton`):** a bounded, prefix-only
> frame-activation skeleton landed ahead of the full frame header. It parses just the
> `frame_header_info()` activation/reference fields (`cur_mfh_id`,
> `seq_header_id_in_frame_header`) and the `tile_group_obu()` prefix
> (`is_first_tile_group`, `frame_header_present_flag`), which unblocks the generic HLS
> reference checks (`hls/unavailable-sequence-header`,
> `hls/unavailable-multi-frame-header`) and CLK/OLK sequence-header activation. This
> deliberately precedes the full frame header (Phase 8) and tile payload (Phase 9):
> the activation skeleton gives exact validator state without committing to the full
> §5.18 / §5.20 syntax or the entropy coder. `AV2-5.18-FRAME-HEADER`,
> `AV2-5.18.1-FRAME-HEADER-GENERAL`, `AV2-5.18.2-FRAME-HEADER-INFO`, and
> `AV2-5.19-TILE-GROUP` are therefore `partial`, not `done`.

> **Status (OpenSpec `frame-tiling-quant-segmentation`):** the intra frame-header
> tail now parses past `disable_cdf_update` in exact § 5.18.2 order:
> `tile_info()` (§ 5.18.7.2), `quantization_params()` (§ 5.18.6.1),
> `segmentation_params()` (§ 5.18.7.1), `setup_qm_params()` (§ 5.18.6.2),
> `delta_q_params()` (§ 5.18.7.8), and the per-segment lossless /
> `allow_tcq` / `allow_parity_hiding` derivation, then stops with the explicit
> `stopped_before_deblocking_filter_params` status before § 5.18.5.2
> `deblocking_filter_params()`. New § 6.17.7.2 tile-bound diagnostics
> (`frame-header/tile-cols-out-of-range`, `frame-header/tile-rows-out-of-range`,
> `frame-header/context-update-tile-id-out-of-range`) and the § 6.17.6.2
> custom-QM plane-count check (`frame-header/qm-plane-count-mismatch`) are live,
> and `splot inspect` surfaces the parsed tile/quant/segmentation/QM/delta-q/
> lossless fields. Remaining for this phase: inter frame-header paths, the
> MFH-gated branches (`cur_mfh_id > 0` stops with `UnsupportedUntilFeature`),
> the § 5.18.7.4 non-uniform sequence-reuse branch (unrecorded start arrays),
> § 5.18.5 filtering onward, and the § 6.17.6.2 layer-dependency constraints
> (dependency maps not modeled). `AV2-5.18.6-QUANTIZATION` and
> `AV2-5.18.7-SEGMENTATION-TILING` are therefore `partial`, not `done`.

Umbrella:

- `AV2-5.18-FRAME-HEADER`

Child rows:

- `AV2-5.18.1-FRAME-HEADER-GENERAL`
- `AV2-5.18.2-FRAME-HEADER-INFO`
- `AV2-5.18.3-FRAME-CONFIGURATION`
- `AV2-5.18.4-FRAME-SIZE`
- `AV2-5.18.5-FILTERING`
- `AV2-5.18.6-QUANTIZATION`
- `AV2-5.18.7-SEGMENTATION-TILING`
- `AV2-5.18.8-TRANSFORM-CODING-MODES`
- `AV2-5.18.9-GLOBAL-MOTION`
- `AV2-5.18.10-FILM-GRAIN-STRUCTURES`
- matching §6.17 semantics rows as needed.

Rules:

- Frame header implementation must depend on parsed sequence header state.
- Avoid introducing a decoder unless a check truly requires it.
- Use AVM traces/differential testing as soon as fixture streams are available.

## Phase 9 — tile group and arithmetic payload boundary validation

**Status: partial** (barely started): only the `tile_group_obu()` prefix from
the Phase 8 activation skeleton landed; `AV2-5.20-TILE-GROUP-PAYLOAD` and the
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

**Status: todo.** Mapping-only; `cargo xtask conformance` is an explicit stub.
An active OpenSpec change (`avm-differential-harness`) plans the harness.

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
| `hls/multiple-active-sequence-headers` | error, or warning until CLK parsing exists | §7.3.8 | `AV2-7.3.8-HLS-AVAILABILITY` | More than one active sequence header observed for an extended layer without a modeled reset. |
| `sequence-state/monotonic-output-order-mismatch` | error | §6.4.1 | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` | Extended layers in a coded multistream video sequence disagree on `monotonic_output_order_flag`. |
| `sequence-state/distinct-mlayer-count-exceeds-seq-max` | error | §6.4.1 | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` | Count of distinct `obu_mlayer_id` values exceeds `SeqMaxMlayerCnt`. Distinct from the landed per-OBU `sequence-state/mlayer-exceeds-max` and the parse-time `sequence-header/seq-max-mlayer-count-out-of-range`. |
| `obu-order/global-hls-after-metadata-suffix` | error | §7.3.7 | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | Global HLS appears after suffix metadata. Needs global suffix-metadata classification, which is pending frame/tile parsing. |
| `obu-order/non-global-hls-before-coded-layer` | error | §7.3.7 | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | Non-global HLS appears in an invalid temporal-unit region. |
| `msdo/sub-xlayer-duplicate` | error | §6.6 | `AV2-5.6-MSDO` | Duplicate `sub_xlayer_id` where uniqueness is required. Add only after confirming the exact §6.6 wording in the spec mirror (spec honesty). |
| `ops/mlayer-dependency-missing` | error | §6.10.7 | `AV2-5.10-OPERATING-POINT-SET` | OPS embedded-layer info disagrees with the activated sequence header's `MLayerDependencyMap`. Deferred: the sequence-header model does not expose the dependency maps (see the intentional non-check in [`VALIDATOR-DIAGNOSTICS.md`](./VALIDATOR-DIAGNOSTICS.md)). |
| `ops/tlayer-dependency-missing` | error | §6.10.7 | `AV2-5.10-OPERATING-POINT-SET` | As above, for the activated `TLayerDependencyMap`. |
| `brt/global-ordering-position` | error | §7.3.7 | `AV2-7.3-OBU-ORDERING` | A global BRT in an invalid temporal-unit position. Deferred: §7.3.7 does not list BRT among the global prefix OBUs, so a hard ordering error needs the §7.3.8 decoder-model / random-access state (tracked by a spec TODO under `AV2-7.3-OBU-ORDERING` in `splot-validate`). |

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
