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
| [VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md) | Registry of every emitted diagnostic rule ID | `cargo xtask check-diagnostic-registry` |
| [VALIDATOR-ROADMAP.md](./VALIDATOR-ROADMAP.md) | Validator phases and the planned-diagnostics backlog | Hand-maintained |

The workflow and ID conventions are in
[FEATURE-TRACKING.md](./FEATURE-TRACKING.md). This file deliberately carries no
status prose — earlier hand-maintained copies drifted and were removed.

## Citation rule

Every syntax-element implementation carries a doc comment (or a
`// TODO(spec: <FEATURE-ID>): …` marker) naming the AV2 section it derives from.
Never invent syntax, constants, or semantics.

## Encoder implementation precondition

Encoder code must not be implemented from rav1e or SVT-AV1 behavior. For every decoder-visible
feature, this mapping must identify:

| Feature | AV2 spec section | AVM oracle | `splot` module | Reference docs consulted | Status |
|---|---|---|---|---|---|

The table is intentionally empty until encoder work begins (`splot-encode` is a
stub); the first encoder feature adds the first row.

If the AV2 section or AVM oracle is unknown, use `TODO(spec: <FEATURE-ID>): <section/topic>` in code
and keep the feature stubbed.

## AV2 is not AV1

The AV2 OBU header is defined in § 5.2.2
([docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2)).
There is **no** `obu_forbidden_bit`, `obu_has_size_field`, temporal/spatial
extension header, or AV1 OBU type table. Do not port AV1 assumptions.
