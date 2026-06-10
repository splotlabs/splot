# Tasks: Matrix validate-stage honesty sweep

Every disposition below is an audit *claim* to re-verify against
`docs/spec/av2/1.0.0/` and `docs/VALIDATOR-DIAGNOSTICS.md` before applying.
A row whose claim does not verify is left untouched and reported.

## 1. Pre-implementation bookkeeping

- [x] 1.1 Register the change in `openspec/changes/README.md`.

## 2. Descriptor / envelope rows

- [x] 2.1 `AV2-4.11.6-LEB128`: claim — the generic bitstream/parse-error
  diagnostic already covers the validate dimension. Verify and either close
  with proof or write the precise residual note.
- [x] 2.2 `AV2-5.2.1-OBU-TYPE`: claim — all §6.2.2 obu-header/* class checks
  landed; residual activated-limit semantics owned by
  `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`. Verify; close or annotate
  ownership.

## 3. Sequence-config children (§5.4.3/.4/.5/.7/.8/.10, §5.4.6)

- [x] 3.1 Claim — the §6.4.2–§6.4.13 mirror text contains no unlanded
  "requirement of bitstream conformance" sentences for these configs; the
  residual is tool-flag gating at frame-header parse (blocked on the
  frame-header backlog items). Grep the mirror per section, verify against
  the registry, then annotate each row's note with the blocker (or close
  where literally nothing remains).
- [x] 3.2 `AV2-5.4.6-SEQUENCE-INTER-CONFIG`: claim — its one local rule
  (§6.4.6 RAS-requires-long_term_frame_id_bits) landed as
  `frame-header/ras-requires-long-term-frame-id-bits`. Verify; close with
  proof if confirmed.
- [x] 3.3 `AV2-5.4-SEQUENCE-HEADER` umbrella: note enumerates that the §6.4
  residuals live on `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`.

## 4. LCR children (§5.8.2/.3/.5/.6/.7/.9)

- [x] 4.1 Claims — §6.8.6 payload-size bound is parse-enforced
  (`lcr/payload-size-overflow`); reserved-bit checks landed
  (`lcr/reserved-bits-nonzero`); the remaining children are syntax containers
  with no per-child §6.8 residual. Verify each against mirror §6.8 and the
  registry; close with proof or annotate the owner row.

## 5. Atlas children (§5.9.1–§5.9.5)

- [x] 5.1 Claim — mode/count/region range checks landed as `atlas/*`;
  residual region-geometry-vs-frame-size is blocked on frame-size state.
  Verify; annotate the blocker (or close where nothing else remains).

## 6. Metadata children

- [x] 6.1 Externally-specified payloads with no §6.16 residual rule:
  `AV2-5.17.4-METADATA-ITUT-T35`, `AV2-5.17.8-METADATA-BANDING-HINTS`,
  `AV2-5.17.9-METADATA-ICC-PROFILE`,
  `AV2-5.17.13-METADATA-USER-DATA-UNREGISTERED`. Verify the mirror sections
  really state no checkable constraint; close with proof or annotate.
- [x] 6.2 `AV2-5.17.11-METADATA-TEMPORAL-POINT-INFO`: claim — the only local
  rule landed (`metadata/temporal-point-info-not-short`);
  `frame_presentation_time` semantics are decoder-model-blocked (Annex E).
  Verify; close/annotate accordingly.
- [x] 6.3 `AV2-5.17.12-METADATA-DECODED-FRAME-HASH`: hash verification is
  decoder-blocked (§6.16.13 needs the §7.21 output process) — document as
  blocked, stage stays `partial`.

## 7. Spec delta and generated artifacts

- [x] 7.1 feature-tracking spec delta: `partial` stages on normative rows
  SHALL name the remaining work or the blocker.
- [x] 7.2 Regenerate `docs/FEATURE-STATUS.md` / `docs/SPEC-COVERAGE.md`;
  re-record the audit ledger.

## 8. Verification

- [x] 8.1 Report: rows closed (with their proof), rows annotated (with their
  blocker), rows left untouched because the audit claim did not verify.
- [x] 8.2 `cargo xtask check-feature-status` passes.
- [x] 8.3 `cargo xtask ci` passes end to end
  (`RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin`).
