# Decoder Roadmap

`status: planned`
`owner: decoder/reconstruction`
`scope: encoder-grade decode and reconstruction support, not playback`

## Scope

`splot` remains validator-first. Decoder work exists only where it helps future
encoder roundtrips and reconstruction correctness:

- parse accepted AV2 streams through the existing Annex B/IVF front door;
- return structured unsupported-feature diagnostics for streams outside the
  supported tier;
- define decoded frames, planes, hashes, limits, and reference-frame state that
  a future encoder can reuse for closed-loop encoding;
- eventually reconstruct a small, documented all-intra tier and prove it with
  self-contained fixtures.

It is not a production media player, not an optimized decoder, and not an
AVM/dav2d wrapper.

Current state: `splot decode` has a narrow byte-consuming runtime success path
for the committed `minimal-intra-8bit420-hash-v1` IVF tier. It reads input
bytes, constructs `DecodeContext`, calls `DecodeContext::plan_bytes`, emits
`splot.decode.hash_report` JSON for `--output-format hash`, and atomically
publishes raw sample bytes for `--output-format raw -o` or Y4M for `-o` /
`--output-format y4m -o` on that minimal fixture.
Diagnostics remain structured data owned by `splot-decode`:
`decode/malformed-source` for malformed source/container bytes,
`decode/resource-limit` for byte-planner or runtime limit failures,
`decode/unsupported-feature` for planner unsupported structures or out-of-tier
runtime requests, and `decode/output-error` for raw/Y4M serialization/publication
failures. It does not perform broad tile decode, broad pixel reconstruction,
broad raw/Y4M output, film grain, reference refresh, or external decoder
invocation.
Decode resource limits now have a source-backed `splot-decode` policy API for
configured thresholds and pure checks, and the byte-stream planner applies the
input-byte, OBU-count, IVF frame-record, and selected-frame-candidate limits
during traversal.
The workspace now includes scaffolded `splot-recon` and `splot-decode` crates
for future reconstruction primitives and the future decode driver. `splot-decode`
also exposes `DecodeRuntimeConfig` and `DecodeContext`; each context owns one
`splot_parallel::WorkerPool` configured by the `--threads auto|N` runtime policy,
and now provides library-only stream planners over raw bytes and already parsed
`splot-core` stream structures. `DecodeContext::plan_bytes` walks raw Annex B or
IVF/DKIF bytes with bounded traversal before returning the same
`DecodeStreamPlan` as `DecodeContext::plan_stream`; both paths preserve raw
Annex B / IVF OBU order and offset metadata, select only the base minimal-tier
layer, treat `OBU_CLOSED_LOOP_KEY` as the only frame candidate, and reject
malformed sources or unsupported structures transactionally. These APIs do not
decode tile payloads, reconstruct pixels, produce hashes, write Y4M, or provide
runtime decode output.
`splot-recon` now exposes immutable decoded output frame and plane model types
with constructor invariants plus a bounded immutable reference-slot container,
canonical decoded-frame hash input serialization, source-backed
`splot-dfh-sha256-v1` digest computation, and a source-backed Y4M writer for
caller-supplied decoded frames. It also exposes scheduler-free scalar
prediction primitives for square and rectangular § 7.13.2.10 DC intra
prediction over caller-provided left/above edge samples, § 7.13.2.11
subsampled DC prediction over caller-provided prepared edges, § 7.13.2.12 IBP
DC prediction over existing DC prediction buffers and prepared edges, plus
§ 7.13.2.2 basic/PAETH prediction over prepared left/above/top-left edge
samples, and § 7.13.2.13 smooth prediction over prepared left/above sentinel
edge samples; rectangular both-edge DC and subsampled DC nonzero-count
prediction use the § 7.13.3.22 approximate divisor path. The H/V cardinal
§ 7.13.2.8 directional subset is supported for caller-prepared edges (`V_PRED`
pAngle 90 and `H_PRED` pAngle 180), and the one-sided non-IDIF chroma
directional-angle subset is supported for caller-prepared pAngles 45, 67, and
203. The middle non-IDIF directional-angle subset is supported for
caller-prepared pAngles 113, 135, and 157 over explicit logical above/left edge
ranges. The current-frame workspace can hand off fully available in-storage
one-sided and middle directional-angle prepared edges to those primitives.
Luma/MRL/IDIF/full-dispatch directional angles, full edge preparation,
data-driven prediction, general directional-angle IBP, full CfL, full
`predict_intra()` dispatch, the full dequantization process, the § 7.15.4
DPCM-direction selection and combined transform-parameter resolve helper, runtime
decode output, output scheduling, and AV2 reference refresh semantics remain
unimplemented. The
full AV2 § 7.14.2 dequantization quantizer functions are available as
scheduler-free `splot-recon` primitives: the quantizer-value lookup core
(`Ac_Qlookup`, `qlookup`, `MaxQ`, and `get_q`;
`RECON-DEQUANT-QUANTIZER-LOOKUP`) and `get_qindex` quantizer-index
resolution plus the per-plane `get_dc_quant` and `get_ac_quant`
composition (`RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION`). All three § 7.15.2
1D inverse transforms are available as scheduler-free primitives: the
§ 7.15.2.1 kernel-based transform (DCT/ADST/FDST/DDTX/FDDT over the shared
`splot-tables` § 9.6 kernels; `RECON-INVERSE-TRANSFORM-1D`), and the § 7.15.2.2
Walsh-Hadamard and § 7.15.2.3 identity transforms
(`RECON-INVERSE-TRANSFORM-MATRIX-FREE`). The § 7.14.3 residual-addition step
(`Clip1(prediction + residual)`; `RECON-RESIDUAL-ADDITION`) is also available as
a scheduler-free primitive. The § 7.15.4.1 2D matrix transform core
(`inverse_transform_2d`; `RECON-INVERSE-TRANSFORM-2D`) is available too: the
row-then-column matrix passes over a caller-supplied dequantized block, with the
√2 rescale and per-pass `get_identity_scale` derived from the original
(unadjusted) `txSz` log2 dimensions and the adjusted operating size capped at
32x32. The § 7.15.4 outer orchestration
(`inverse_transform_2d_outer`; `RECON-INVERSE-TRANSFORM-2D-OUTER`) wraps that
core with the `Lossless && IDTX` bit-shift shortcut, the DPCM cumulative sum, and
adjusted-size sample duplication, all derived from the original log2 dimensions
(no conversion tables) over caller-resolved transform selections. The § 7.14.4
dequantization process (`dequant_coefficient` / `dequantize_block`;
`RECON-DEQUANT-PROCESS`) turns coded `Quant` coefficients into the `Dequant`
array the inverse transform consumes — the per-coefficient steps 3-8 and the
non-quantization-matrix transform-block helper, over caller-resolved quantizers.
The § 7.14.4 step-2 quantization-matrix weighting
(`quantization_matrix_weight` / `qm_weighted_quantizer`; `RECON-DEQUANT-QM-WEIGHT`)
is also available, consuming the built-in § 9.4 `Quantizer_Matrix` (relocated to
the dependency-free `splot-tables` crate). The § 7.15.4 `Transform_Shift` row and
column down-shift lookup (`transform_shift`; `RECON-TRANSFORM-SHIFT-LOOKUP`), the
first of the caller-resolved inverse-transform parameter derivations, is available
too: it returns `(rowShift, colShift)` keyed on the original `(log2W, log2H)` shape
from the verbatim § 7.15.4 constant table. The § 7.15.4 `get_transform_1d_type`
row and column transform-type derivation (`get_transform_1d_type`;
`RECON-GET-TRANSFORM-1D-TYPE`) is also available: it returns the
`Transform_1d_Type[PlaneTxType][dir]` selection (as the `InverseTransform2dDim`
the 2D transform consumes), with the `useDdt` `DDTX`/`FDDT` substitution. The
§ 5.20.7.30 `get_scan` coefficient scan order (`coefficient_scan_order`;
`RECON-COEFFICIENT-SCAN-ORDER`) is available too: it writes the coefficient scan
order for a `w * h` block (the anti-diagonal 2D scan and the row/column raster
scans), a prerequisite for the coefficient decode loop and § 7.14.4 coefficient
placement. Its § 8.3.2 `get_tx_class` companion (`tx_class`;
`RECON-GET-TX-CLASS`) is available too: a `const fn` mapping a `PlaneTxType` to the
`TransformClass` that selects the scan (the vertical-only and horizontal-only
transforms to `Vertical`/`Horizontal`, everything else to `TwoD`). `splot-decode`
also has crate-private tile coefficient state buffers
(`DECODE-TILE-COEFF-STATE-BUFFERS`): transform-block-local § 5.20.7.27
`Level[]` / `QuantSign[]` arrays and three-plane above/left level/DC context
lines, with checked end-of-`coeffs()` context updates and block-context resets.
Those buffers now feed the minimal trace's luma and V-plane `all_zero` context
handoff (`DECODE-COEFF-ALL-ZERO-CONTEXT-STATE`), replacing literal first-block
level/DC reductions with state-backed reads while keeping output unchanged. The
minimal trace also applies the § 5.20.7.27 all-zero coefficient block state
effects (`DECODE-COEFF-ALL-ZERO-BLOCK-STATE`): zero `Level[]`, `QuantSign[]`,
and `Quant[]` state, `eob == 0`, and zero above/left level/DC context writes for
the traced luma and V all-zero branches. They are still not read by a real
coefficient symbol loop. The nonzero branch now has a checked § 5.20.7.27 EOB
value helper (`DECODE-COEFF-EOB-VALUE-STATE`) for caller-decoded `eobPt`,
`eob_extra`, and packed `eob_extra_bit` refinements, plus a crate-private EOB
symbol reader (`DECODE-COEFF-EOB-SYMBOL-READ`) that consumes caller-selected
`eob_pt_*` CDF rows, size-specific `eob_pt_*_extra` literal bits, `eob_extra`,
and packed `eob_extra_bit` refinements before calling that value helper. That
reader now has a caller-fact derivation helper
(`DECODE-COEFF-EOB-SIZE-CONTEXT`) that maps caller-resolved
`Tx_Width_Log2[txSz]` / `Tx_Height_Log2[txSz]` values to the `eob_pt_*` family
and derives `eobCtx = (plane > 0) ? 2 : is_inter`, plus a derived EOB reader
(`DECODE-COEFF-EOB-DERIVED-SYMBOL-READ`) that composes those caller facts with
the symbol-read sequence. A coefficient EOB branch handoff
(`DECODE-COEFF-EOB-BRANCH-HANDOFF`) now dispatches caller-selected all-zero
branches to the all-zero state helper and nonzero branches to that derived EOB
reader, and the nonzero branch now initializes a zeroed local coefficient block
state shell (`DECODE-COEFF-NONZERO-BLOCK-STATE`) before reading EOB syntax. The
ordinary non-FSC nonzero path also has a checked decode-local scan-walk boundary
(`DECODE-COEFF-SCAN-WALK`) over caller-supplied `scan[c]` positions: it validates
EOB length and scan-position bounds and returns reverse-order `c`/`pos`/row/col
facts without importing `splot-recon`, consuming symbols, mutating CDFs, or
writing coefficients. The FSC/IDTX path now has the corresponding forward scan
window (`DECODE-COEFF-FSC-SCAN-WALK`): it validates caller-resolved `segEob`,
derives `bob = segEob - eob`, and returns checked `bob..segEob` entries without
reading symbols or writing coefficients. The ordinary non-IDTX coefficient base/base-EOB/base-range
CDF row families are now loaded and selectable in the tile CDF subset
(`DECODE-COEFF-BASE-CDF-ROWS`), including tile copy/save/average and frame-end
count scaling. A crate-private ordinary non-FSC coefficient base symbol-read
boundary (`DECODE-COEFF-BASE-SYMBOL-READ`) now consumes caller-resolved
`coeff_base_eob`, `coeff_base`, and conditional `coeff_br` rows over checked
scan-walk entries and returns decoded level-building symbols. Those decoded
ordinary non-FSC levels can now be applied to local transform-block `Level[]`
state (`DECODE-COEFF-LEVEL-STATE-WRITE`) after validating the read records
against the checked scan walk, while keeping `QuantSign[]` and `Quant[]`
untouched. A crate-private sign-read boundary
(`DECODE-COEFF-SIGN-SYMBOL-READ`) now consumes caller-resolved `dc_sign`,
`dc_sign_horz_vert`, and raw `sign_bit` sources over those local `Level[]`
entries and returns sign summaries, but still does not write `QuantSign[]` or
`Quant[]`. A crate-private sign-source derivation boundary
(`DECODE-COEFF-SIGN-SOURCE-DERIVE`) now derives those sign read inputs from
post-level `Level[]`, hidden parity, plane, transform class, and above/left DC
context lines, selecting luma `dc_sign`, luma horizontal/vertical
`dc_sign_horz_vert`, raw `sign_bit`, or a skipped source without consuming
symbols. A crate-private `maxLevel` derivation boundary
(`DECODE-COEFF-MAX-LEVEL-DERIVE`) now applies the § 5.20.7.27
`get_lf_limits(row, col, txClass, plane)` branches plus the hidden `c == 0`
override over checked scan entries, returning records convertible to the quant
pass inputs. `DECODE-COEFF-TX-CLASS-DERIVE` now removes one staged caller fact
by deriving ordinary coefficient `txClass` from caller-resolved `PlaneTxType`
locally in `splot-decode`, covering the § 8.3.2 vertical-only,
horizontal-only, and fallback 2D branches before delegating to the existing
max-level path. It still does not implement § 5.20.7.29 `compute_tx_type`,
derive scan order, or wire runtime `coeffs()`. A crate-private § 5.20.7.28
`read_quant` parser
(`DECODE-COEFF-READ-QUANT-SYNTAX`) now consumes caller-resolved
level, max-level, hidden, and TCQ facts plus the reached `q_length_bit`,
`golomb_length_bit`, and `coeff_rem` literal syntax, returning quant records.
Those sign summaries plus `read_quant` outputs can feed a crate-private
ordinary non-FSC quantized-state boundary
(`DECODE-COEFF-QUANT-STATE-WRITE`) that applies hidden parity, clamped
`culLevel`, `dcCategory`, optional TCQ, sign, and signed `Quant[pos]` writes
while preserving `QuantSign[]`. A loaded-but-unwired composition boundary
(`DECODE-COEFF-QUANT-PASS-COMPOSE`) now preflights caller facts, runs
`read_quant`, and feeds those decoded records into the quantized-state writer
for the ordinary non-FSC second pass. A crate-private max-level handoff
(`DECODE-COEFF-QUANT-PASS-MAXLEVEL-HANDOFF`) now derives those quant-pass
`maxLevel` inputs from checked scan entries, caller-resolved plane and transform
class, and the quant-pass hidden flag before delegating to the composer. A
loaded-but-unwired ordinary non-FSC pass composition boundary
(`DECODE-COEFF-ORDINARY-PASS-COMPOSE`) now composes nonzero block start, checked
scan walk, base-symbol reads, local `Level[]` writes, and the per-coefficient
interleaved sign, `maxLevel`, `read_quant`, and signed `Quant[]` write steps
over caller-resolved scan, selector, plane, transform-class, hidden, sumAbs1,
TCQ, and lossless facts while resetting `hrLevelAvg` to 0 at block entry. These
boundaries are not wired into runtime `coeffs()` yet. A crate-private
state-derived first pass (`DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`) now derives
`coeff_base_eob`, later `coeff_base`, and conditional `coeff_br` selectors from
the evolving local `Level[]`, updates first-pass `tcqState`, `sumAbs1`, `numNz`,
and `isHidden`, and writes each decoded `Level[row][col]` before deriving the
next selector. `DECODE-COEFF-BASE-PH-CDF-ROW`
now loads/selects the parity-hidden-only `TileCoeffBasePhCdf` row and proves an
eob>=5 hidden-parity first pass consumes it for the final DC coefficient.
`DECODE-COEFF-IDTX-CDF-ROWS` now loads/selects the FSC/IDTX
`TileCoeffBaseBobCdf`, `TileCoeffBaseIdtxCdf`, `TileCoeffBrIdtxCdf`, and
`TileIdtxSignCdf` row families in the tile CDF subset with tile copy/save/average
and frame-end scaling coverage, but still leaves the runtime `useFsc` symbol pass
unwired.
`DECODE-COEFF-FSC-LEVEL-PASS` now consumes the FSC/IDTX level rows in a
loaded-but-unwired first pass: it validates the checked `bob..segEob` walk
against local block geometry, derives `coeff_base_bob`, later
`coeff_base_idtx`, and conditional `coeff_br_idtx` selectors from current
`Level[]`, clamps the FSC transform-size context axis, reads the selected rows,
and writes local `Level[]` in forward scan order. It still does not read
`idtx_sign`, run `read_quant`, write `QuantSign[]` or `Quant[]`, commit tile
context, or wire runtime `coeffs()`.
`DECODE-COEFF-FSC-SIGN-PASS` now consumes the FSC/IDTX sign rows in a
loaded-but-unwired second pass: it walks `0..segEob`, derives `idtx_sign`
selectors from evolving local `QuantSign[]` and `Level[]`, reads signs only for
nonzero levels, writes local `QuantSign[]` so later sign contexts observe prior
signs, and leaves `Quant[]` untouched.
`DECODE-COEFF-FSC-QUANT-PASS` now composes the FSC/IDTX second loop in a
loaded-but-unwired pass: starting from the level pass, it interleaves
`idtx_sign`, immediate `QuantSign[]` writes, § 5.20.7.28 `read_quant` with FSC
constants (`isHidden = 0`, `maxLevel = NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1`,
`allowTcq = 0`), signed local `Quant[]` writes, and final `culLevel` /
`dcCategory` derivation. `DECODE-COEFF-FSC-CONTEXT-COMMIT` now wraps that FSC
pass with the § 5.20.7.27 end-of-`coeffs()` tile context update, committing the
final `culLevel` and `dcCategory` through `TileCoeffContextState` with
caller-resolved plane and 4x4 geometry. It still does not wire runtime
`coeffs()`. `DECODE-COEFF-FSC-BRANCH-HANDOFF` now composes the loaded FSC
nonzero branch target: it rejects all-zero and non-luma routing, runs the
nonzero EOB start, derives the checked FSC `bob..segEob` scan walk from
caller-resolved `segEob` and scan order, then runs the FSC level and
quant/context-commit stages. `DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF` now
derives `segEob` from the caller-resolved scan extent before delegating to that
branch, matching the shared § 5.20.7.27 / § 5.20.7.30 capped transform extent.
`DECODE-COEFF-FSC-BRANCH-SCAN-ORDER` now derives
`scan = get_scan(txSz, txClass)` from generated transform-size dimensions and
caller-resolved `PlaneTxType`, sharing the same § 5.20.7.30 scan-order helper
with the ordinary branch before delegating to the scan-extent wrapper. The
loaded-but-unwired `DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF` helper now derives
FSC branch EOB context, adjusted level/sign dimensions, `txSzCtx`, scan order,
and context-commit geometry from caller-resolved block geometry plus `txSz`,
before delegating to the scan-order wrapper. Runtime `useFsc`, full
transform/`compute_tx_type`, `PlaneTxType`, `is_inter`, and `coeff_cdf_q_ctx`
derivation remain unwired.
Separately, `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS` now composes the ordinary
state-derived first pass into the ordinary coefficient pass, carrying
first-pass `isHidden`, `sumAbs1`, and `useTcq` into the interleaved
sign/`read_quant`/signed `Quant[]` stage and deriving second-pass
plane/transform-class facts from the same config.
`DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS` now removes caller-supplied sign
inputs from that derived-base path: it derives sign sources from the first-pass
`Level[]`, `isHidden`, `sumAbs1`, plane, transform class, and DC context-line
facts before the interleaved sign/quant stage.
`DECODE-COEFF-NONZERO-CONTEXT-COMMIT` now wraps that derived-base/derived-sign
ordinary pass with the § 5.20.7.27 end-of-`coeffs()` tile context update,
committing the pass result's final `culLevel` and `dcCategory` to the
above/left level and DC context lines through `TileCoeffContextState`.
`DECODE-COEFF-STATE-CONTEXT-HANDOFF` now adds the next state-backed handoff:
the ordinary nonzero pass reads `AboveDcContext[plane]` and
`LeftDcContext[plane]` from that same `TileCoeffContextState` before sign-source
derivation, then commits the final context lines after the pass succeeds.
`DECODE-COEFF-ORDINARY-BRANCH-HANDOFF` now wraps the caller-decoded `all_zero`
choice with one ordinary branch boundary: the minimal trace uses its all-zero
arm for the existing luma and V applications without changing output, while the
nonzero arm composes EOB start with the state-backed ordinary pass for staged
tests. `DECODE-COEFF-ORDINARY-BRANCH-TX-CLASS-HANDOFF` now lets that branch
boundary accept caller-resolved `PlaneTxType`, derive `txClass` with the
decode-local § 8.3.2 helper, and then delegate to the existing branch path while
leaving all-zero behavior unchanged. `DECODE-COEFF-ORDINARY-BRANCH-PLANE-TYPE-HANDOFF`
adds the next branch-level wrapper: it derives AV2 § 5.20.7.27 `ptype = plane > 0`
from the caller-resolved plane before delegating to the `PlaneTxType` handoff, so
the nonzero path no longer accepts a contradictory caller-supplied `plane_type`
at that wrapper. `DECODE-COEFF-ORDINARY-BRANCH-GEOMETRY-HANDOFF` now derives the
state-context `x4`, `y4`, `w4`, and `h4` facts from the same nonzero block-start
geometry carried by the branch input before delegating to the `plane_type`
handoff. `DECODE-COEFF-ORDINARY-BRANCH-COEFFS-GEOMETRY-HANDOFF` now derives that
block geometry from AV2 § 5.20.7.27 `coeffs()` geometry facts (`startX`,
`startY`, caller-resolved `Tx_Width[txSz]`, and `Tx_Height[txSz]`) before
delegating to the block-geometry handoff. `DECODE-TX-SIZE-SYMBOLIC-TABLES` now
extends generated § 9.2 conversion-table support with the `TX_*` enum-valued
`Adjusted_Tx_Size`, `Tx_Size_Sqr`, and `Tx_Size_Sqr_Up` arrays, so future decode
wrappers can consume the generated `splot-core` copies rather than local table
transcriptions. `DECODE-MODE-TO-TXFM-SYMBOLIC-TABLE` now similarly generates
the TxType-valued § 9.2 `Mode_To_Txfm` conversion table for future
`compute_tx_type()` work, without wiring runtime transform-type computation.
`DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-DIMENSIONS`
now derives
`Tx_Width[txSz]`, `Tx_Height[txSz]`, `Tx_Width_Log2[txSz]`, and
`Tx_Height_Log2[txSz]` from the generated § 9.2 conversion tables before
delegating to the `coeffs()` geometry handoff.
`DECODE-COEFF-ORDINARY-BRANCH-ADJUSTED-TX-SIZE` now consumes generated
`Adjusted_Tx_Size[txSz]` so § 8.3.2 ordinary base contexts receive adjusted
width, height, and width-log2 dimensions while raw dimensions still drive
§ 5.20.7.27 block geometry and EOB-size context.
`DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-CONTEXT` now derives `txSzCtx` from
generated `Tx_Size_Sqr[txSz]` and `Tx_Size_Sqr_Up[txSz]` before the ordinary
base-context pass. `DECODE-COEFF-ORDINARY-BRANCH-SCAN-ORDER` now derives
`scan = get_scan(txSz, txClass)` from raw transform dimensions and decode-local
§ 5.20.7.30 scan-order logic after deriving `txClass` from caller-resolved
`PlaneTxType`.
`DECODE-COEFF-ORDINARY-BRANCH-MODE-TO-TXFM-HANDOFF` now derives `PlaneTxType`
for the non-lossless intra chroma non-directional `Mode_To_Txfm` subset from
caller-resolved `enable_chroma_dctonly`, generated § 9.2 `Mode_To_Txfm`, and
the inline § 5.20.7.29 `Tx_Type_In_Set_Intra` membership table before
delegating to the `PlaneTxType` handoff.
`DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF` now extends that
intra chroma handoff with the directional `UVMode` branch: it carries
caller-resolved `AngleDeltaUV`, derives `pAngle` from generated § 9.2
`Mode_To_Angle` plus § 3 `ANGLE_STEP`, applies the inline § 5.20.7.29
`wide_angle_mapping` thresholds over generated transform dimensions, and then
maps the resulting mode through generated `Mode_To_Txfm` before the same
transform-set membership check.
`DECODE-COEFF-ORDINARY-BRANCH-LUMA-TXTYPES-HANDOFF` now extends the same
transform-type handoff with the non-lossless luma `TxTypes[blockY][blockX]`
branch: it carries caller-resolved luma `TxTypes`, validates the value against
the AV2 `TX_TYPES` domain, and returns it before chroma-only
`enable_chroma_dctonly` and `UVMode` logic.
`DECODE-COEFF-ORDINARY-BRANCH-CHROMA-INTER-TXTYPES-HANDOFF` now adds the
non-lossless chroma-inter `TxTypes[y4][x4]` branch: it carries caller-resolved
chroma-inter `TxTypes`, validates the value against the AV2 `TX_TYPES` domain,
checks the inline § 5.20.7.29 `Tx_Type_In_Set_Inter` membership table, and
falls back to `DCT_DCT` when the transform type is outside the inter set.
`DECODE-COEFF-ORDINARY-BRANCH-TX-SET-HANDOFF` now derives AV2 § 5.20.8.3
`txSet` from `txSz`, plane, caller-resolved `is_inter`, caller-resolved
`reduced_tx_set`, caller-resolved `enable_chroma_dctonly`, and generated § 9.2
transform-size conversion tables before delegating to the `Mode_To_Txfm`
handoff. `DECODE-COEFF-ORDINARY-BRANCH-LOSSLESS-HANDOFF` now adds the staged
§ 5.20.7.29 `Lossless` branch that selects `DCT_DCT` before `get_tx_set`, while
delegating non-lossless inputs back to the `txSet` handoff. The coefficient
branch still does not implement the full § 5.20.7.29 `compute_tx_type` process
or wire runtime `coeffs()`: runtime FSC/IDTX routing, lossless runtime
handling, and frame-state derivation remain staged gaps.
Runtime integration of nonzero coefficient blocks, tile context fact derivation
for nonzero blocks, dequantization, and
reconstruction remain unsupported. The
§ 7.14.4
`useQm` / `UserQm` gating and `shift` derivation, the rest
of the § 7.14.3 reconstruct process, the § 7.15.3 secondary transform, the
§ 7.15.4 DPCM-direction selection and combined transform-parameter resolve helper,
the remaining § 5.20.7.29 `compute_tx_type` transform-type branches that produce
`PlaneTxType`, and the coefficient
entropy decode that produces nonzero `Quant` remain unimplemented.
`splot-recon` remains scheduler-free:
future decoder code must partition and schedule parallel work from
`splot-decode`, then call deterministic reconstruction primitives.
`splot-core` exposes a complete, spec-exact bounded AV2 § 8.2 `SymbolDecoder`
primitive for caller-provided tile payload slices: initialization, pseudo-raw
bool/literal reads, caller-supplied-CDF symbol reads with optional CDF updates,
and `exit_symbol()` trailing/padding conformance validation, proven across every
arity, the adaptation-rate extremes, deep-negative `SymbolMaxBits` padding, a
random-CDF property test, and the `symbol_decoder_bytes` fuzz target. This does
not make runtime tile decode supported; broad § 8.3 CDF selection, full tile CDF
banks, `decode_tile()`, broad reconstruction, broad hash output, and broad Y4M
output remain future rows beyond the committed minimal fixture tier.
`splot-decode` now also has crate-private tile-payload planning for the minimal
one-tile closed-loop-key tier. The boundary consumes § 5.20.1
`TileGroupFraming`, checks tile payload/count limits, derives one deterministic
tile work unit with exact source/layer/tile/MI-range/byte-span provenance,
initializes § 8.2 symbol state for the bounded tile slice, attaches a
crate-private partition CDF subset (`TileDoSplitCdf`, `TileDoSquareSplitCdf`,
`TileRectTypeCdf`, `TileDoExtPartitionCdf`, and
`TileDoUneven4wayPartitionCdf`) copied from generated § 9.3 defaults with typed
§ 8.3 row selection, bounded left/above-derived § 8.3.2 contexts for `do_split`,
`do_square_split`, `rect_type`, `do_ext_partition`, and
`do_uneven_4way_partition`, § 8.2 copy/average policy metadata, and a
crate-private single-symbol boundary for the five corresponding § 5.20.3.2
partition-entry `S()` reads. It also has a crate-private § 5.20.3.2 partition
decision boundary that returns one typed partition outcome from caller-provided
allowed/implied facts, BRU-active state, rect-type facts, the existing `S()`
read helper, and the isolated `uneven_4way_partition_type L(1)` read. It also
has a crate-private § 9.2 partition-size table boundary backed by generated
`splot-core` `Partition_Subsize` and `H_Partition_Midsize` arrays; the wrapper
returns valid block sizes or an explicit `BLOCK_INVALID` result for future
partition traversal. It also has a crate-private § 5.20.3.2 allowed-partition
boundary that derives `partition_implied`, `partition_implied_at_boundary`,
`rect_type_implied_by_bsize`, `is_partition_allowed`, and
`init_allowed_partitions` from explicit bounded tile facts while preserving
`BLOCK_INVALID` residual-size results. It also has a crate-private § 5.20.3.1
partition traversal frontier that composes those boundaries to advance from a
tile work unit to the first in-frame `decode_block()` frontier, with
transactional tile-CDF mutation, a symbol-decoder checkpoint, and pending
sibling partition calls preserved for a future block decoder. It also has a
crate-private minimal flat intra block-symbol trace frontier for the committed
runtime fixture: after the partition frontier, tile-payload code consumes the
traced `y_mode_set`, `y_mode_index`, luma/U all-zero transform skip, `uv_mode`,
and V all-zero transform skip rows from generated § 9.3 defaults, validates
`exit_symbol()`, and hands only the summary back to the minimal runtime. The
`DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` is limited to that
currently supported CDF subset: the partition-entry rows plus those minimal
flat-intra block-symbol rows. It makes Tile-to-Saved mutation occur only
after successful tile completion and `exit_symbol()`, preserve Saved and Frame
CDF state on symbol mismatch, CDF/symbol parse failure, resource-limit failure,
or exit-padding failure, and adds supported-subset `frame_end_update_cdf()`
count scaling. That work does not claim full § 8.3 selector coverage, all § 9.3
CDF banks, multi-tile or multi-tile-group CDF averaging, reference-frame CDF
persistence, `load_cdfs`, `save_cdfs`, or `blend_cdfs`. The
generic tile payload boundary still stops at structured
`decode/unsupported-feature` metadata for the unimplemented broad
`decode_tile()` block syntax. A crate-private source-backed derivation bridge
now validates a selected `DecodePlannedObu` against a borrowed `splot-core`
`ObuEnvelope`, slices only the complete § 5.19-derived § 5.20 payload region,
uses parser-derived tile grid, quantizer, CDF, and `disable_cdf_update` facts,
and runs the resulting boundary inside the context-owned
`splot_parallel::WorkerPool`, preserving the PR #101 concurrency model without
exposing public tile-payload APIs. The narrow minimal hash/raw/Y4M runtime is wired
through the partition and block-symbol trace frontiers, but it still does not
support multiple tiles or tile groups, bridge/BRU paths,
full `read_partition()`/`decode_tile()` traversal past the first
`decode_block()` frontier, broad MI-size mutation beyond the crate-private
`DECODE-TILE-MI-SIZE-STATE-BOUNDARY`, `decode_block()` syntax,
broad Saved CDF mutation outside the supported subset, broad reconstruction,
broad hashes, broad runtime raw/Y4M, reference refresh, or external decoders.

Canonical decoder status lives in
[`DECODER-SUPPORT-MATRIX.toml`](./DECODER-SUPPORT-MATRIX.toml), rendered to
[`DECODER-SUPPORT-STATUS.md`](./DECODER-SUPPORT-STATUS.md). The global feature
ledger remains [`IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml).
The future full-decoder conformance claim is defined in
[`DECODER-FULL-CONFORMANCE.md`](./DECODER-FULL-CONFORMANCE.md), and the
decode-relevant AV2 section-family ownership map is generated in
[`DECODER-SPEC-COVERAGE.md`](./DECODER-SPEC-COVERAGE.md). These documents expose
current unsupported and partial runtime decoder gaps; they do not make the
narrow minimal hash/raw/Y4M paths a full supported decoder.
The output-equivalence contract tracked by
`DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` defines the future runtime output
identity target: `raw_intermediate_output` and `post_film_grain_output`
variants, `splot-dfh-sha256-v1` raw-intermediate hash reporting, visible sample
bytes, show-existing and flush output order, raw/Y4M output policy, metadata
hash separation, and atomic file publication. The
`minimal-intra-8bit420-hash-v1` rows now support the first raw-intermediate hash
success artifact, first atomically published raw sample file, and first
atomically published Y4M file for the committed minimal IVF fixture; film grain,
broad output ordering, broad raw/Y4M output, and full decoder conformance remain
unsupported.
Emitted `splot decode` diagnostic rule IDs are registered in
[`DECODER-DIAGNOSTICS.md`](./DECODER-DIAGNOSTICS.md), enforced by
`cargo xtask check-diagnostic-registry`.

## Supported Tier

The first supported decode tier is implemented for hash JSON output,
atomically published raw sample output, and atomically published Y4M file output
on the committed minimal fixture. The
repository contract is:

```text
contract_id = "splot.decode.minimal_tier"
contract_version = 1
tier_id = "minimal-intra-8bit420-hash-v1"
feature_id = "DOC-MINIMAL-DECODE-TIER-CONTRACT"
runtime_feature_id = "DECODE-MINIMAL-TIER-RUNTIME-SUCCESS"
reconstruction_feature_id = "DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER"
raw_runtime_feature_id = "DECODE-MINIMAL-RAW-RUNTIME-OUTPUT"
y4m_runtime_feature_id = "DECODE-Y4M-RUNTIME-OUTPUT"
```

This is a `splot` implementation-supported subset, not an Annex A
level-conformant decoder claim. Annex A decoder conformance is broader than the
encoder-MVP subset below.

The tier is deliberately small:

- input is one committed IVF/DKIF-wrapped AV02 frame whose payload uses the
  Annex B length-delimited OBU framing; raw Annex B planning remains supported
  by the byte planner but is outside this runtime hash success tier;
- one selected stream/layer only: non-global OBUs use `obu_xlayer_id == 0`,
  `obu_tlayer_id == 0`, and inferred `obu_mlayer_id == 0`;
- no external HLS, multistream composition, sub-bitstream extraction, MSDO, LCR,
  Atlas, or OPS selection path;
- sequence format uses `seq_profile_idc == 0` (`Main_420_10_IP0`),
  `chroma_format_idc == 0`, `bit_depth_idc == 1` (8-bit),
  `max_tlayer_id == 0`, `max_mlayer_id == 0`, `SeqMaxMlayerCnt == 1`, and
  `film_grain_params_present == 0`;
- frame dimensions, tile counts, decoded-frame bytes, reference-store bytes,
  hash bytes, and output bytes pass `DecodeLimits` using checked arithmetic
  before allocation or output;
- accepted frames are closed-loop key-frame output only, with parsed facts
  proving `obu_type == OBU_CLOSED_LOOP_KEY`, `FrameType = KEY_FRAME`, and
  `FrameIsIntra = 1`;
- inline frame headers only: `cur_mfh_id == 0`, `frame_size_override_flag == 0`,
  `immediate_output_frame == 1`, `implicit_output_frame == 0`, and no sequence
  cropping window;
- one tile and one first-and-only tile group;
- deterministic decoded-frame hashes, raw sample files, and Y4M files are the first success
  artifacts; current runtime support is limited to the committed flat 64x64
  fixture, its traced six-symbol §8.2 tile stream, and its
  `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` handoff: the traced block
  symbols prove a luma-DC/no-residual recipe, the runtime builds a
  `splot-recon` current-frame workspace, predicts the 64x64 luma DC block,
  explicitly prepares the top-left chroma H_PRED no-neighbor left-edge
  fallback sample (`129` for this 8-bit tier), runs cardinal horizontal
  prediction for the traced U/V blocks, and freezes the workspace into the
  output frame. Broad edge preparation, chroma H/V beyond that traced fallback
  path, and broad reconstruction remain unsupported.

Runtime `splot decode` raw output is supported only for the committed minimal
IVF tier through `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`. The explicit form is
`--output-format raw -o <output>`. The CLI publishes the complete
`av2-output-samples-v1` sample byte stream atomically through a same-directory
temporary file and reports `decode/output-error` for publication failures. All
raw requests outside the minimal tier still emit a structured
unsupported/resource/malformed diagnostic without touching the requested output
path.

Runtime `splot decode` Y4M output is supported only for the committed minimal
IVF tier through `DECODE-Y4M-RUNTIME-OUTPUT`. The compatibility form
`splot decode <input> -o <output>` remains the implicit Y4M form, and
`--output-format y4m -o <output>` is the explicit Y4M form. The CLI publishes
that output atomically through a same-directory temporary file and reports
`decode/output-error` for publication failures. Hash success JSON uses the
separate `splot.decode.hash_report` schema rather than the diagnostic JSON
shape. All Y4M requests outside the minimal tier still emit a structured
unsupported/resource/malformed diagnostic without touching the requested output
path.

Everything outside the tier must fail explicitly with a structured diagnostic:
`decode/unsupported-feature` for unsupported tools or tier violations, and
`decode/resource-limit` for configured limit excess or overflow once that
diagnostic is emitted by source. Silent fallback to AVM, dav2d, ffmpeg, or any
other external decoder is forbidden.

## Stages

| Stage | Scope | Status |
|---|---|---|
| 0 | Roadmap, support matrix, generated status, drift gate | supported |
| 1 | Decode API contract, runtime context, limits, resource diagnostics, crate scaffolding, byte entry point | crate scaffolding, `DecodeContext` worker-pool runtime policy, limits runtime API, bounded byte-stream planning, and resource diagnostic emission supported |
| 2 | Shared decoded frame, plane, pixel format, workspace, and deterministic hash types | frame/plane model types, current-frame workspace, hash-input serialization, and `splot-dfh-sha256-v1` digest computation supported |
| 3 | CLI `splot decode` contract backed by library diagnostics | minimal hash JSON, minimal raw output, and minimal Y4M output supported; broad runtime output unsupported |
| 4 | Container traversal, base-layer parsed/raw traversal, transactional decode planning | parsed and raw-byte stream planners supported; operating-point selection and broad CLI runtime unsupported |
| 5 | Self-contained decode fuzz target and fixture smoke | `decode_plan_bytes` fuzz target supported for the raw byte planner; `decode_runtime_hash_bytes`, `decode_runtime_raw_bytes`, and `decode_runtime_y4m_bytes` supported for the minimal runtime byte APIs; minimal runtime fixture smoke supported |
| 6 | AV2 § 8 symbol/CDF decoder foundation | § 8.2 generic primitive supported (spec-exact, all arities, fuzzed, runtime-reachable); crate-private partition CDF subset boundary partial; supported-subset Tile-to-Saved/Saved-to-Frame lifecycle supported; broad § 8.3 and tile decode planned |
| 7 | Constrained intra tile syntax | tile payload and tile CDF boundaries partial; individual partition-entry symbol reads, one caller-fact partition decision, allowed partition derivation, first `decode_block()` frontier planning, and minimal block-symbol trace supported; supported-subset CDF lifecycle supported; full recursive `read_partition()` / `decode_tile()` syntax planned |
| 8 | Scalar intra prediction, dequant/reconstruction, inverse transform, frame hashes | current-frame workspace plus square DC, rectangular DC, subsampled DC, IBP DC, basic/PAETH, smooth, H/V cardinal directional, one-sided directional-angle, middle directional-angle primitives, workspace directional-angle handoff, and the minimal luma-DC plus traced top-left chroma H_PRED runtime handoff supported; luma/MRL/IDIF/full-dispatch directional angles, DIP/general directional-angle IBP/full CfL modes, broad chroma prediction, dequant/reconstruction, inverse transforms, broad runtime hashes planned |
| 9 | Raw/Y4M output and reconstructed reference-frame store | reference-slot runtime store, source-backed Y4M writer, minimal runtime raw output, and minimal runtime Y4M output supported; broad runtime raw/Y4M output and AV2 refresh semantics planned |
| 10 | Portable local-reference evidence manifests | metadata contract and offline checker wired; two AVM/dav2d raw MD5 agreement entries recorded as non-executable metadata |
| 11 | Encoder reconstruction API contract | planned |

## Runtime Concurrency Contract

Decoder and reconstruction work must follow the repository concurrency policy in
[`CONCURRENCY.md`](./CONCURRENCY.md), tracked by
`INFRA-PARALLEL-RUNTIME-POLICY`. This is project runtime policy, not an AV2
conformance rule, and it does not make the current unsupported decode entry
point byte-consuming.

The decoder/reconstruction ownership rule is:

- `splot-decode` owns runtime orchestration through `DecodeRuntimeConfig` and a
  `DecodeContext` with exactly one `splot_parallel::WorkerPool`;
- data-parallel decode work must reach Rayon traits only through
  `splot_parallel::prelude::*` and must run inside
  `ctx.pool().install(|| { ... })`;
- `splot-recon` stays pool-agnostic and must not construct worker pools, spawn
  codec worker threads, depend on Rayon/crossbeam directly, or own decode
  pipeline queues;
- bounded queues are allowed only through `splot_parallel::bounded_queue` at
  coarse producer/consumer boundaries, never for per-pixel, per-block, per-row,
  or other hot inner-loop signalling;
- future decoded-frame hashes, Y4M output, diagnostics, stats, progress events,
  and reference-state commits must be emitted in AV2 bitstream, presentation, or
  `splot` emission-index order, not worker completion order.

Future runtime decode rows may not be marked supported until self-contained tests
prove the supported behavior across all required thread-count forms:
`--threads 1`, `--threads auto`, and at least one fixed positive
`--threads N`. `--threads 0` remains a CLI/runtime alias for `auto`, resolved
once when the context-owned pool is created.

The current parsed stream planner is intentionally serial but runs through
`DecodeContext`, so future parallel planning or decode work already has the
context-owned `WorkerPool` boundary. Its tests prove identical plan metadata
across `ThreadCount::Auto`, one worker, and a fixed positive worker count. It
does not call direct Rayon/crossbeam APIs, does not construct queues, and does
not make `splot-recon` scheduler-aware.

## Spec Anchors

Decoder and reconstruction work must cite the committed AV2 v1.0.0 mirror:

- general decoding process: § 7.1,
  `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-1`;
- Annex B length-delimited input: Annex B.2-Annex B.3,
  `docs/spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2`;
- OBU syntax and OBU header semantics: § 5.2 and § 6.2,
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2`;
- sequence format, layer counts, and frame-size semantics: § 6.4.1 and
  § 6.17.4.1,
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1`;
- temporal units, coded extended layer units, and random access: § 7.3-§ 7.4,
  `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-3`;
- tile group and tile payload syntax: § 5.19-§ 5.20,
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`;
- parsing process and symbol/CDF decoding: § 8.2-§ 8.3,
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2`;
- prediction, reconstruction, inverse transforms, filters, output, and reference
  updates: § 7.13-§ 7.23,
  `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13`.

Do not infer AV2 syntax from AV1 projects or copy source, constants, tables,
comments, or prose from AVM, dav2d, rav1e, SVT-AV1, or any other implementation.

## Decode Limits Contract

Future byte-consuming decode entry points must accept explicit resource limits
before they allocate from bitstream-derived values. The source-backed runtime
API shape is:

```text
DecodeOptions {
    limits: DecodeLimits
}
```

This is repository policy layered over AV2 syntax-derived values, not an AV2
conformance rule. The diagnostic must cite the AV2 section that supplied the
measured value, while the configured threshold comes from `DecodeLimits`.
`splot-decode` now provides typed limit names, units, thresholds, actual values,
and pure check helpers for this contract. Those helpers are not decoder
diagnostics and do not read bytes, traverse OBUs, allocate frames, write output,
or change the current unsupported `splot decode` behavior.

The first contract covers:

- `max_input_bytes`;
- `max_obus`;
- `max_ivf_frame_records`;
- `max_frames_to_decode`;
- `max_output_frames`;
- `max_frame_width`;
- `max_frame_height`;
- `max_luma_samples_per_frame`;
- `max_decoded_frame_bytes`;
- `max_reference_slots`;
- `max_reference_store_bytes`;
- `max_tile_count`;
- `max_tile_partition_steps`;
- `max_tile_payload_bytes`;
- `max_output_bytes`.

The primary spec-derived surfaces are leb128 length fields (§ 4.11.6), Annex B
length-delimited input (Annex B.2-Annex B.3), OBU sizing (§ 5.2.1), sequence
maximum dimensions (§ 6.4.1), reference-frame count (§ 6.4.6), per-frame
dimensions (§ 6.17.4.1), tile grid counts (§ 6.17.7.2), tile group count and
semantics (§ 5.19 and § 6.18), tile payload traversal and semantics (§ 5.20.1
and § 6.19.1), the general decode input/output model (§ 7.1), decoded output
arrays (§ 7.21), and reference frame storage (§ 7.23). The byte-stream planner
checks `max_input_bytes` before traversing accepted input bytes, checks
`max_obus` before continuing OBU traversal or accumulating OBU state, and checks
`max_ivf_frame_records` before traversing each complete IVF frame record. Future
runtime stages must check the relevant derived resource limit before allocating
decoded frames, traversing tile payloads, storing reference frames, producing
frame hashes, or writing Y4M output.

Every derived `actual` resource value must be computed with checked arithmetic
before comparison or allocation. Overflow while deriving dimensions, strides,
tile products, plane sizes, reference-storage bytes, output bytes, or frame
counts is a `decode/resource-limit` failure, not a wraparound or panic.
Runtime emission of that diagnostic remains future work and belongs at the
byte-consuming decode boundary.
`DecodeLimits::zero()` and `DecodeLimits::unlimited()` are explicit constructors
for tests and callers; `DecodeLimits::default()` and `DecodeOptions::default()`
use finite repository policy thresholds for CI, fuzzing, and early decoder work.

`DecodeContext::plan_stream` applies only the limits it can derive from an
already parsed stream: `max_input_bytes` from the caller-supplied input length,
`max_obus` before adding each planned OBU, `max_ivf_frame_records` before
traversing each IVF frame record, and `max_frames_to_decode` before accepting
each closed-loop-key frame candidate. `DecodeContext::plan_bytes` is the first
raw byte-consuming planner: it performs bounded raw Annex B / IVF traversal and
then reuses the same selected-frame-candidate limit and unsupported-structure
classification as the parsed planner. It is still plan-only and does not parse
tile payloads or allocate decoded frames.

## Decoded Frame and Plane Model Contract

Decoded-frame data structures must preserve AV2 output semantics while
remaining reusable by future reconstruction, reference-frame storage, hashes,
Y4M, and encoder closed-loop tests. The source-backed `splot-recon` model now
provides:

```text
DecodedFrameInfo
DecodedFrame
FramePlanes<T>
Plane<T>
PlaneSize
PlaneRect
PixelFormat
BitDepth
OutputIndex
ReferenceSlot
ReferenceFrameStore<F>
ReferenceFrameEntry<'a, F>
ReferenceFrameEntries<'a, F>
ReconError
```

This is a committed Rust output-model API, not a byte-consuming decode API.
The model validates AV2-derived frame/plane geometry, sample storage, and
reference-slot container bounds, but does not reconstruct pixels, compute
hashes, write Y4M, or implement AV2 reference refresh semantics.

`PixelFormat` is derived from AV2 § 6.4.1 `chroma_format_idc`:

- `Monochrome` / 4:0:0: `SubsamplingX = 1`, `SubsamplingY = 1`,
  `NumPlanes = 1`;
- `Yuv420`: `SubsamplingX = 1`, `SubsamplingY = 1`, `NumPlanes = 3`;
- `Yuv422`: `SubsamplingX = 1`, `SubsamplingY = 0`, `NumPlanes = 3`;
- `Yuv444`: `SubsamplingX = 0`, `SubsamplingY = 0`, `NumPlanes = 3`.

`BitDepth` is derived from AV2 § 6.4.1 `bit_depth_idc`: AV2 v1.0.0 permits
10-bit samples for `bit_depth_idc = 0` and 8-bit samples for
`bit_depth_idc = 1`. Future decoded sample storage must reject values outside
`0..=(1 << bit_depth) - 1`.

The model must distinguish coded/reconstructed storage from cropped output:

- coded luma dimensions are `FrameWidth x FrameHeight` (§ 6.17.4.1);
- the visible output luma rectangle is `CropLeft`, `CropTop`, `CropWidth`, and
  `CropHeight`; `CropWidth` and `CropHeight` must be positive, and non-monochrome
  crop origins must be aligned to `SubsamplingX` / `SubsamplingY` (§ 6.17.4.4);
- decoded output frames are AV2 § 7.21 `OutY`/`OutU`/`OutV` arrays emitted by
  the AV2 output processes (§ 7.1, § 7.21.5, § 7.21.6);
- `splot` assigns a zero-based emission index in that output-process order after
  supported stream/layer selection; this index is repository-owned metadata, not
  an AV2 syntax element, and it is not decode order;
- output luma dimensions are `w x h` from § 7.21.2, and output chroma dimensions
  are `((w + subX) >> subX) x ((h + subY) >> subY)`;
- U and V planes are absent or ignored when `NumPlanes == 1`.

`Plane<T>` may include padding for efficient storage, and it carries explicit
storage `width`, storage `height`, `stride_samples`, and visible rectangle
metadata when storage and visible output differ. Invariants:

- `stride_samples >= storage_width`;
- `required_samples = stride_samples * storage_height` is computed with checked
  arithmetic;
- the backing buffer exposes exactly `required_samples` samples, and
  `allocation_bytes = required_samples * bytes_per_sample` is computed with
  checked arithmetic before reporting backing size;
- every product used for dimensions, strides, backing samples, byte sizes, hash
  lengths, Y4M output, or reference storage uses checked arithmetic;
- `splot-recon` constructors reject local arithmetic overflow with typed
  `ReconError` values and do not emit decoder diagnostics directly;
- future byte-consuming decode code must charge the full backing allocation,
  including padding, against `DecodeLimits` before allocation;
- future byte-consuming decode code reports allocation overflow or
  configured-limit excess as `decode/resource-limit`;
- padding and stride samples are not visible decoded output and must be excluded
  from frame hashes, Y4M output, and fixture expectations.

Reference-frame storage is related but not the same shape as output. AV2 § 7.23
stores loop-restored `LrFrame` into `FrameStore` over padded coded dimensions
(`MiCols * MI_SIZE` by `MiRows * MI_SIZE` for luma, shifted by subsampling for
chroma) and records reference metadata such as `RefFrameWidth`,
`RefFrameHeight`, `RefCropWidth`, `RefCropHeight`, `RefCropLeft`, `RefCropTop`,
`RefSubsamplingX`, `RefSubsamplingY`, `RefBitDepth`, `RefNumPlanes`,
`RefOutputOrder`, `RefOrderHint`, and `RefFilmGrainPresent`. Future APIs must
not treat cropped output-frame dimensions and reference-store backing dimensions
as interchangeable.

The source-backed `splot-recon` reference store is a safe runtime container for
future callers that have already derived AV2 reference update decisions:

- `ReferenceSlot::MAX_SLOTS == 16`, matching AV2 § 3 `NUM_REF_FRAMES`;
- `ReferenceSlot::new` validates indices in `0..16`;
- `ReferenceRefreshMask::new` validates caller-derived refresh masks within the
  16-slot ceiling and iterates selected slots in ascending order;
- `ReferenceFrameStore<F>::with_capacity` validates a fixed capacity in
  `1..=16`;
- `put`, `get`, `take`, `refresh_slots_with`, `clear`, `occupied`, and
  `entries` manage immutable caller-owned frame payloads without exposing
  mutable frame access;
- `refresh_slots_with` validates every selected mask bit against the fixed
  store capacity before calling the payload producer, treats a zero mask as a
  no-op, returns replaced payloads, and does not require `F: Clone`;
- entries iterate occupied slots in ascending `ReferenceSlot` order.

The payload type is intentionally generic so future reference/reconstruction
payloads do not need to fabricate output-emission metadata just to live in the
store. This runtime store does not model active `NumRefFrames` or
`ActiveNumRefFrames` from § 5.4.6 / § 6.4.6, AV2 `RefValid`,
`refresh_frame_flags` parsing/inference, output scheduling, show-existing
deduplication, counters, order hints, dimensions, crop metadata, motion vectors,
CDFs, grain params, segment IDs, global motion state, CCSO params, or any other
§ 7.23 metadata. Future byte-consuming decode code must translate parsed AV2
state into store operations and charge allocations against `DecodeLimits` before
storing frames.

Emitted output frames must remain immutable and valid after emission. Reference
slots may own or share reconstructed buffers, but overwriting a reference slot
must not mutate an already emitted output frame. Borrowed or shared views are
allowed only when the backing samples are immutable for the output view, when the
output owns an independent copy, or when copy-on-write or unique ownership is
proven before any reference-slot mutation.

## Hash Policy

Frame hashing was the first supported runtime output proof and remains the
canonical deterministic sample identity check. The first repository-owned
contract is:

```text
contract_id = "splot.decoded_frame_hash"
contract_version = 1
algorithm_id = "splot-dfh-sha256-v1"
byte_stream_id = "av2-output-samples-v1"
```

The `splot-dfh-sha256-v1` digest is SHA-256 over canonical decoded output sample
bytes. The sample-byte stream follows AV2 § 6.16.13's decoded-frame-hash sample
serialization, but the digest is `splot`-owned fixture and roundtrip identity,
not the AV2 metadata MD5 value. AV2 `hash_type = 0` MD5 remains a separate
future verification path for `METADATA_TYPE_DECODED_FRAME_HASH` metadata.

The canonical byte stream is defined as follows:

- frame order uses the `splot` zero-based emission index assigned in AV2
  output-process order after supported stream/layer selection, including
  show-existing and flush output frames once those output paths are implemented;
- region is cropped visible output only: luma dimensions are `w x h`; chroma
  dimensions are `((w + subX) >> subX) x ((h + subY) >> subY)`;
- backing allocation padding and `Plane` stride bytes are excluded;
- non-monochrome plane order is Y, then U, then V; monochrome output hashes only
  Y;
- samples are traversed left-to-right, top-to-bottom within each plane;
- 8-bit samples are written as one byte;
- samples with bit depth greater than 8 are written as two bytes in little-endian
  order, least significant byte first, with no normalization;
- codec metadata, OBU bytes, container timestamps, HDR/ICC/timecode metadata,
  and signaled decoded-frame-hash metadata are excluded from the digest input
  and must be asserted separately when relevant.

`splot-recon` source-backs this contract with
`DecodedFrameHashInput<'_, T>` and `DecodedFrameHash`. The input API serializes
a caller-supplied `DecodedFrame<T>`'s modeled visible rows and exposes
`byte_stream_id = "av2-output-samples-v1"` plus
`variant_id = "raw_intermediate_output"`. The digest API computes
`algorithm_id = "splot-dfh-sha256-v1"` over that same byte stream and exposes
raw 32-byte digest access plus lowercase hex formatting. These APIs do not
verify AV2 metadata MD5, select output order, synthesize film grain, read
bitstreams, write Y4M, reconstruct pixels, or invoke AVM/dav2d.

The default hash variant is `raw_intermediate_output`, corresponding to
AV2 § 6.16.13 `has_grain = 0`: `OutY`/`OutU`/`OutV` from the § 7.21.2
intermediate output preparation process before § 7.21.7 film-grain synthesis.
A post-film-grain hash may be added later only as an explicit, separately named
variant after film-grain synthesis is implemented and tested.

Local AVM/dav2d output can be useful evidence, but committed `splot` tests
must not require those tools. The checked local-reference evidence manifest
records AVM/dav2d raw MD5 agreement for two background fixtures and raw SHA-256
agreement for the committed minimal runtime hash fixture; it is non-executable
metadata only and does not add an external decoder dependency. Future decoder
local-reference evidence also belongs in
[`LOCAL-REFERENCE-EVIDENCE.toml`](./LOCAL-REFERENCE-EVIDENCE.toml), which is
checked by `cargo xtask check-reference-evidence` and
`cargo xtask check-decoder-support`. The manifest stores portable metadata only:
repo-relative fixture identity, upstream reference-tool identity, sanitized
command summaries, digest metadata, and assertions.

## Unsupported Feature Contract

Decoder unsupported-feature output carries structured data. Hash/raw/Y4M
requests for planable inputs outside the minimal runtime tier emit this
diagnostic after byte planning succeeds:

```json
{
  "rule_id": "decode/unsupported-feature",
  "severity": "Error",
  "spec_section": "7.1",
  "matrix_row": "minimal-decode-tier-contract",
  "feature_id": "DECODE-MINIMAL-TIER-RUNTIME-SUCCESS",
  "message": "minimal tier requires exactly three planned OBUs, one frame candidate, and no source warnings",
  "remediation": "Use a stream inside minimal-intra-8bit420-hash-v1 or wait for the referenced decoder support row.",
  "detail_kind": "unsupported_feature",
  "unsupported_reason": "unexpected_planned_stream_shape",
  "tier_id": "minimal-intra-8bit420-hash-v1",
  "output_format": "y4m"
}
```

Planner-level unsupported structures also use `decode/unsupported-feature`, but
with `decode-stream-state` / `DECODE-STREAM-STATE-PLANNER` metadata and details
such as `unsupported_reason`, `obu_type`, and `byte_offset`.
Runtime hash tier rejections use `minimal-decode-tier-contract` /
`DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` metadata plus `unsupported_reason`,
`tier_id`, and an optional byte offset.

The CLI renders diagnostics as text by default and as JSON with
`splot decode --json`. Library-facing decode diagnostics must preserve stable
field names for tests and encoder roundtrips. The emitted `rule_id` set is
registered in [`DECODER-DIAGNOSTICS.md`](./DECODER-DIAGNOSTICS.md).

Resource-limit diagnostics now use `decode/resource-limit` for byte-planner
limit failures. The diagnostic extends the stable decoder fields with
`limit_name`, `limit`, `actual`, `unit`, `byte_offset`, and `bit_offset`.

## Local References

AVM and dav2d are local development aids only. They may be used to read source
code, generate tiny streams, compare decoded hashes, or record evidence in
agent logs, PR descriptions, and portable manifests.

They must not be added as:

- source, submodule, vendored tree, binary, object file, generated binding, or
  copied snippet;
- Cargo dependency, build dependency, `build.rs` probe, wrapper script, or
  runtime process execution;
- `xtask` command, CI job, Docker image, cache, default test path, or required
  developer setup.

Committed evidence must be portable: no local absolute paths and no assumption
that CI will rerun AVM or dav2d.

## Crate Split

Maintainer approval for the decoder/reconstruction dependency graph landed on
2026-06-13. The approved crate split is now scaffolded as:

```text
crates/splot-core      bitstream model + parsers
crates/splot-recon     decoded output model types; hash-input bytes, frame hashes, Y4M writer; future reconstruction primitives, references
crates/splot-parallel  approved local worker-pool and bounded-queue runtime policy
crates/splot-decode    diagnostic API; runtime context; stream planners; minimal hash runtime using splot-recon
crates/splot-encode    future encoder with private splot-recon dependency boundary
crates/splot-cli       thin CLI rendering splot-decode diagnostics
```

The scaffold is still an ownership boundary for decode. `splot-recon` exposes a
runtime decoded output frame/plane model, reference-slot container,
deterministic hash-input byte serializer, `splot-dfh-sha256-v1` digest API, and
Y4M writer for caller-supplied decoded frames, but no reconstruction algorithm,
output scheduling, or AV2 reference refresh process.
`splot-decode` owns the decode scheduler boundary through
`DecodeRuntimeConfig` and `DecodeContext`, whose single `WorkerPool` is sized by
the CLI/runtime `--threads` policy. It now depends on `splot-core` for parsed
stream-planner input and the bounded raw-byte `plan_bytes` planner,
`splot-parallel` for worker-pool execution, and `splot-recon` for the narrow
minimal runtime decoded frame/hash/raw/Y4M handoff. Broad tile decode, pixel
reconstruction, broad raw/Y4M output, and reference update semantics remain
unsupported.
`splot-cli` reads input bytes for `splot decode`, calls the plan-only
or minimal runtime `splot-decode` handoff, renders structured diagnostics,
emits hash JSON for the supported minimal tier, and atomically publishes raw or
Y4M output for the same minimal tier. `splot-encode` now has only a private
`splot-recon` dependency boundary; public encoder/reconstruction API reuse
remains future work.
