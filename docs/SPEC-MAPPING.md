# Spec mapping

This document records the AV2 spec sources and citation rules — never
per-feature status, which lives in
[IMPLEMENTATION-MATRIX.toml](./IMPLEMENTATION-MATRIX.toml).

Normative reference: **AV2 Bitstream & Decoding Process Specification v1.0.0**
(Final Deliverable, 2026-05-28).

## Sources

- **Committed mirror (single source of truth, offline):**
  [`docs/spec/av2/1.0.0/`](./spec/av2/1.0.0/) — a byte-faithful `pdftotext -layout`
  copy of the PDF below, split per chapter with a section index
  ([`index.md`](./spec/av2/1.0.0/index.md)). Cite `§ N.M` plus the mirror path
  (e.g. § 5.16 plus `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-16`). The
  PDF stays normative; the mirror is the citable/greppable copy. Regenerate with
  `scripts/spec/regenerate-av2-spec.sh`; integrity is gated by
  `cargo xtask check-spec-mirror`.
- HTML spec: <https://av2.aomedia.org/v1.0.0/index.html>
- PDF spec (upstream, normative): <https://av2.aomedia.org/v1.0.0/20260528_38f28e7_AV2_Spec_v1.0.0.pdf>
- Syntax browser: <https://av2.aomedia.org/v1.0.0/syntax_browser.html>
- Additional tables: <https://av2.aomedia.org/v1.0.0/attachments/all_tables.h>
- AVM reference software (oracle): <https://github.com/AOMediaCodec/avm/tree/v1.0.0>

## Canonical status

| File | Role | Enforcement |
|---|---|---|
| [IMPLEMENTATION-MATRIX.toml](./IMPLEMENTATION-MATRIX.toml) | Canonical per-feature status | `cargo xtask check-feature-status` |
| [SPEC-COVERAGE.md](./SPEC-COVERAGE.md) | Generated per-spec-section view — start there for "is § X.Y implemented?" | Drift-gated by `check-feature-status` |
| [FEATURE-STATUS.md](./FEATURE-STATUS.md) | Generated per-feature ledger | Drift-gated by `check-feature-status` |
| [DECODER-SUPPORT-MATRIX.toml](./DECODER-SUPPORT-MATRIX.toml) | Canonical decoder/reconstruction support status | `cargo xtask check-decoder-support` |
| [DECODER-SUPPORT-STATUS.md](./DECODER-SUPPORT-STATUS.md) | Generated decoder/reconstruction support view | Drift-gated by `check-decoder-support` |
| [VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md) | Registry of every emitted diagnostic rule ID | `cargo xtask check-diagnostic-registry` |
| [DECODER-DIAGNOSTICS.md](./DECODER-DIAGNOSTICS.md) | Registry of every emitted decoder diagnostic rule ID | `cargo xtask check-diagnostic-registry` |
| [VALIDATOR-ROADMAP.md](./VALIDATOR-ROADMAP.md) | Validator phases and the planned-diagnostics backlog | Hand-maintained |
| [DECODER-ROADMAP.md](./DECODER-ROADMAP.md) | Decoder/reconstruction scope and staged tiering | Hand-maintained |

The workflow and ID conventions are in
[FEATURE-TRACKING.md](./FEATURE-TRACKING.md). This file deliberately carries no
status prose — earlier hand-maintained copies drifted and were removed.

## Citation rule

Every syntax-element implementation carries a doc comment (or a
`// TODO(spec: <FEATURE-ID>): …` marker) naming the AV2 section it derives from.
Never invent syntax, constants, or semantics.

## Decoder tile partition surface

Decoder partition work uses these local mirror citations:

- AV2 § 5.20.2.1 decode tile syntax:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1).
- AV2 § 5.20.3.1 decode partition syntax:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-1`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-1).
- AV2 § 5.20.3.2 read partition syntax:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2).
- AV2 § 5.20.4.1 decode block syntax, cited only as a future boundary beyond
  the current partition-frontier feature:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-4-1`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-4-1).
- AV2 § 5.20.9.1 tile-local `is_inside` function:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-9-1`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-9-1).
- AV2 § 5.20.10.4 / § 5.20.10.5 loop-restoration syntax, cited only for
  unsupported gate context because the partition-frontier feature uses a
  precomputed frame fact and does not parse these symbols:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-10-4`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-10-4) and
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-10-5`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-10-5).
- AV2 § 6.19.2.1 tile-level semantics:
  [`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-2-1`](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-2-1).
- AV2 § 6.19.3 partition structure semantics:
  [`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-3`](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-3).
- AV2 § 8.2.5 / § 8.2.6 literal and symbol parsing:
  [`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2-5`](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) and
  [`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2-6`](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6).
- AV2 § 8.3.1 / § 8.3.2 syntax-element CDF selection:
  [`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-1`](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-1) and
  [`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2).
- AV2 § 9.2 / § 9.3 generated conversion and default-CDF tables:
  [`docs/spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2`](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) and
  [`docs/spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3`](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3).

Per-feature support status stays in
[`IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml) and
[`DECODER-SUPPORT-MATRIX.toml`](./DECODER-SUPPORT-MATRIX.toml); this section is
only the citation surface for tile partition syntax, semantics, and parsing
helpers.

## Decoder transform-size surface

Decoder transform-record work uses these local mirror citations:

- AV2 § 5.20.6.1 TX size syntax:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-6-1`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-6-1).
- AV2 § 5.20.6.3 read TX partition syntax:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-6-3`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-6-3).
- AV2 § 5.20.7.24 residual syntax, whose `skip_flag` participates in retained
  `LrTxSkip` derivation:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-24`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-24).
- AV2 § 5.20.7.27 coefficient syntax, which writes the `LrTxSkip` values read
  by loop-restoration classification:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27).
- AV2 § 5.20.7.29 transform-type and secondary-transform syntax, including
  the intra IST `sec_tx_type` branch used by the local decoder mission zero/frontier and
  active LR tx-skip handoff rows:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-29`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-29).
- AV2 § 7.20.4 pixel classified Wiener filter process, including use of
  retained `LrTxSkip` values derived after local decoder mission transform-record syntax
  handoff:
  [`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-20-4`](./spec/av2/1.0.0/07-decoding-process.md#s-7-20-4).
- AV2 § 8.3.2 syntax-element CDF selection for transform and secondary-transform
  symbols:
  [`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2).
- AV2 § 9.3 default CDF tables for transform and secondary-transform rows:
  [`docs/spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3`](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3).

Per-feature support status stays in
[`IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml) and
[`DECODER-SUPPORT-MATRIX.toml`](./DECODER-SUPPORT-MATRIX.toml); this section is
only the citation surface for transform-size syntax and the LR transform-record
handoff.

## Decoder inter prediction surface

Minimal decoder inter/compound work uses these local mirror citations:

- AV2 § 5.20.7.6 inter block mode info syntax:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-6`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-6).
- AV2 § 5.20.7.10 reference-frame syntax and § 5.20.7.11 compound reference
  syntax:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-10`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-10) and
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-11`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-11).
- AV2 § 5.20.7.13 assign-MV syntax, § 5.20.7.16 compound-type syntax, and
  § 5.20.7.17 refine-MV syntax:
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-13`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-13),
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-16`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-16), and
  [`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-17`](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-17).
- AV2 § 7.13.3.16 rounding variable derivation and § 7.13.3.18 block inter
  prediction:
  [`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-3-16`](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-3-16) and
  [`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-3-18`](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-3-18).
- AV2 § 8.3.2 syntax-element CDF selection for inter syntax symbols:
  [`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2).

Per-feature support status stays in
[`IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml) and
[`DECODER-SUPPORT-MATRIX.toml`](./DECODER-SUPPORT-MATRIX.toml); this section is
only the citation surface for inter block syntax, compound prediction, and
motion-compensation helpers.

## Non-normative containers

IVF support (`AV2-IVF-CONTAINER`) is tracked in the implementation matrix because
real AV2 workflows often wrap Annex B payloads in an IVF `DKIF` container. IVF is
not AV2 bitstream syntax and has no AV2 spec section; `splot-core` treats it as a
container envelope, then parses frame payloads through the normal AV2 Annex B path.
Use [Duck IVF](https://wiki.multimedia.cx/index.php/Duck_IVF) only for the generic
container layout (header fields and frame records), not for AV2 semantics.
Decoder byte-stream planning (`DECODE-BYTE-STREAM-PLANNER`) uses the same local
container boundary: raw bytes are interpreted as AV2 Annex B unless they begin
with `DKIF`, in which case IVF frame payloads are traversed as Annex B while IVF
timestamps remain container metadata only.

## Encoder implementation precondition

Encoder code must not be implemented from rav1e or SVT-AV1 behavior. For every decoder-visible
feature, this mapping must identify:

| Feature | AV2 spec section | AVM oracle | `splot` module | Reference docs consulted | Status |
|---|---|---|---|---|---|

The table remains intentionally empty for decoder-visible encoder behavior. The
current encoder input-view and lifecycle work is non-normative API plumbing
tracked in `docs/IMPLEMENTATION-MATRIX.toml` under `ENC-Y4M-INPUT` and
`ENC-CONTEXT-STATE-MACHINE`; the first decoder-visible encoder feature adds the
first row here.

If the AV2 section or AVM oracle is unknown, use `TODO(spec: <FEATURE-ID>): <section/topic>` in code
and keep the feature stubbed.

## AV2 is not AV1

The AV2 OBU header is defined in § 5.2.2
([docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2)).
There is **no** `obu_forbidden_bit`, `obu_has_size_field`, temporal/spatial
extension header, or AV1 OBU type table. Do not port AV1 assumptions.
