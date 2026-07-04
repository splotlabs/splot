<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->
<!-- SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com> -->

# AVM decoder fixture / oracle mission log

Mission: build a reusable, AVM-backed AV2 **decoder-output** fixture and
differential-oracle system for `splot`. Source brief:
`splot-decoder-avm-fixture-conformance-mission.md` (local, not committed).

This log records decisions and evidence. Long history lives here, not in source
comments (mission §2, AGENTS.md §11).

## Key decision (2026-07-04): reuse the existing conformance corpus

The mission brief prescribed a brand-new corpus at `tests/fixtures/av2/decoder/`.
The repo already has a mature, self-contained corpus:

- ~68 committed AVM-generated `.ivf` vectors under
  `tests/conformance/vectors/valid/` (project-owned synthetic inputs; no
  third-party media; AVM never vendored/invoked in CI).
- `tests/conformance/manifest.toml` — validator-outcome manifest
  (`CONF-AVM-VALID-STREAMS`).
- `docs/LOCAL-REFERENCE-EVIDENCE.toml` — portable AVM/dav2d raw-output evidence.

Maintainer decision: **reuse & extend** the existing corpus rather than duplicate
it. The decode-output oracle system layers on top of the committed `.ivf`
vectors (referenced by path); it does not re-encode a parallel corpus. Rationale:
the repo enforces a hard anti-duplication budget (`dupehound`) and a
"reuse before reimplement" rule; a duplicate 68-vector corpus would fight both.

Divergences from the brief's literal layout, all deliberate:

- Manifest lives at `tests/conformance/decoder-oracle.toml` (co-located with the
  corpus it references), not `tests/fixtures/av2/decoder/fixtures.toml`.
- Fixtures may exceed the brief's 64×64 "tiny" guideline (the reused corpus has
  up to 192×128); all are ≤ 4 frames and remain small. Recorded per-fixture.
- `.obu` Annex-B twins are deferred (none committed): the reused corpus is IVF,
  and the raw-OBU / Annex-B parse path is already covered by the committed
  `tests/fixtures/*.av2` corpus and the `parse_obu` fuzz target. A twin per `.ivf`
  would double the corpus and bloat the validator manifest for no new coverage.

## Differential basis (de-risking experiment, confirmed)

`splot decode --output-format raw` emits concatenated **visible I420 sample
bytes** per frame (§6.18). Confirmed byte-identical to `avmdec --i420
--rawvideo` and to the committed golden `syn-flat-intra-64x64-minimal.raw`
(all three SHA-256 `92c4477c…`). So the oracle differential is a direct
whole-stream SHA-256 comparison of raw I420 samples, AVM ↔ splot.

- **AVM oracle hash** (committed metadata): `sha256(avmdec --i420 --rawvideo)`.
- **splot side** (run at CI time, no AVM): `sha256(splot decode
  --output-format raw)`.
- `must_pass`: splot raw SHA-256 == recorded AVM raw SHA-256.
- `xfail_splot`: splot exits 1 with `decode/unsupported-feature` + recorded reason.

AVM local revision: `457cd58681a747465661baccb1f32095bc5b7774`
(`v1.0.0-33-g457cd5868`). `avmdec`/`avmenc` built at
`/Users/bartosztomczyk/Devel/avm/build/`.

## Empirical classification (2026-07-04, all 68 valid vectors)

Whole-stream raw SHA-256, splot vs AVM:

- **47 `must_pass`** — byte-identical (intra DC/directional/smooth/rect/grid,
  8-bit + 10-bit, multi-superblock; plus 2–3 frame inter/compound/multiref/
  subpel/CDEF/CCSO/deblock streams).
- **21 `xfail_splot`** — splot fails closed with `decode/unsupported-feature`.
  Reason clusters (feature-unlock backlog, mission §16):
  - `general_intra_transform_tool_residual` ×8
  - `unsupported_cfl_intra` ×5
  - `unsupported_10bit_non_dc_intra` ×2
  - `unsupported_10bit_frozen_minimal_tier` ×1
  - `inter_ccso_reuse_unimplemented` ×1
  - `compound_missing_is_joint_context` ×1
  - `inter_interintra_unimplemented` ×1
  - `multistream_selection` ×1 (OPS)
- **0 mismatches** — splot never emits output that differs from AVM; it matches
  or fails closed. This "match or fail closed" invariant is the harness's core
  value: no fixture can hide a silent wrong-output regression.

## Flight G (feature unlock) — evidence-based tractability

The 21 `xfail_splot` fixtures were instrumented (temporary `route_general_minimal_intra`
+ inner-reason prints, reverted) to establish exactly what each needs. Result:
none is a bounded in-session fix. Both large clusters are deep, actively-staged
decoder frontiers, tracked stage-by-stage under their own matrix rows.

- **CfL cluster (5: `unsupported_cfl_intra`).** All have `route_general_minimal_intra
  = false` with the full AVM default tool set enabled (`enable_ibp/mrls/
  intra_edge_filter/sdp/fsc/cctx/idtx_intra/intra_ist` all on) — the `must_pass`
  general-intra fixtures have every one off. The CfL sequence gate is only the
  *first* gate hit; behind it is the entire unimplemented general-intra tool set.
  Lifting the gate cannot make them decode (they fall to the flat minimal tier).
  Unlock = the full AVM-default intra tool set, not a routing change.

- **Transform-residual cluster — UNLOCKED (chroma non-DCT admit).** All had
  `route_general_minimal_intra = true` and tripped the same inner gate
  `unsupported_dctonly_residual_tx_set` (`general_intra_residual.rs:1325`): a
  chroma (`plane > 0`), non-DCT-forced block with `ActiveChromaResidualPolicy =
  Reject`. Investigation (spec §5.20.7.29 + AVM `av1_get_tx_type`) established
  that the chroma intra transform type is **derived, never coded** — `Mode_To_Txfm`
  of the wide-angle-remapped `UVMode`, with a `get_tx_set` membership check and a
  DCT_DCT fallback. splot already implements that derivation exactly
  (`coeff_loop/ordinary_pass/geometry.rs::mode_to_txfm_plane_tx_type`) and the
  inverse-transform path is plane-generic. So the admit is a **one-line policy
  flip** at `pipeline/general_intra.rs` (`ActiveChromaResidualPolicy::Reject` →
  `LrTxSkipRecordHandoff`, the existing admit variant the inter path already uses),
  not new codec logic. Oracle-verified: **+9 fixtures xfail→must_pass, byte-exact
  to AVM, 0 mismatches, 0 regressions** across deblock/quad/split/grid/shsplit/
  svsplit (8-bit) and one 10-bit split. Spec-generic, not fixture-tuned. The 10th
  (`syn-smooth-intra-64x64-10bit-q80`) now fails closed one stage later at
  `general_intra_rect_non_dc_luma` (a separate 10-bit luma frontier).

- **CfL cluster (5) — NOT unlocked.** See above: full AVM-default tool set; needs
  the whole general-intra tool frontier, not a routing change.

- **Remainder.** `unsupported_10bit_non_dc_intra`, `general_intra_rect_non_dc_luma`
  (10-bit luma), and inter/misc (`compound_missing_is_joint_context`,
  `inter_ccso_reuse_unimplemented`, `inter_interintra_unimplemented`,
  `unsupported_10bit_frozen_minimal_tier`, `multistream_selection`) — each an
  independent frontier.

State after this flight: **56 `must_pass`, 12 `xfail_splot`, 0 mismatches.** The
chroma non-DCT unlock is the payoff of the oracle system: it made the change
measurable and proved it bit-exact and regression-free before landing. No
stream-specific shortcut was taken (AGENTS.md §18) — the admit is the
spec-generic derivation for all chroma intra blocks, verified across 9 diverse
fixtures.

## Flight F (runtime_minimal) status

Already complete: `rg runtime_minimal` over `crates/splot-decode docs tests`
returns only `docs/DECISIONS/decoder-runtime-structure.md` (a decision record).
Production decode modules are already domain-named (bitstream, prediction,
residual, reference, filters, output, pipeline, …). No rename flight needed.
