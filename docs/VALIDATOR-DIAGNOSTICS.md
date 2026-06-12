# Validator diagnostics registry

`status: enforced`
`owner: validator`
`purpose: the canonical, CI-enforced list of every diagnostic rule id the validator emits`

> **Canonical diagnostic registry (CI-enforced).** The tables in the marker-delimited
> region below (between the `diagnostics-registry:begin` and `:end` HTML comments) are the
> single source of truth for validator diagnostic rule IDs.
> `cargo xtask check-diagnostic-registry` (run inside `cargo xtask ci`, tracked as
> `XTASK-DIAGNOSTIC-REGISTRY`) fails if any rule-id literal in `crates/splot-validate/src`
> is missing from these tables, or if a table lists an ID that is not present in the source.
> The gate enforces the rule-ID *set*; the `Severity` and `Section` columns are maintained by
> hand. The planned-diagnostics backlog in
> [`VALIDATOR-ROADMAP.md`](./VALIDATOR-ROADMAP.md) feeds into this registry as its
> diagnostics land. The extractor lives in `xtask/src/diagnostic_registry.rs`.

Diagnostics are the validator product. Every finding carries:

- a stable `rule_id`;
- a `severity` (`error`, `warning`, `info`);
- an optional `spec_section`;
- optional byte offset and bit offset;
- a human-readable message;
- test coverage when the owning feature is marked proven in `docs/IMPLEMENTATION-MATRIX.toml`.

<!-- diagnostics-registry:begin -->

## Emitted diagnostics

Every rule ID below is emitted by `crates/splot-validate/src`, grouped by namespace. The
`Section` column cites the AV2 v1.0.0 conformance section the check derives from.

### `annex-a/`

Annex A profile/level/tier **static** constraints (`AV2-A-PROFILES` / `AV2-A-LEVELS-TIERS`),
grounded in `docs/spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md`. Rate-based and
decoder-model constraints (Annex E) are out of scope. Hand-crafted unit vectors only —
`avm_diff` is never claimed for them (no AVM differential oracle has been run yet).

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `annex-a/frame-size-below-minimum` | error | § A.4 | a parsed intra frame's FrameWidth or FrameHeight is below 16 under a table-mapped seq_level_idx (not 31, not reserved) |
| `annex-a/frame-size-exceeds-level` | error | § A.4 | FrameWidth*FrameHeight > MaxPicSize, FrameWidth > MaxHSize, or FrameHeight > MaxVSize for the activated seq_level_idx (Tables A.8) |
| `annex-a/high-tier-below-4-0` | warning | § A.4 | a High tier is signaled below level 4.0 (Table A.9 NOTE — informative, hence advisory). The reachable case is an OPS-signaled ops_tier_flag == 1 with ops_level_idx < 4 (the OPS PTL carries ops_tier_flag unconditionally, § 5.11.2). The sequence-header arm (seq_tier High with seq_level_idx < 4) is syntax-unreachable because seq_tier is only signaled for seq_level_idx > 3, and is kept as a defensive guard. |
| `annex-a/lcr-required-for-iop` | error | § A.2 | a Table A.4 interoperability-point row requires a local or activated global OBU_LAYER_CONFIGURATION_RECORD (or the IOP2 either-or combinations) for the coded video sequence and none is present (Table A.4, mirror lines 191/197/199-200) |
| `annex-a/level-reserved` | error | § A.4 | an activated seq_level_idx, or an observed ops_level_idx, is in the reserved range 22-30 (Table A.7) |
| `annex-a/msdo-prohibited-for-iop` | error | § A.2 | a Table A.4 row prohibits an OBU_MSDO (a single-extended-layer coded video sequence) but one is present (Table A.4 MSDO Prohibited rows; documented defensive arm — a present MSDO declares E > 1 under Table A.3, so unreachable in-band today) |
| `annex-a/msdo-required-for-iop` | error | § A.2 | a Table A.4 row requires an OBU_MSDO (a multi-extended-layer coded video sequence, or the IOP2 MSDO-or-activated-global-LCR either-or) and none is present (Table A.4, mirror lines 185/189/195) |
| `annex-a/profile-bit-depth-mismatch` | error | § A.2 | bit_depth_idc is not 0 or 1 for a profile in 0-4 (Table A.1; defensive — the parsed bit_depth_idc only models 0/1, so unreachable today) |
| `annex-a/profile-chroma-format-mismatch` | error | § A.2 | chroma_format_idc is outside the activated profile's allowed set (Table A.1; profile 31 / reserved profiles are skipped) |
| `annex-a/profile-reserved` | error | § A.2 | seq_profile_idc, an observed ops_seq_profile_idc, or a multistream_profile_idc (§ 6.6 binds its value space to seq_profile_idc / Table A.1; the spec's "Table A.4" cross-reference is an erratum) is in the reserved range 5-30 (Table A.1) |
| `annex-a/tile-count-exceeds-level` | error | § A.4 | a parsed intra frame's NumTiles > MaxTiles or TileCols > MaxTileCols for the activated seq_level_idx (Table A.9) |

### `atlas/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `atlas/duplicate-input-stream-id` | error | § 6.9.6 | ats_input_stream_id / ats_msi_input_stream_id values are not unique |
| `atlas/local-atlas-unavailable` | error | § 7.3.8.4 | local LCR references lcr_local_atlas_id with no available local atlas (external disabled) |
| `atlas/multistream-requires-global-xlayer` | error | § 6.9 | multistream atlas mode does not use GLOBAL_XLAYER_ID |
| `atlas/region-dimension-out-of-range` | error | § 6.9.3.1 | an atlas region dimension is out of range |
| `atlas/segment-count-out-of-range` | error | § 6.9.6 | atlas segment count is out of range |
| `atlas/segment-mode-out-of-range` | error | § 6.9 | ats_atlas_segment_mode_idc is out of range |

### `bitstream/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `bitstream/parse-error` | error | varies | a payload parse / EOF / malformed-descriptor error (spec section set per call site) |

### `brt/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `brt/ops-count-mismatch` | error | § 6.11 | BRT br_ops_cnt differs from the active OPS ops_cnt |
| `brt/unavailable-operating-point-set` | error | § 7.3.8.5 | BRT `(obu_xlayer_id, br_ops_id)` has no active in-band OPS and no matching external-HLS OPS declaration (per-object, unlike the coarse external-disabled gate on the other availability checks) |

### `byte-alignment/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `byte-alignment/zero-bit-not-zero` | error | § 6.2.4 | a byte_alignment() padding zero bit is non-zero |

### `celu/`

Coded-extended-layer-unit (§7.3.6) constraints and the §7.3.7/§7.4.6 display-order-hint (DOH)
constraints (`AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`, `AV2-7.3.7-TEMPORAL-UNIT-ORDER`). Built above
the FrameUnitSegmenter (`crates/splot-validate/src/celu.rs`), keyed per `obu_xlayer_id` across a
temporal unit. Disjoint from `obu-order/non-global-hls-before-coded-layer` (which owns the
HLS-header-after-frame-region case) and `frame-unit/ci-not-in-first-frame-unit` (the §7.3.8.10
temporal-unit-scoped CI form). Output-class- and OrderHint-derived judgments drop to silence when
undecidable (the Unknown invariant); the DOH checks fire only when the temporal unit's
`lcr_doh_constraint_flag` / `multistream_doh_constraint_flag` is 1. Hand-crafted unit vectors only.

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `celu/clk-olk-mixed` | error | § 7.3.6 | a coded extended layer unit contains both an OBU_CLOSED_LOOP_KEY and an OBU_OPEN_LOOP_KEY (mirror line 554). CLK/OLK identity is type-decided and boundary-independent, so it is recorded for every frame-bearing OBU regardless of its coded-frame-unit boundary — an `Ambiguous`-boundary OBU (an OLK plus an ambiguous CLK) and a `ContinuesUnit`-boundary OBU (a CLK opener plus an OLK-typed non-first tile group, which the segmenter also flags `frame-unit/mixed-coded-frame-types`) both still fire |
| `celu/content-interpretation-not-in-first-celu` | error | § 7.3.6 | an OBU_CONTENT_INTERPRETATION is present for an embedded layer in a coded extended layer unit that is NOT the first coded extended layer unit of the coded video sequence, but the sequence's first CELU carried no CI for that embedded layer (mirror lines 560-562, the PRESENCE half of "a CI present in any CELU shall also be present in the first CELU of the sequence ... for a given embedded layer"; the contents-identity half is owned by `content-interpretation/repeated-ci-not-identical`, § 6.14). Coded-video-sequence-scoped, so implemented in `crates/splot-validate/src/context.rs` (not the per-TU CELU tracker): per extended layer, the embedded layers whose CI appeared in the CVS's first CELU (its CELU in the first temporal unit, since a CVS starts at the CLK temporal unit, § 7.3.6) are recorded and reset per CVS; a later CELU adding a CI for an embedded layer the first CELU lacked fires once per (xlayer, mlayer, CVS epoch), anchored at the offending CI. Resolved at the temporal-unit boundary (and end of bitstream) because the whole temporal unit containing a CLK belongs to the new CVS — a CI's CELU membership is final only once the unit is complete. DROPS when the first CELU of the CVS was not observed: a mid-CVS join (no CLK seen for the layer, so the implicit CVS began before the first observed OBU), or any external-HLS Provided mode (an external CI in the first CELU cannot be enumerated by ExternalHlsSet, which expresses only sequence headers and operating point sets). Distinct from `celu/content-interpretation-not-in-first-unit` (the CELU-scoped first-FRAME-UNIT form, lines 557-559) and `frame-unit/ci-not-in-first-frame-unit` (the §7.3.8.10 temporal-unit form) |
| `celu/content-interpretation-not-in-first-unit` | error | § 7.3.6 | an OBU_CONTENT_INTERPRETATION appears outside the first frame unit of its embedded layer within the coded extended layer unit (the CELU-scoped form; the §7.3.8.10 temporal-unit form is `frame-unit/ci-not-in-first-frame-unit`) |
| `celu/doh-order-hint-bits-mismatch` | error | § 7.3.7 | with a DOH constraint flag set, frame units in one temporal unit carry different OrderHintBits (mirror line 655 — constraint (1), judged over EVERY frame unit of the temporal unit; the §7.3.7 cross-CELU OrderHint constraint (2) is the separate `celu/doh-order-hint-mismatch`, gated per compared output-unit pair, round-6 F2 — this rule no longer gates that comparison). The mismatch is recorded ONLY between two KNOWN OrderHintBits values and **fires regardless of any undecidable frame unit in the same temporal unit** (round-7 F4): an undecidable (None) participant can only prevent proving AGREEMENT (never reported) and cannot make a pair already proven to differ conforming, so a None bits value is simply not recorded — it neither establishes nor suppresses a mismatch. The accumulator is fed per frame UNIT, not per OBU (F1): an `OpensNewUnit` OBU notes its unit's resolved bits (or None when its opener does not resolve), an `Ambiguous` OBU notes None (it might open a unit with unknowable bits — e.g. a same-type no-delimiter bridge / TIP, round-7 F2), and a `ContinuesUnit` OBU (a non-first tile group of an already-counted coded frame) is skipped — so a continuation contributes no redundant value. A frame contributes its OrderHintBits only when its referenced sequence header resolved to the active header it parsed against (the stale-activation guard); a frame referencing an absent / stale header contributes None rather than the stale active header's bits. A global frame-bearing OBU (obu_xlayer_id == GLOBAL_XLAYER_ID, itself invalid and diagnosed by `obu-header/global-xlayer-allowed-types`) is excluded before the accumulator — it is not part of any CELU (§ 7.3.6) and never resolves an active header, so it no longer feeds a spurious None |
| `celu/doh-order-hint-mismatch` | error | § 7.3.7 | with a DOH constraint flag set, coded output frame units in different coded extended layer units of one temporal unit carry different OrderHint (mirror lines 656-657 / 1316-1320). Compared via the `order_hint` LSB **proxy** for the decoded OrderHint (`get_disp_order_hint`; a named residual blocked on reference-state modelling — AV2-5.18.2-FRAME-HEADER-INFO). Because equal decoded OrderHints can carry different-width LSB encodings, the cross-CELU comparison is **gated PER COMPARED PAIR on the two output CELUs' own OrderHintBits being known and equal** (round-6 F2): the per-output-unit bits are carried alongside each output-unit OrderHint sample, NOT taken from the temporal-unit-wide same-bits judgment. To realise that per-pair gate across ALL CELUs rather than only against the first sample, output-CELU samples are **GROUPED BY their known OrderHintBits value** and each is compared to its own group's representative (round-8 F3): e.g. samples (bits, hint) = (4, 0), (5, 1), (5, 2) prove the (5, 1)/(5, 2) WITHIN-GROUP mismatch that a compare-only-to-the-first-sample scheme missed (both (5, *) samples fail the equal-bits gate against the (4, 0) representative). An unknown-bits output sample stays out of all groups; one diagnostic is emitted per temporal unit (the first within-group disagreement found), anchored at the offending later sample. §7.3.7 has two distinct constraints — (1) all frame units share one OrderHintBits (`celu/doh-order-hint-bits-mismatch`, over EVERY frame unit) and (2) output units in multiple CELUs share one OrderHint (this rule). An unknown-bits non-output (or otherwise unrelated) frame unit elsewhere in the temporal unit drops constraint (1) but does NOT suppress a decidable constraint (2) mismatch between two output CELUs whose own bits are known and equal. When a compared pair's bits are known but UNEQUAL they land in different groups, so this comparison never compares them (unsound cross-width proxy); `celu/doh-order-hint-bits-mismatch` fires instead. A mismatch recorded between two known equal-bits output CELUs **fires regardless of a third output CELU whose own OrderHint is undecidable** (round-7 F5): such an undecidable CELU is not contributed to the cross-CELU comparison at all (feeding the known units' value would be a guess), so a recorded mismatch is always proven between two fully-decidable CELUs and an undecidable participant cannot excuse it. The DOH flag for a completed temporal unit (shared with `celu/doh-order-hint-bits-mismatch`) is the `multistream_doh_constraint_flag` of the preceding MSDO **or** the `lcr_doh_constraint_flag` of the activated global LCR of the CMVS that **contains** that temporal unit (§ 7.3.7 "for each temporal unit in the CMVS"). The LCR side is sampled against that GOVERNING CMVS window — captured before the § 7.3.2 begin/end conditions mutate the live window — so a CLK boundary temporal unit that ENDS the CMVS (end condition 2, clearing the live window) is still governed by the ending CMVS's activated global LCR rather than reading the flag false (the MSDO side is window-independent live last-wins state). The same governing-window capture is mirrored at the END OF THE BITSTREAM (round-6 F1): the final temporal unit, which has no trailing temporal delimiter, samples the flag against the CMVS window captured before the end-of-stream `complete_temporal_unit` clears it, so a CLK final unit that ends the CMVS at EOF is still governed by its containing CMVS's activated global LCR |
| `celu/in-unit-order` | error | § 7.3.6 | an HLS header appears after a later HLS-header phase (LCR → OPS → atlas → sequence header), or a frame unit opens at a lower obu_mlayer_id than an earlier one, or a coded frame unit opens in an embedded layer after that layer's coded output frame unit (the per-layer output-slot grammar: zero or more non-output units then zero or one output unit, the output unit last — mirror lines 521-529). The ascending-obu_mlayer_id ordering counts every frame-unit constituent at its own obu_mlayer_id — the head (CI) and interior (BRT/QM/FGM/prefix-metadata/MFH) OBUs as well as the coded frame — so a head heading a higher embedded layer's unit makes a later lower-mlayer coded frame out of order (a suffix metadata shares its frame's mlayer, so it never lowers the high-water mark). The ordering is boundary-independent (F3): a `ContinuesUnit` frame OBU (a non-first tile group) belongs to its opener's already-begun unit and an `Ambiguous` frame OBU belongs to some layer-m unit either way, so both participate in the ascending-mlayer accounting — e.g. mlayer0 first tile, mlayer1 frame, mlayer0 non-first tile fires. Coded-frame-unit boundaries come from the FrameUnitSegmenter (the single source of truth). The output-slot grammar is a DECIDED-PAIR-ORDER judgment, so an intervening Ambiguous boundary does NOT drop it (round-8 F1): once a decided output unit consumes the slot, any LATER DECIDED (`OpensNewUnit`) unit in the layer fires regardless of an earlier/intervening ambiguous OBU — the ambiguity changes only the unit COUNT/INDEX, never the relative order of the two decided units (no resolution can merge an `OpensNewUnit` split into the output unit). An ambiguous OBU with NO later decided unit stays silent (its own existence as a separate unit is undecided). Disjoint from `obu-order/non-global-hls-before-coded-layer` (the HLS-header-after-frame-region case) |
| `celu/key-not-in-first-unit` | error | § 7.3.6 | an OBU_CLOSED_LOOP_KEY / OBU_OPEN_LOOP_KEY opens a frame unit that is not the first coded frame unit of its embedded layer within the coded extended layer unit (mirror lines 543-545 / 551-553). This is a DECIDED-PAIR-ORDER judgment, gated on "a DECIDED earlier unit already exists in the layer" (`units_opened >= 1`, counting only decided `OpensNewUnit` units), NOT on the blanket ambiguity poison (round-8 F2): when a decided earlier unit precedes the key, the key is provably not the layer's first coded frame unit WHATEVER an intervening ambiguous OBU was, so it FIRES. The ASYMMETRIC dropped case is when the key ITSELF is the first DECIDED unit (`units_opened == 0`) while an ambiguous OBU preceded it — the ambiguous OBU might or might not have been an earlier unit, so "not first" rests on a guess and stays silent. (The sibling `celu/lowest-layer-not-key` keeps the same asymmetry by construction: its predicate is the identity of the lowest layer's FIRST unit, which an ambiguous OBU before the candidate first unit leaves unknown — no decided-pair-order reformulation exists, so it stays dropped.) |
| `celu/leading-frame-mix` | error | § 7.3.6 | a coded extended layer unit mixes leading and non-leading frame units (mirror lines 555-556). Leading-ness is the tri-state Leadingness (mirror AVM is_leading_picture, av2/decoder/obu.c:2544-2549): LEADING_* is Leading, the IsRegular==1 set (OLK / REGULAR_* / SWITCH / RAS / BRIDGE) is Regular, a CLK is Indeterminate and is excluded from the judgment entirely (the § 6.4.1 gloss at 06-syntax-structures-semantics.md:4546 would class a CLK as leading, but the oracle does not — documented sound under-report). Leading-ness is type-decided and boundary-independent, so it is recorded for every frame-bearing OBU regardless of its coded-frame-unit boundary — an `Ambiguous`-boundary OBU (a LEADING frame plus an ambiguous Regular-typed OBU) and a `ContinuesUnit`-boundary OBU (a LEADING opener plus a REGULAR-typed non-first tile group) both still fire. Fires only when a Leading unit and a Regular unit coexist |
| `celu/lowest-layer-not-key` | error | § 7.3.6 | a coded extended layer unit contains a CLK (resp. OLK) but the lowest embedded layer's first coded frame unit is not a CLK (resp. OLK) (mirror lines 543-545 / 551-553) |
| `celu/missing-output-frame-unit` | error | § 7.3.6 | a coded extended layer unit contains no coded output frame unit (mirror line 536). Fires for a header-only CELU (≥ 1 non-padding constituent OBU at a non-global obu_xlayer_id, zero frame-bearing OBUs) and for a frame-bearing CELU with ≥ 1 output-class-decided frame unit and no output unit. Output is type-decided by obu_type for a SEF (output) and an OBU_BRIDGE_FRAME (non-output) before any flag parse, so a bridge-only CELU fires. A `cur_mfh_id > 0` (MFH-backed) frame is now decidable (F2): when its in-band multi-frame header (§ 7.3.8.7) resolves to the active sequence header it parsed against (`mfh_seq_header_id` equals that id), the § 5.18.2 core parse reaches the output flags using the active header alone, so the frame's output class decides rather than routing to Unknown; an externally-only-declared, out-of-range, absent, or different-header MFH keeps Unknown. Dropped when a frame-bearing unit's output class is Unknown, or when an Ambiguous-boundary OBU could itself be an output unit (i.e. its output class is not type-decided non-output — an ambiguous bridge does not poison, since it can never satisfy output presence). A padding-only / reserved-type-only obu_xlayer_id group never constitutes a CELU |
| `celu/non-output-without-output` | error | § 7.3.6 | an embedded layer of a coded extended layer unit has a coded non-output frame unit but no coded output frame unit (mirror lines 537-538). Output is type-decided by obu_type for a SEF (output) and an OBU_BRIDGE_FRAME (non-output), so a bridge-only embedded layer alongside an output layer fires. An MFH-backed (`cur_mfh_id > 0`) frame whose in-band multi-frame header resolves to the active header is decidable here too (F2), rather than routing to Unknown. Dropped for a layer whose output class was Unknown, or poisoned by an Ambiguous-boundary OBU that could itself be the layer's output unit (an ambiguous bridge does not poison) |
| `celu/output-order-hint-mismatch` | error | § 7.3.6 | coded output frame units in one coded extended layer unit carry different OrderHint (mirror lines 539-540). Compared via the `order_hint` LSB **proxy** for the decoded OrderHint (a named residual blocked on reference-state modelling — AV2-5.18.2-FRAME-HEADER-INFO); within one CELU all output units share one active header (one OrderHintBits), so the LSB comparison is a sound under-approximation (no cross-width gate needed, unlike the cross-CELU §7.3.7 form). The mismatch is recorded between two KNOWN output-unit OrderHints and **fires regardless of another output unit in the CELU whose OrderHint is undecidable** (round-7 F3): an undecidable output unit can only prevent proving AGREEMENT (never reported) and cannot make a pair already proven to differ conforming |

### `content-interpretation/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `content-interpretation/aspect-ratio-idc-out-of-range` | error | § 6.14 | ci_aspect_ratio_idc exceeds 16 when not equal to 255 |
| `content-interpretation/chroma-sample-position-out-of-range` | error | § 6.14 | ci_chroma_sample_position top or bottom exceeds 5 |
| `content-interpretation/repeated-ci-not-identical` | error | § 6.14 | repeated CI OBU for same xlayer/mlayer in CVS carries different information |
| `content-interpretation/reserved-bits-nonzero` | warning | § 6.14 | ci_reserved_2bit is non-zero (decoder-ignored producer anomaly) |

### `cmvs/`

Coded-multistream-video-sequence (§7.3.2) boundary semantics (`AV2-7.3.2-CMVS-BOUNDARIES`).
Decidable-disagreement-only and evaluated at temporal-unit-completion resolution; the §7.3.2
begin/end tracker's Unknown states never fire. Hand-crafted unit vectors only.

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `cmvs/boundary-set-mismatch` | error | § 7.3.2 | a temporal unit begins a new coded video sequence with no OBU_MSDO but with an activated global LCR, so it ends the CMVS under the MSDO-alone boundary rules yet continues it under the MSDO-plus-global-LCR rules; § 7.3.2 requires the two boundary sets to be identical (mirror line 351) |

### `decoder-model/`

No AVM differential oracle exists for the `decoder-model/` rules: AVM parses both
decoder-model syntax sites but never enforces or consumes the signaled buffer-delay
values (its only consumer hardcodes 70000/20000), so proof for these rules is
hand-crafted unit vectors only — `avm_diff` is never claimed for them.

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `decoder-model/buffer-delay-sum-changed` | error | § 6.10.5 | the same (obu_xlayer_id, ops_id, operating-point index) is redefined within one coded video sequence with no intervening OPS reset, both signalings explicitly carry decoder-model info, and the ops_decoder_buffer_delay + ops_encoder_buffer_delay sum differs (non-conforming under § 6.4.13 / § 6.10.5 sum-constancy on every candidate "video sequence" reading) |
| `decoder-model/buffer-delay-sum-changed-across-cvs` | warning | § 6.4.13 / § 6.10.5 | an explicitly signaled buffer-delay sum changes across a coded-video-sequence or OPS-reset boundary — the activated sequence header's seq_decoder_model_info() sum across a CLK boundary (frame-confirmed activations only; emitted with spec_section § 6.4.13) or an OPS sum across a CVS/reset boundary for the same triple (emitted with spec_section § 6.10.5); advisory because the § 6.4.13 / § 6.10.5 "video sequence" scope is unspecified |

### `film-grain/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `film-grain/chroma-idc-out-of-range` | error | § 6.13 | fgm_chroma_idc exceeds 3 |
| `film-grain/chroma-points-not-paired` | error | § 6.17.10.2 | in 4:2:0, num_cb_points and num_cr_points are not both zero or both non-zero |
| `film-grain/duplicate-slot-in-coded-frame-unit` | error | § 6.13 | a film grain slot is updated more than once in the same coded frame unit |
| `film-grain/scaling-point-not-increasing` | error | § 6.17.10.2 | a scaling point value is not strictly increasing or not less than 256 |
| `film-grain/scaling-points-out-of-range` | error | § 6.17.10.2 | num_y/cb/cr_points exceeds 14 |
| `film-grain/update-flags-zero` | error | § 6.13 | fgm_update_flags is 0 |

### `frame-header/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `frame-header/bridge-ref-index-out-of-range` | error | § 6.17.2 | bridge_frame_ref_idx is not less than NumRefFrames |
| `frame-header/ccso-ext-filter-reserved` | error | § 6.17.7.8 | a parsed `ccso_params()` plane in the `!ccso_bo_only` arm has `ccso_ext_filter == 7`, the reserved value §6.17.7.8 (mirror :5819) forbids |
| `frame-header/ccso-max-band-out-of-range` | error | § 6.17.7.8 | a parsed `ccso_params()` plane has `1 << ccso_max_band_log2 > CCSO_BAND_NUM` (64), violating §6.17.7.8 (mirror :5824). Only reachable in the `ccso_bo_only` arm where `ccso_max_band_log2` is `f(3)` |
| `frame-header/context-update-tile-id-out-of-range` | error | § 6.17.7.2 | context_update_tile_id is not less than TileCols * TileRows |
| `frame-header/copy-bits-mismatch` | error | § 6.17.1 | a non-first tile group's `frame_header_copy()` (`frame_header( isFirst == 0 )`, §5.18.1) differs from the coded frame's first tile group's frame header: `header_bit[i]` is not equal to the value of the bit at offset `i` from the start of the first `frame_header()` (mirror :4296-4300), violating the §6.17.1 requirement of bitstream conformance that the copy be bit-identical. Fired ONLY when the first tile group's `frame_header_info()` parsed to completion (`FrameHeaderParseStatus::IntraHeaderComplete`) so `NumFrameHeaderBits` and the exact first-header bits are known, and the non-first tile group is a decidable continuation of the SAME coded frame (the segmenter reports `FrameBoundary::ContinuesUnit`). The diagnostic is anchored at the precise location of the first differing `header_bit`: the copy region start (after the two `tile_group_obu()` prefix bits) translated through `mismatch_bit` into the OBU-payload byte offset (`byte_offset`) and the MSB-first bit-within-byte (`bit_offset`) of the offending bit, with the copy-region bit index `header_bit[i]` in the message. (Before this it anchored at the OBU header offset with no bit offset.) An incomplete/coverage-stopped/unresolvable first header records nothing (the copy region stays unparsed); a `FrameBoundary::Ambiguous` boundary (unreadable `is_first_tile_group` delimiter) drops the judgment; and a SEF/TIP/bridge frame opening a new coded frame in the same `(xlayer, mlayer, tlayer)` triple clears the recorded header so a later flag-0 tile group the segmenter routes as continuing that SEF coded frame does not pair against the stale record. The recorded bits exclude the `tile_group_obu()` `is_first_tile_group` flag before `frame_header()` (mirror :4303-4305), so the comparison is bit-aligned not byte-aligned |
| `frame-header/copy-bits-truncated` | error | § 6.2.1 | a non-first tile group's `frame_header_copy()` region ends before all `NumFrameHeaderBits` copied `header_bit` `f(1)` reads (§5.18.1, mirror :3973-3981) — the OBU payload is shorter than the recorded first header, and `frame_header( )` must read those elements from the §6.2.1 OBU payload (the payload lies before the trailing bits). Fired ONLY when every copy bit available before the EOF matched the first header (a differing bit within the available prefix is the decidable `frame-header/copy-bits-mismatch` instead) and the first header completed (`IntraHeaderComplete`, so `NumFrameHeaderBits` is known). Anchored at the offending non-first tile-group OBU. Same gating/Unknown-routing as `frame-header/copy-bits-mismatch` |
| `frame-header/cur-mfh-id-out-of-range` | error | § 6.17 | cur_mfh_id is not less than MAX_MFH_NUM |
| `frame-header/film-grain-model-unavailable` | error | § 6.17.10.1 | a parsed `film_grain_config()` has `apply_grain == 1` and references an `fgm_id` whose slot has no received in-band film-grain model (`FilmGrainPresent[fgm_id] != 1`), violating §6.17.10.1 / §7.3.8.8. Fires ONLY under `ExternalHlsMode::Disabled` — `ExternalHlsSet` cannot express film-grain OBUs, so any `Provided` mode means the model MAY be supplied externally and the check suppresses (inexpressible-kind blanket policy). The §7.3.8.1 random-access-point-unavailability direction is under-reported (monotonic `available[]` never resets at a RAP; awaits a film-grain RAP-replay key, AV2-7.3.8-HLS-AVAILABILITY); the §6.17.10.1 FgmTLayerId/FgmMLayerId/FgmChromaIdc layer-dependency constraints remain a residual |
| `frame-header/frame-size-exceeds-sequence-max` | error | § 6.17.4.1 | derived FrameWidth/FrameHeight exceeds active sequence maximum (the explicit-override `frame_size()` path). The no-double-fire deferral keys on the parsed PATH, not on dimension equality: only the `cur_mfh_id > 0` non-override default path (`frame_size_override_flag == 0`, derived dims taken from the MFH's stored dims) defers to `frame-header/mfh-frame-size-exceeds-sequence-max`. An override==1 frame that explicitly codes the same out-of-range dims the MFH stores is a separate §6.17.4.1 violation of its own `frame_width/height_minus_1` fields, so both rules fire |
| `frame-header/frame-to-refresh-out-of-range` | error | § 6.17.2 | refresh_frame_flags sets a reference slot at or beyond NumRefFrames |
| `frame-header/intra-only-refresh-all-slots` | error | § 6.17.2 | INTRA_ONLY_FRAME with NumRefFrames>1 refreshes every slot |
| `frame-header/mfh-frame-size-exceeds-sequence-max` | error | § 6.17.2 | a `cur_mfh_id > 0` frame references a multi-frame header whose STORED mfh_frame_width/height_minus_1 exceeds max_frame_width/height_minus_1 (mirror :4348-4349). Evaluated after load_sequence_header for every referencing frame from the resolved MFH record and the sequence maxima alone, INDEPENDENT of frame_size_override_flag (an in-range override hides an out-of-range MFH) AND of the frame-header core parse outcome — a truncated/malformed frame-header remainder still fires it — so distinct from the derived-FrameWidth `frame-header/frame-size-exceeds-sequence-max`. An MFH with no mfh_frame_size payload infers its dims to the maxima (§ 5.18.2, mirror :4101) and stays silent; an unresolvable MFH stays silent. On the override==0 default path this rule is the single home and the §6.17.4.1 derived check defers by parsed path (not by value equality) |
| `frame-header/mfh-mlayer-dependency-missing` | error | § 7.3.8.7 | frame header references an MFH whose recorded MfhMLayerId the frame's obu_mlayer_id does not depend on (§ 6.17.2: MLayerDependencyMap[obu_mlayer_id][MfhMLayerId] != 1) |
| `frame-header/mfh-tlayer-dependency-missing` | error | § 7.3.8.7 | frame header references an MFH whose recorded MfhTLayerId the frame's layer does not depend on (§ 6.17.2: TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][MfhTLayerId] != 1) |
| `frame-header/num-total-refs-out-of-range` | error | § 6.17.2 | an inter frame's parsed `num_total_refs` (explicit reference map, §5.18.2 mirror :4601) exceeds `ActiveNumRefFrames = Min(REFS_PER_FRAME, NumRefFrames)` (§5.18.2 mirror :963; REFS_PER_FRAME == 7 per §3), violating the §6.17.2 requirement of bitstream conformance that `num_total_refs <= ActiveNumRefFrames` (mirror :4578-4579). Decidable from the recorded value and the active sequence's NumRefFrames alone; `num_total_refs` is read as `f(3)` so the parse stays safe (the `ref_frame_idx` loop runs at most 7 times). Only the explicit-map arm records `num_total_refs`; the implicit `get_ref_frames()` map (unmodeled) records nothing and stays silent |
| `frame-header/primary-ref-frame-out-of-range` | error | § 6.17.2 | an inter frame signals `primary_ref_frame` (`signal_primary_ref_frame == 1`, `f(3)`, §5.18.2 mirror :4391-4399) that is neither `PRIMARY_REF_NONE` (7) nor less than `NumTotalRefs`, violating the §6.17.2 requirement of bitstream conformance that "when primary_ref_frame is present in the bitstream primary_ref_frame is either equal to PRIMARY_REF_NONE, or primary_ref_frame is less than NumTotalRefs" (mirror :4500-4502). Decidable from the two recorded scalars alone — no reference-frame buffer state. Only the signaled case is checked; an inferred `primary_ref_frame` (PRIMARY_REF_NONE on the switch/bridge/intra arms, or PRIMARY_REF_CHOOSE when `signal == 0`) satisfies the constraint trivially. The bound needs `NumTotalRefs`, which only the explicit-reference-map arm records; the implicit `get_ref_frames()` map (unmodeled) records `num_total_refs == None` and stays silent (under-reported) |
| `frame-header/bru-ref-out-of-range` | error | § 6.17.2 | an inter frame with `use_bru == 1` codes `bru_ref` at or beyond `NumTotalRefs`, violating the §6.17.2 requirement of bitstream conformance that "bru_ref is less than NumTotalRefs" (mirror :4592). Decidable from the two recorded scalars; the implicit `get_ref_frames()` map records `num_total_refs == None` and stays silent (under-reported) |
| `frame-header/bru-without-immediate-output` | error | § 6.17.2 | an inter frame with `use_bru == 1` codes `immediate_output_frame == 0`, violating the §6.17.2 requirement of bitstream conformance that "immediate_output_frame is equal to 1" when `use_bru == 1` (mirror :4591). The remaining §6.17.2 BRU clauses (the `RefOrderHint`/`RESTRICTED_OH`, reference-dims, and refresh-bit conditions, mirror :4589-4596) need reference-state facts and stay named residuals |
| `frame-header/qm-level-unavailable` | error | § 7.3.8.9 | a parsed `setup_qm_params()` has `using_qmatrix == 1` and references a custom quantizer-matrix level (`< NUM_CUSTOM_QMS`) whose availability state has no record — no QM OBU made the level available, or a `reset_qm()` cleared it and it was not re-sent in the current temporal unit (mirror :847-858). The availability state honors the QmProtected discipline: a temporal delimiter clears `QmProtected` for every level (§5.5 mirror :1626-1630); a QM OBU sets `QmProtected` for the levels it sends (§5.13 mirror :3010/:3033); `reset_qm()` clears the availability of UNPROTECTED levels at a CLK / OLK (`keyFrame && FirstPictureInTU`, §5.18.2 mirror :4106-4108), or a RAS / restricted SWITCH (`restricted_prediction_switch == 1`, mirror :4278-4286). The CLK/OLK reset is the fully-decidable `needsReset = 1` arm (every unprotected level cleared); the SWITCH/RAS reset models only the `QmMLayerId == -1` arm of `needsReset` (the `MLayerPresenceMap[QmMLayerId][obu_mlayer_id]` arm is a named residual — clearing on it would risk false unavailability). The SWITCH/RAS `reset_qm()` call sits at a §5.18.2 parse point (mirror :4283) AFTER `restricted_prediction_switch` (and, for RAS, `num_key_ref_frames` / `ref_long_term_id`), so the reset is applied only when the frame's core parse CONFIRMS it reached that point — a resolvable RAS core, or a SWITCH core with `restricted_prediction_switch == 1`. When the reset is UNCONFIRMED (a truncated header that never reached the call site, an unresolvable core, or a SWITCH whose gate bit was never read) the level's availability is POISONED (unknown) rather than cleared: the check DROPS its judgment for that level (no false fire, no stale "available") until the level is re-grounded by a QM OBU re-sending it, or by a later confirmed reset (which grounds it definitively unavailable). The poison persists across temporal delimiters (it is HLS availability state, not the §6.12 coded-frame window). Fires ONLY under `ExternalHlsMode::Disabled` — `ExternalHlsSet` cannot express QM OBUs, so any `Provided` mode means the levels MAY be supplied externally and the check suppresses (inexpressible-kind blanket policy, matching `frame-header/film-grain-model-unavailable`). The §7.3.8.1 random-access-point-visibility direction is under-reported (the linear availability test); the §6.17.6.2 QM layer-dependency constraints remain a separate residual |
| `frame-header/qm-plane-count-mismatch` | error | § 6.17.6.2 | a qm_y/qm_u/qm_v custom-QM reference whose recorded QmNumPlanes differs from the sequence NumPlanes |
| `frame-header/ras-ref-long-term-id-not-in-use` | error | § 6.17.2 | an OBU_RAS_FRAME's explicit-map `ref_frame_idx[i]` selects a §7.23 reference slot the modeled buffer PROVES valid but whose modeled `RefLongTermId` is not in the RAS frame's own `ref_long_term_id` list, violating §6.17.2 `long_term_id_in_use(RefLongTermId[ ref_frame_idx[i] ]) == 1` (mirror :4615-4616). A RAS may reference only the long-term frames it lists (§7.4.5). The per-slot `RefLongTermId` is tracked in the §7.23 reference-state model (`reference_state.rs`): a KEY frame's `long_term_id_plus_1 - 1` (§5.18.2 mirror :4231-4239) becomes the slot's `RefLongTermId`; every other refresh sets `-1`. A slot the buffer cannot PROVE valid (Unknown / ProvenInvalid) drops to silence (the Unknown invariant); a proven-valid `-1` (non-long-term) slot is never in-use and fires. REACHABILITY RESIDUAL: the rule fires only for a `max_mlayer_id != 0` RAS (whose `refresh_frame_flags` are read explicitly so the inter parse reaches `ref_frame_idx`); for `max_mlayer_id == 0` the RAS refresh derivation needs reference state and the parse stops before `ref_frame_idx`, so `ref_frame_idx` is empty and the check is an honest under-report (never a false positive) |
| `frame-header/ras-requires-long-term-frame-id-bits` | error | § 6.4.6 | OBU_RAS_FRAME present but active sequence long_term_frame_id_bits == 0 |
| `frame-header/ref-frame-idx-invalid-slot` | error | § 6.17.2 | an inter frame's `ref_frame_idx[i]` (explicit reference map, §5.18.2 mirror :4611-4625) names a reference slot the modeled §7.23 reference state proves invalid (`RefValid[idx] == 0` against a modeled per-extended-layer buffer), or an index outside the valid slot range. §6.17.2 (mirror :4605-4606) requires `RefValid[ ref_frame_idx[i] ] == 1`, and RefValid is defined only over the ACTIVE slots `0..NumRefFrames-1` of the NUM_REF_FRAMES (16) buffer; the read width `CeilLog2(NumRefFrames)` over-covers a non-power-of-two NumRefFrames (e.g. 6 -> 3 bits -> values up to 7), so an index `>= min(NumRefFrames, NUM_REF_FRAMES)` is an unconditional violation decidable from the parsed value alone (the active-bound check folds the active-`NumRefFrames` arm and the physical-`NUM_REF_FRAMES` arm into this one home, no double-fire). Slots the model has not grounded stay Unknown and are not reported (no guessing); the under-reported direction is the implicit reference map (`get_ref_frames()`), which needs unmodeled derivation |
| `frame-header/ref-long-term-id-reserved` | error | § 6.17.2 | a ref_long_term_id[i] equals the reserved (1<<long_term_frame_id_bits)-1 |
| `frame-header/refresh-frame-flags-zero-on-deferred-output` | error | § 6.17.2 | immediate_output_frame==0 with refresh_frame_flags==0 |
| `frame-header/sef-trailing-bits-invalid` | error | § 6.2.3 | a show-existing-frame OBU's §5.2.3 `trailing_bits( remainingPayloadBits )` is malformed. A SEF OBU is not an `is_tile_group()` type, so `usedArith == 0` and the rest of the payload after the SEF `frame_header()` (which `return`s right after `film_grain_config()`, §5.18.2 mirror :4145/:4186) is exactly `trailing_bits()` — no tile data — so the boundary is decidable from the payload alone. Catches the missing `trailing_one_bit`, a stray set zero-padding bit, and the grain_seed-eats-the-marker case (a `grain_seed` short by its final bit consumes what should have been the `trailing_one_bit`). The parsed SEF facts are preserved (the core classifies the tail without failing) |
| `frame-header/seq-header-id-out-of-range` | error | § 6.17 | seq_header_id_in_frame_header is not less than MAX_SEQ_NUM |
| `frame-header/show-existing-frame-invalid-slot` | error | § 6.17.2 | a show-existing-frame OBU's `frame_to_show_map_idx` references a reference slot the modeled §7.23 reference-frame buffer state PROVES invalid (`RefValid[ frame_to_show_map_idx ] == 0`): the slot was cleared by a CLK reset (§5.18.2 mirror :4449-4455 sets `RefValid[i] = 0` over `0..NumRefFrames`) and not re-validated by any later parsed refresh. A show-existing-frame outputs the frame stored at that slot, which must be valid (§6.17.2 / §7.23). Validator-derived state: the per-extended-layer buffer is updated at each completed frame's coded-frame boundary from the parsed `refresh_frame_flags` / `OrderHint` / dimensions (with the §7.23 key/switch `first` RefValid rule), with honest all-slot poisoning whenever the refresh mask is unparsed (inter / TIP / bridge / truncated / ambiguous-boundary frames) and a grounded CLK reset at a new coded video sequence. Fires ONLY on a PROVEN-invalid slot; a poisoned (Unknown) slot — an unestablished mid-stream-join buffer, or a slot a later unparsed mask may have touched — drops to silence (the Unknown invariant). Only fires when the frame's core resolved against its active sequence header (so the parsed `frame_to_show_map_idx` is trustworthy). No external-HLS suppression: reference-frame buffers are written only in-band by the §7.23 process, so the modeled state is always validator-derived |
| `frame-header/still-picture-requires-key-frame` | error | § 6.17.2 | still_picture sequence without KEY_FRAME and immediate_output_frame==1 |
| `frame-header/switch-or-ras-mlayer-dependency-not-self-contained` | error | § 6.4.1 | OBU_SWITCH / OBU_RAS_FRAME has MLayerDependencyMap[obu_mlayer_id][m] != 0 for some embedded layer m != obu_mlayer_id |
| `frame-header/tile-cols-out-of-range` | error | § 6.17.7.2 | frame tile_info() derives TileCols greater than MAX_TILE_COLS |
| `frame-header/tile-rows-out-of-range` | error | § 6.17.7.2 | frame tile_info() derives TileRows greater than MAX_TILE_ROWS |
| `frame-header/truncated-frame-header` | error | § 6.2.1 | the OBU payload ends inside the frame header before mandatory `frame_header_info()` syntax (§5.18.2) could be read — `frame_header( )` reads those elements from the §6.2.1 OBU payload, which must contain them (the payload lies before the first trailing bit; the §6.2.1 NOTE makes consuming bits within the trailing bits a detectable error). Fired ONLY on an EOF in a FULLY-MODELED region (the core parse reports `StoppedInsideFilterParams` / `StoppedInsideIntraTail` / `StoppedInsideShowExistingFrame` / `StoppedInsideInterControl`, i.e. `is_truncated_in_modeled_region()`); the already-parsed facts are preserved and still feed every other frame-header diagnostic. The inter / switch / TIP / bridge case (`StoppedInsideInterControl`) is the §5.18.2 control region — the region IS fully modeled up to its coverage stops (`InterStop`), so the parser returns `Ok` only at a coverage stop or `Err(UnexpectedEof)` while reading a mandated control field; only the EOF case is a truncation. Unsupported-coverage stops (`StoppedBeforeWienerNsFilter`, `UnsupportedUntilFeature` — a clean inter coverage stop, the MFH-unresolvable stops, `CoreFieldsOnly`) and complete headers stay silent — an early stop where the following syntax is unmodeled is not evidence of truncation |

### `frame-unit/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `frame-unit/buffer-removal-timing-multiplicity` | error | § 7.3.4 | a coded non-output frame unit carries more than one OBU_BUFFER_REMOVAL_TIMING (an output frame unit permits more) |
| `frame-unit/ci-not-in-first-frame-unit` | error | § 7.3.8.10 | OBU_CONTENT_INTERPRETATION appears outside the first coded frame unit of its embedded layer in the temporal unit |
| `frame-unit/duplicate-content-interpretation` | error | § 7.3.3 | a coded frame unit carries more than one OBU_CONTENT_INTERPRETATION |
| `frame-unit/first-tile-group-flag` | error | § 7.3.3 | the first tile OBU of a coded frame has is_first_tile_group != 1 (a later tile OBU re-asserting is_first_tile_group == 1 starts the next coded frame unit, § 7.3.6) |
| `frame-unit/missing-coded-frame` | error/warning | § 7.3.3 | a coded frame unit's head OBUs (CI / MFH / pre-frame) are not followed by a coded frame before the temporal unit (error) or bitstream (warning, possible truncation) ends |
| `frame-unit/mixed-coded-frame-types` | error | § 7.3.3 | the OBUs of one coded frame do not all share a single obu_type |
| `frame-unit/region-order` | error | § 7.3.3 | a content-interpretation or multi-frame-header OBU appears after a later region of its coded frame unit |
| `frame-unit/sef-single-obu` | error | § 7.3.3 | a SEF coded frame is not exactly one OBU (a frame OBU follows the SEF in the same coded frame unit) |
| `frame-unit/suffix-metadata-before-coded-frame` | error | § 7.3.3 | suffix metadata (metadata_is_suffix == 1) appears before the coded frame of its coded frame unit |

### `hls/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `hls/external-hls-disabled` | warning | § 7.3.8.1 | a referenced sequence header is unavailable in-band and external HLS is disabled (advisory) |
| `hls/multiple-active-sequence-headers` | error | § 7.3.6 | a frame-confirmed activation of a different seq_header_id follows an earlier frame-confirmed activation within the same coded video sequence (no intervening CLK) |
| `hls/repeated-sequence-header-not-identical` | error | § 7.3.6 | activated sequence header is repeated within CVS with different payload bytes |
| `hls/unavailable-at-random-access-point` | error | § 7.3.8.1 | a linearly-available HLS OBU (sequence header, multi-frame header, operating point set, layer configuration record, or local atlas segment) referenced at or after that extended layer's § 7.4.1 random access point (per-extended-layer; § 7.4.6) was not (re)sent in or after that point's temporal unit, so it is unavailable on real random access (replay; under external-HLS Provided: expressible kinds — sequence headers, operating point sets — suppressed only when the exact key is declared, inexpressible kinds blanket-suppressed) |
| `hls/unavailable-layer-configuration-record` | error | § 7.3.8.3 | seq_lcr_id resolves to no available local or global LCR (external disabled) |
| `hls/unavailable-multi-frame-header` | error | § 7.3.8.7 | frame header references a cur_mfh_id with no available multi-frame header (external HLS disabled) |
| `hls/unavailable-sequence-header` | error | § 7.3.8.6 | frame header references a sequence header id that is unavailable |

### `ivf/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `ivf/invalid-header-length` | error | IVF | IVF header length is smaller than the 32-byte baseline header |
| `ivf/invalid-signature` | error | IVF | container signature is not `DKIF` when parsing as IVF |
| `ivf/truncated-frame-header` | error | IVF | input ends before a complete 12-byte IVF frame header |
| `ivf/truncated-frame-payload` | error | IVF | input ends before the declared IVF frame payload is complete |
| `ivf/truncated-header` | error | IVF | input ends before the declared IVF header is complete |

### `lcr/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `lcr/dependent-xlayers-flag-nonzero` | warning | § 6.8.2 | lcr_dependent_xlayers_flag is set (decoder-ignored) |
| `lcr/doh-constraint-required` | error | § 6.8.2 | inside a CMVS, a frame-confirmed activated sequence header has monotonic_output_order_flag == 0 while the activated global LCR's lcr_doh_constraint_flag == 0 (mirror lines 1619-1621) |
| `lcr/global-id-out-of-range` | error | § 6.8.2 | lcr_global_config_record_id is 0 (must be 1..7) |
| `lcr/global-lcr-unavailable` | error | § 7.3.8.3 | local LCR references an unavailable global LCR (external HLS disabled) |
| `lcr/global-xlayer-map-missing-xlayer` | error | § 6.4.1 | sequence header xlayer is not set in the referenced global LCR lcr_xlayer_map |
| `lcr/local-id-zero` | error | § 6.8.3 | lcr_local_id equals 0 |
| `lcr/max-expected-dims-exceed-sequence-max` | error | § 6.8.9 | an activated LCR's `lcr_max_expected_width[..][j]` / `lcr_max_expected_height[..][j]` (per embedded layer j, present when `lcr_same_sh_max_resolution_flag == 0`) exceeds the activated sequence header's `max_frame_width/height_minus_1 + 1` (mirror :2135-2148). The pure-arithmetic clause of §6.8.9, decidable at activation from the snapshotted association + active-sequence maxima alone. Gated like the §6.8.8 rep-info agreement: fires only under `ExternalHlsMode::Disabled` (an unmodeled external local LCR could shadow the in-band association) and only on a strict frame-confirmed activation; anchored at the defining LCR OBU. The companion per-frame `FrameWidth/FrameHeight <= lcr_max_expected_*` clause (mirror :2137-2139 / :2144-2146) is a named residual — it needs each frame's `(obu_xlayer_id, obu_mlayer_id) -> (xId, j)` mapping joined against the activated LCR |
| `lcr/mlayer-dependency-missing` | error | § 6.8.9 | activated LCR lcr_mlayer_map includes an embedded layer without a layer the activated sequence header's MLayerDependencyMap requires |
| `lcr/msdo-aggregate-mismatch` | error | § 6.8.2 | with lcr_aggregate_info_present_flag == 1, multistream_profile_idc is inconsistent with lcr_config_idc (Table A.6), its interop point != lcr_max_interop (Table A.1), multistream_level_idx != lcr_aggregate_level_idx, or multistream_tier != lcr_max_tier_flag (mirror lines 1657-1664) |
| `lcr/msdo-doh-flag-mismatch` | error | § 6.8.2 | multistream_doh_constraint_flag != the activated global LCR's lcr_doh_constraint_flag (mirror line 1673) |
| `lcr/msdo-stream-count-mismatch` | error | § 6.8.2 | num_streams_minus_2 + 2 != the activated global LCR's LcrMaxNumXLayerCount (mirror line 1650) |
| `lcr/msdo-sub-xlayer-not-in-lcr` | error | § 6.8.2 | an MSDO sub_xlayer_id[i] is not in the activated global LCR's LcrXLayerID[] (mirror lines 1651-1652) |
| `lcr/msdo-substream-ptl-mismatch` | error | § 6.8.2 | with lcr_seq_profile_tier_level_info_present_flag == 1, sub_stream_max_*[i] != lcr_*[sub_xlayer_id[i]] (exact equality, mirror lines 1666-1671) |
| `lcr/payload-size-overflow` | error | § 6.8.6 | layer config record declared payload size overflows |
| `lcr/ptl-level-exceeds-max` | error | § 6.8.5 | with lcr_seq_profile_tier_level_info(i) present in the activated LCR, an activated sequence header's seq_level_idx exceeds lcr_max_level_idx[i] (mirror lines 1782-1784) |
| `lcr/ptl-mlayer-count-exceeds-max` | error | § 6.8.5 | with lcr_seq_profile_tier_level_info(i) present in the activated LCR, an activated sequence header's seq_max_mlayer_cnt_minus_1 + 1 exceeds lcr_max_mlayer_count[i] (mirror lines 1808-1810) |
| `lcr/ptl-profile-exceeds-max` | error | § 6.8.5 | with lcr_seq_profile_tier_level_info(i) present in the activated LCR, an activated sequence header's seq_profile_idc exceeds lcr_seq_profile_idc[i] (mirror lines 1774-1776) |
| `lcr/ptl-tier-exceeds-max` | error | § 6.8.5 | with lcr_seq_profile_tier_level_info(i) present in the activated LCR, an activated sequence header's seq_tier exceeds lcr_tier_flag[i] (mirror lines 1793-1795) |
| `lcr/rep-info-mismatch` | error | § 6.8.8 | an activated LCR's rep info (lcr_max_pic_width/height, lcr_bit_depth_idc, lcr_chroma_format_idc, or the cropping window flag/offsets) disagrees with the sequence header activated for the same extended layer (mirror lines 1925-1968) |
| `lcr/reserved-bits-nonzero` | warning | § 6.8 | a layer config record reserved-zero field is non-zero (decoder-ignored) |
| `lcr/tlayer-dependency-missing` | error | § 6.8.9 | activated LCR lcr_tlayer_map includes a temporal layer without a layer the activated sequence header's TLayerDependencyMap requires |
| `lcr/xlayer-map-empty` | error | § 6.8.2 | lcr_xlayer_map is 0 (must be 1..(1<<31)-1) |

### `metadata/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `metadata/group-header-underflow` | error | § 6.16.3 | metadata group header underflows the payload |
| `metadata/group-layer-idc-reserved` | warning | § 6.16.3 | group-unit muh_layer_idc is 4..7 (reserved for AOMedia use) |
| `metadata/group-mlayer-map-below-obu-mlayer` | error | § 6.16.3 | muh_mlayer_map sets a bit below obu_mlayer_id |
| `metadata/group-reserved-bits-nonzero` | warning | § 6.16.3 | muh_reserved_zero_2bits is non-zero (decoder-ignored) |
| `metadata/group-unit-count-too-large` | error | § 6.16.3 | metadata group unit count is too large |
| `metadata/group-xlayer-map-global-bit-set` | error | § 6.16.3 | bit 31 of muh_xlayer_map is set |
| `metadata/decoded-frame-hash-reserved-nonzero` | warning | § 6.16.13 | decoded-frame-hash reserved bit is non-zero (decoder-ignored producer anomaly) |
| `metadata/hdr-cll-first-coded-picture` | error | § 6.16.5 | an explicit-pair-targeted HDR CLL metadata unit first establishes content for an embedded layer after that layer's first coded picture of the CVS has passed (it shall be indicated at the first coded picture) |
| `metadata/hdr-cll-repeat-content-differs` | error | § 6.16.5 | HDR CLL metadata units in a CVS associated with a common embedded layer (per § 6.16.3 layer targeting) have different content |
| `metadata/hdr-mdcv-first-coded-picture` | error | § 6.16.6 | an explicit-pair-targeted HDR MDCV metadata unit first establishes content for an embedded layer after that layer's first coded picture of the CVS has passed (it shall be indicated at the first coded picture) |
| `metadata/hdr-mdcv-repeat-content-differs` | error | § 6.16.6 | HDR MDCV metadata units in a CVS associated with a common embedded layer (per § 6.16.3 layer targeting) have different content |
| `metadata/persistence-idc-reserved` | warning | § 6.16.3 | muh_persistence_idc is 4..7 (reserved for AOMedia use) |
| `metadata/scan-type-ci-scan-type-mismatch` | error | § 6.16.10 | mps_pic_struct_type requires a ci_scan_type_idc that differs from a non-zero CI value established in the CVS scope at or after the layer's most recent random access point (§ 7.3.8.11) |
| `metadata/scan-type-ci-scan-type-unestablished` | warning | § 6.16.10 | scan-type metadata present but no CI established a non-zero ci_scan_type_idc in the CVS scope (default is 0, § 7.3.8.11) |
| `metadata/scan-type-equal-picture-interval-required` | error | § 6.16.10 | mps_pic_struct_type 7/8 while CI timing_info established in the current § 7.3.8.11 epoch signals equal_picture_interval 0 |
| `metadata/scan-type-pic-struct-group-inconsistent` | error | § 6.16.10 | mps_pic_struct_type values in the same CVS fall into more than one Table 6.18 group |
| `metadata/scan-type-pic-struct-reserved` | error | § 6.16.10 | mps_pic_struct_type exceeds 12 (reserved) |
| `metadata/short-layer-idc-out-of-range` | error | § 6.16.2 | muh_layer_idc >= 3 for OBU_METADATA_SHORT |
| `metadata/temporal-point-info-not-short` | error | § 6.16.11 | METADATA_TYPE_TEMPORAL_POINT_INFO appears outside OBU_METADATA_SHORT |
| `metadata/timecode-counting-type-reserved` | warning | § 6.16.7 | counting_type is 7..31 (reserved for AOMedia use; decoder-ignored producer anomaly) |
| `metadata/timecode-hours-out-of-range` | error | § 6.16.7 | timecode hours_value exceeds 23 |
| `metadata/timecode-inferred-without-previous` | error | § 6.16.7 | timecode omits seconds_value/minutes_value/hours_value but no previous timecode in the CVS scope (decoding order) carried that field for the inference to draw from |
| `metadata/timecode-minutes-out-of-range` | error | § 6.16.7 | timecode minutes_value exceeds 59 |
| `metadata/timecode-n-frames-exceeds-rate` | error | § 6.16.7 | n_frames is not less than maxPicPerSecond = ceil(time_scale / TicksPerPicture) of the in-scope content-interpretation timing_info() (ci_timing_info_present_flag 1) |
| `metadata/timecode-seconds-out-of-range` | error | § 6.16.7 | timecode seconds_value exceeds 59 |
| `metadata/unit-payload-underflow` | error | § 6.16.1 | metadata unit payload underflows declared size |

### `mfh/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `mfh/id-out-of-range` | error | § 5.7 | mfhId is not less than MAX_MFH_NUM (16) |
| `mfh/seq-header-id-out-of-range` | error | § 6.4.1 | mfh_seq_header_id is not less than MAX_SEQ_NUM (16) |
| `mfh/sequence-header-unavailable` | error | § 7.3.8.6 | multi-frame header references an unavailable mfh_seq_header_id |

### `msdo/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `msdo/doh-constraint-required` | error | § 6.6 | inside a coded multistream video sequence, a frame-confirmed activated sequence header has monotonic_output_order_flag == 0 while the MSDO's multistream_doh_constraint_flag == 0 (mirror lines 1391-1393) |
| `msdo/non-global-layer-id` | error | § 6.6 | OBU_MSDO does not use tlayer==0, mlayer==0, xlayer==GLOBAL_XLAYER_ID |
| `msdo/non-rap-not-identical` | error | § 7.3.8.2 | an OBU_MSDO in a temporal unit that is not a random access point (§ 7.4.1: no CLK/OLK/RAS) differs from the previous OBU_MSDO; resolved at temporal-unit end |
| `msdo/profile-below-substream-max` | error | § 6.6 | multistream_profile_idc < sub_stream_max_profile[i] for some i (mirror line 1347) |
| `msdo/substream-level-exceeds-max` | error | § 6.6 | a frame-confirmed sequence header activated by sub-stream i (mapped via sub_xlayer_id[i]) has seq_level_idx > sub_stream_max_level[i] (mirror lines 1368-1372) |
| `msdo/substream-profile-exceeds-max` | error | § 6.6 | a frame-confirmed sequence header activated by sub-stream i (mapped via sub_xlayer_id[i]) has seq_profile_idc > sub_stream_max_profile[i] (mirror lines 1362-1366) |
| `msdo/substream-tier-exceeds-max` | error | § 6.6 | a frame-confirmed sequence header activated by sub-stream i (mapped via sub_xlayer_id[i]) has seq_tier > sub_stream_max_tier[i] (mirror lines 1374-1378) |
| `msdo/too-many-streams` | error | § 6.6 | num_streams_minus_2 exceeds 2 |

### `obu-header/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `obu-header/base-layer-only-types` | error | § 6.2.2 | a base-layer-only OBU type has non-zero obu_tlayer_id or obu_mlayer_id |
| `obu-header/extension-flag-not-zero` | error | § 6.2.1 | obu_extension_flag is not 0 in this spec version |
| `obu-header/global-xlayer-allowed-types` | error | § 6.2.2 | GLOBAL_XLAYER_ID used by an OBU type that does not permit it |
| `obu-header/global-xlayer-required` | error | § 6.2.2 | OBU type requiring GLOBAL_XLAYER_ID uses a non-global obu_xlayer_id |
| `obu-header/global-xlayer-requires-base-layers` | error | § 6.2.2 | GLOBAL_XLAYER_ID used with non-zero obu_mlayer_id or obu_tlayer_id |
| `obu-header/reserved-obu-type` | info | § 6.2.2 | a reserved obu_type is present (ignored by conformant decoders) |
| `obu-header/temporal-layer-zero-only-types` | error | § 6.2.2 | key/switch/RAS frame type has non-zero obu_tlayer_id |

### `obu-order/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `obu-order/duplicate-temporal-delimiter` | error | § 7.3.7 | a second global temporal delimiter with no intervening OBU |
| `obu-order/global-hls-after-coded-layer` | error | § 7.3.7 | a global HLS prefix OBU appears after a coded extended layer unit |
| `obu-order/global-hls-after-metadata-suffix` | error | § 7.3.7 | a global HLS prefix OBU appears after a global suffix metadata OBU (metadata_is_suffix == 1) |
| `obu-order/non-global-hls-before-coded-layer` | error | § 7.3.6 | a non-global HLS header OBU (LCR / OPS / atlas / sequence header) appears after the coded frame region of its coded extended layer unit has begun |
| `obu-order/padding-non-global-outside-coded-layer` | error | § 7.3.7 | OBU_PADDING outside a coded extended layer unit is not GLOBAL_XLAYER_ID |
| `obu-order/temporal-unit-missing-delimiter` | error | § 7.3.7 | an OBU appears before a global temporal delimiter starts the temporal unit |
| `obu-order/xlayer-order-not-ascending` | error | § 7.3.7 | coded extended layer units are not in ascending obu_xlayer_id order |

### `obu-reserved/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `obu-reserved/all-zero-payload` | error | § 5.3 | reserved OBU has non-empty payload that is entirely zero |

### `ops/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `ops/inherited-op-index-out-of-range` | error | § 6.10.2 | inherited ops_embedded_op_index out of range for referenced OPS |
| `ops/local-reserved-bits-nonzero` | error | § 6.10.2 | local OPS ops_reserved_2bits is non-zero |
| `ops/mlayer-dependency-missing` | error | § 6.10.7 | explicit ops_mlayer_map includes an embedded layer without a layer the activated sequence header's MLayerDependencyMap requires |
| `ops/mlayer-info-idc-reserved` | error | § 6.10.2 | global OPS ops_mlayer_info_idc == 3 (reserved) |
| `ops/payload-size-mismatch` | error | § 6.10.2 | ops_data_size differs from the parsed payload byte count |
| `ops/ptl-reserved-bits-nonzero` | error | § 6.10.4 | ops_ptl_reserved_2bits is non-zero |
| `ops/tlayer-dependency-missing` | error | § 6.10.7 | explicit ops_tlayer_map includes a temporal layer without a layer the activated sequence header's TLayerDependencyMap requires |

### `padding/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `padding/all-zero-payload` | error | § 5.16 | OBU_PADDING payload is entirely zero (no non-zero byte) |
| `padding/invalid-trailing-bits` | error | § 5.16 | padding OBU trailing_bits are invalid |

### `qm/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `qm/duplicate-level-between-frames` | error | § 6.12 | same quantizer matrix level specified twice between coded frames |
| `qm/duplicate-reset-between-frames` | error | § 6.12 | QM OBU with qm_bit_map==0 is not the first QM OBU between coded frames |
| `qm/quant-delta-out-of-range` | error | § 6.4.11 | a quantizer-matrix quant delta value is out of range |

### `sequence-header/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `sequence-header/bit-depth-out-of-range` | error | § 6.4.1 | coded bit depth is out of range |
| `sequence-header/chroma-format-out-of-range` | error | § 6.4.1 | chroma format value out of range |
| `sequence-header/crop-bottom-out-of-range` | error | § 6.4.1 | crop_bottom is out of range |
| `sequence-header/crop-left-out-of-range` | error | § 6.4.1 | crop_left is out of range |
| `sequence-header/crop-right-out-of-range` | error | § 6.4.1 | crop_right is out of range |
| `sequence-header/crop-top-out-of-range` | error | § 6.4.1 | crop_top is out of range |
| `sequence-header/seq-header-id-out-of-range` | error | § 6.4.1 | seq_header_id is out of its valid range |
| `sequence-header/seq-max-mlayer-count-out-of-range` | error | § 6.4.1 | seq_max_mlayer_count is out of range |
| `sequence-header/timing-display-tick-mismatch` | error | § 6.4.12 | num_units_in_display_tick differs across embedded layers in same CVS |
| `sequence-header/timing-display-tick-zero` | error | § 6.4.12 | num_units_in_display_tick is zero |
| `sequence-header/timing-equal-picture-interval-mismatch` | error | § 6.4.12 | equal_picture_interval differs across embedded layers in same CVS |
| `sequence-header/timing-num-ticks-mismatch` | error | § 6.4.12 | num_ticks_per_picture_minus_1 differs across embedded layers in same CVS |
| `sequence-header/timing-num-ticks-per-picture-out-of-range` | error | § 6.4.12 | num_ticks_per_picture_minus_1 is out of range |
| `sequence-header/timing-num-units-zero` | error | § 6.4.1 | timing num_units value is zero |
| `sequence-header/timing-time-scale-mismatch` | error | § 6.4.12 | time_scale differs across embedded layers in same CVS |
| `sequence-header/timing-time-scale-zero` | error | § 6.4.12 | time_scale is zero |

### `sequence-state/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `sequence-state/distinct-mlayer-count-exceeds-seq-max` | error | § 6.4.1 | the distinct obu_mlayer_id count in an extended layer's coded video sequence exceeds the active sequence header's SeqMaxMlayerCnt |
| `sequence-state/mlayer-exceeds-max` | error | § 6.2.2 | obu_mlayer_id exceeds active sequence max_mlayer_id |
| `sequence-state/monotonic-output-order-mismatch` | error | § 6.4.1 | extended layers inside a coded multistream video sequence disagree on monotonic_output_order_flag |
| `sequence-state/no-active-sequence-header` | error | § 7.3.8 | OBU uses an xlayer before an active sequence header is available |
| `sequence-state/tlayer-exceeds-max` | error | § 6.2.2 | obu_tlayer_id exceeds active sequence max_tlayer_id |
| `sequence-state/unknown-sequence-header-id` | error | § 7.3.8 | the active seq_header_id for an xlayer is unavailable |

### `tile-params/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `tile-params/nonuniform-cols-do-not-cover-frame` | error | § 6.17.7.3 | non-uniform tile column widths do not sum to sbCols |
| `tile-params/nonuniform-rows-do-not-cover-frame` | error | § 6.17.7.3 | non-uniform tile row heights do not sum to sbRows |
| `tile-params/tile-cols-out-of-range` | error | § 6.17.7.2 | TileCols exceeds MAX_TILE_COLS |
| `tile-params/tile-rows-out-of-range` | error | § 6.17.7.2 | TileRows exceeds MAX_TILE_ROWS |

### `tile-group/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `tile-group/first-tg-start-not-zero` | error | § 6.18 | the FIRST tile group of a coded frame codes an explicit `tg_start != 0`, but §6.18 (mirror :6215-6216) requires `tg_start` to equal `TileNum` at `tile_group_payload`, and `TileNum == 0` for the first tile group of a regular intra frame (§5.19 mirror :3956). Decidable only for the FIRST tile group of an intra-complete coded frame (`FrameHeaderParseStatus::IntraHeaderComplete`, `frame_is_intra == true`, with a parsed `tile_info()`): on that path `use_bru`/`bru_inactive` derive to the §5.18.2 constant 0 (mirror :4127-4129 / :4653), so `parse_tile_group_structure` reads the §5.19 `tg_start`/`tg_end` exactly. The inferred-range path (`tile_start_and_end_present_flag == 0`) sets `tg_start = 0` by construction and never fires. Cross-tile-group continuity (`tg_start == previous tg_end + 1`) and the last-group `tg_end == NumTiles - 1` are under-reported (need prior-tile-group state) |
| `tile-group/tg-end-before-tg-start` | error | § 6.18 | the tile group codes `tg_end < tg_start`, violating §6.18 (mirror :6220) which requires `tg_end >= tg_start`. Same intra-complete first-tile-group gating as `tile-group/first-tg-start-not-zero` |
| `tile-group/tg-end-out-of-range` | error | § 6.18 | the tile group codes an explicit `tg_end > NumTiles - 1`, but §6.18 (mirror :6218-6223) makes `tg_end` a zero-based tile index whose maximum (the last tile group's `tg_end`) is `NumTiles - 1`. Same gating; the inferred-range path sets `tg_end = NumTiles - 1` and never fires |
| `tile-group/truncated-structure` | error | § 6.2.1 | the OBU payload ends inside the §5.19 `tile_group_obu()` structure (`tile_start_and_end_present_flag` / `tg_start` / `tg_end` / `byte_alignment`) before it could be read; the §6.2.1 OBU payload must contain every mandatory tile-group syntax element. Parallels `frame-header/truncated-frame-header`; the already-parsed structure facts are preserved. Same intra-complete first-tile-group gating |
| `tile-group/byte-alignment-zero-bit` | error | § 6.2.4 | the §5.19 `tile_group_obu()` `byte_alignment()` padding contains a non-zero `zero_bit` (§6.2.4 requires every alignment bit to be 0). Decidable on the same intra-complete first-tile-group path; the §5.20 `tile_group_payload()` bytes after the alignment boundary stay unparsed |

### `trailing-bits/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `trailing-bits/empty` | error | § 6.2.3 | trailing_bits() found empty payload where a trailing one-bit was required |
| `trailing-bits/missing-one-bit` | error | § 6.2.3 | trailing_bits() is missing the required leading 1 bit |
| `trailing-bits/zero-bit-not-zero` | error | § 6.2.3 | a trailing_zero_bit after the one-bit is non-zero |

## Check registry identifiers

These are `Check::id()` registry identifiers, **not** diagnostics emitted verbatim: a failed
parse of the corresponding OBU surfaces a specific `bitstream/parse-error`,
`trailing-bits/*`, or `byte-alignment/*` diagnostic via `syntax_error_diagnostic()` instead.
They are listed here so the registry's documented set equals the rule-id literals present in
the source (the `Parse §` column is the section the OBU's syntax is parsed from).

| Registry ID | Parse § | Routed through |
|---|---|---|
| `atlas/syntax` | § 5.9 | `syntax_error_diagnostic()` |
| `brt/syntax` | § 5.12 | `syntax_error_diagnostic()` |
| `content-interpretation/syntax` | § 5.15 | `syntax_error_diagnostic()` |
| `film-grain/syntax` | § 5.14 | `syntax_error_diagnostic()` |
| `lcr/syntax` | § 5.8 | `syntax_error_diagnostic()` |
| `metadata/syntax` | § 5.17 | `syntax_error_diagnostic()` |
| `mfh/syntax` | § 5.7 | `syntax_error_diagnostic()` |
| `msdo/syntax` | § 6.6 | `syntax_error_diagnostic()` |
| `ops/syntax` | § 5.10 | `syntax_error_diagnostic()` |
| `padding/syntax` | § 5.16 | `syntax_error_diagnostic()` |
| `qm/syntax` | § 5.13 | `syntax_error_diagnostic()` |
| `sequence-header/syntax` | § 5.4 | `syntax_error_diagnostic()` |
| `trailing-bits/empty-syntax-obu-payload` | § 5.2.3 | `syntax_error_diagnostic()` |

<!-- diagnostics-registry:end -->

## Severity guidance

- `error` — a conformance violation that leaves the bitstream parseable (reserved bits, an
  out-of-range field, an unavailable referenced HLS object, an ordering violation).
- `warning` — a decoder-ignored reserved field or a capability-gated condition that is not a
  hard violation (the `*/reserved-bits-nonzero` checks, `hls/external-hls-disabled`).
- `info` — informative only (e.g. a reserved `obu_type` a conformant decoder ignores).

A parse failure — input ending before a required field, a malformed variable-length code, or
a non-zero closing `byte_alignment()` pad bit — is converted into a `bitstream/parse-error`
(or a specific `trailing-bits/*` / `byte-alignment/*`) diagnostic rather than a panic. IVF
container failures use `ivf/*` diagnostics. Malformed payloads and containers are reported
with byte offsets instead of silently accepted.

## Planned / not yet emitted

The following namespaces are reserved for future validator work and are intentionally **absent
from the enforced registry above** because nothing emits them yet:

- `tile-group/` — the §5.19 `tile_group_obu()` STRUCTURE diagnostics (tg-range, byte-alignment,
  truncation) have landed for the intra-complete first tile group (see the registry above). The
  remaining residuals are the §5.20 `tile_group_payload()` tile-data boundary checks (need full
  tile parsing, `AV2-5.20-TILE-GROUP-PAYLOAD`) and the cross-tile-group continuity / last-group
  `tg_end == NumTiles - 1` clauses (need prior-tile-group state threaded through the segmenter).
- `hls-availability/` — a dedicated high-level-syntax availability namespace; today the landed
  availability checks live under `hls/` (see the registry above).
- `obu-payload/` — strict-mode payload constraints.
  (The `decoder-model/` namespace has landed — see the registry tables above — but is limited to
  signaled buffer-delay sum-constancy; Annex E decoder-schedule simulation remains future. The
  `annex-a/` namespace has also landed for the static profile/level/tier subset — see the registry
  tables above — but the rate-based / decoder-model Annex A constraints remain future, tracked by
  the Annex E change.)

Design sketches and phase plans for these live in the planned-diagnostics backlog of
[`VALIDATOR-ROADMAP.md`](./VALIDATOR-ROADMAP.md).
When a planned diagnostic lands, add its rule ID to the enforced tables above (the CI gate
will require it) and update `DIAGNOSTIC_PREFIXES` in `xtask/src/feature_status.rs` if it
introduces a new namespace.

## Intentional non-checks (spec honesty)

Conformance points deliberately not flagged, in two groups.

**Structurally unobservable or not a spec requirement** — these stay non-checks:

- The global atlas (§ 7.3.8.4) is "can be available", so a missing global atlas is not an error.
- § 6.8 / § 6.9 define no "repeated record must be identical" rule, so no LCR/atlas
  duplicate-not-identical diagnostic is emitted (unlike `OBU_MSDO` / sequence headers).
- The § 6.4.11 requirement that no value written into `UserQm` equals 0
  (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md`, "User defined QM semantics") is not
  a diagnostic: the § 5.4.11 parse makes a zero entry unrepresentable — the running quant
  starts at 32, a computed `quant2 == 0` selects the coefficient-repeat path (writing the
  prior non-zero value), and mirror/copy paths replicate already non-zero values — so the
  validator cannot observe a violation.

**Deferred pending infrastructure** — planned in the
[`VALIDATOR-ROADMAP.md`](./VALIDATOR-ROADMAP.md) backlog, not fabricated today:

- The § 6.10.7 / § 6.8.9 / § 7.3.8.7 dependency-map agreement checks (landed as
  `ops/*-dependency-missing`, `lcr/*-dependency-missing`,
  `frame-header/mfh-*-dependency-missing`) run only against a **decidable activated
  in-band** sequence header, and the maps are never fabricated from defaults, max
  layer IDs, or an ambiguous multi-header fallback guess. Each group's
  no-false-positive gate matches what external HLS could shadow: the OPS checks
  (decidable via a parsed frame-header reference or the OBU-order sole-header
  fallback) are suppressed when external HLS declares any sequence header; the LCR
  agreement checks (§ 6.8.5 ceilings, § 6.8.8 rep-info, § 6.8.9 dependency closure)
  require a **strict frame-confirmed** activation — no sole-header fallback, since
  they fire unconditionally on a violation — and are suppressed whenever external HLS
  is enabled at all, because a Provided declaration is partial (it cannot enumerate
  external LCRs) and an unmodeled external *local* LCR would win the local-first
  § 6.4.1 resolution; the MFH checks are skipped when the referenced sequence header
  does not resolve in-band.
  The § 6.8.9 pairing binds the header's § 6.4.1 *association*, snapshotted at each
  observation of that header (an LCR "present prior to this sequence header"): a
  later-arriving LCR is not retroactively paired, and a record redefined after the
  header's latest observation is not the associated one. An OPS/LCR entry whose
  extended layer never activates a decidable in-band header is not checked.
- An unresolved cross-OPS inheritance reference is not flagged (`ops/inherited-ops-unavailable`
  is reserved) because the reference may be supplied through external HLS.

## Testing expectations per diagnostic

Every new diagnostic requires:

1. one positive case that does **not** emit it;
2. one negative case that emits it;
3. a byte offset when available;
4. a spec section in the diagnostic;
5. a proof entry in `docs/IMPLEMENTATION-MATRIX.toml` when the owning feature stage is `done`;
6. a CLI JSON test for at least one diagnostic per new namespace.

## Diagnostic JSON compatibility

Diagnostic JSON is part of the product. Do not rename existing fields without a compatibility
plan; adding fields is acceptable when the CLI tests are updated. `splot validate --json`
prints a report object whose `diagnostics` array holds one object per finding, serialized
from `Diagnostic` in `crates/splot-validate/src/diagnostic.rs`: `severity` is the
capitalized variant name (`"Error"`, `"Warning"`, `"Info"`), and an unset `spec_section`,
`byte_offset`, or `bit_offset` serializes as `null`. One finding looks like:

```json
{
  "rule_id": "sequence-header/chroma-format-out-of-range",
  "spec_section": "6.4.1",
  "severity": "Error",
  "byte_offset": 42,
  "bit_offset": 3,
  "message": "chroma_format_idc must be <= 3, found 4"
}
```

No `feature_id` field is emitted: feature IDs live in code comments, tests, and
`docs/IMPLEMENTATION-MATRIX.toml`, not in the diagnostic payload.
