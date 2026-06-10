# Spec mapping

Normative reference: **AV2 Bitstream & Decoding Process Specification v1.0.0**
(Final Deliverable, 2026-05-28).

## Sources

- **Committed mirror (single source of truth, offline):**
  [`docs/spec/av2/1.0.0/`](./spec/av2/1.0.0/) — a byte-faithful `pdftotext -layout`
  copy of the PDF below, split per chapter with a section index
  ([`index.md`](./spec/av2/1.0.0/index.md)). Cite `§ N.M` + the mirror path
  (e.g. `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-16`). The PDF stays
  normative; the mirror is the citable/greppable copy. Regenerate with
  `scripts/spec/regenerate-av2-spec.sh`; integrity is gated by
  `cargo xtask check-spec-mirror`.
- HTML spec: <https://av2.aomedia.org/v1.0.0/index.html>
- PDF spec (upstream, normative): <https://av2.aomedia.org/v1.0.0/20260528_38f28e7_AV2_Spec_v1.0.0.pdf>
- Syntax browser: <https://av2.aomedia.org/v1.0.0/syntax_browser.html>
- Additional tables: <https://av2.aomedia.org/v1.0.0/attachments/all_tables.h>
- AVM reference software (oracle): <https://github.com/AOMediaCodec/avm/tree/v1.0.0>

## Canonical status

The **canonical** per-feature status lives in
[IMPLEMENTATION-MATRIX.toml](./IMPLEMENTATION-MATRIX.toml) and is rendered to two
generated, drift-gated documents: [SPEC-COVERAGE.md](./SPEC-COVERAGE.md) (one row
per spec section, with parse/validate/test glyphs and mirror links — start there
to answer "is § X.Y implemented?") and [FEATURE-STATUS.md](./FEATURE-STATUS.md)
(the full per-feature ledger). Diagnostics are registered in the CI-enforced
[VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md). The workflow and
conventions are in [FEATURE-TRACKING.md](./FEATURE-TRACKING.md). This file
deliberately carries **no per-module status prose** — earlier hand-maintained
copies drifted and were removed.

## Rule

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

## Validator roadmap

The validator coverage plan is split across:

- [VALIDATOR-ROADMAP.md](./VALIDATOR-ROADMAP.md) — phases, current focus, and the planned-diagnostics backlog
- [VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md) — the CI-enforced registry of every emitted diagnostic

The canonical status remains
[IMPLEMENTATION-MATRIX.toml](./IMPLEMENTATION-MATRIX.toml).

## ⚠️ AV2 is not AV1

The AV2 OBU header (§ 5.2.2) is:

```text
obu_header() {
    obu_header_extension_flag  f(1)
    obu_type                   f(5)
    obu_tlayer_id              f(2)
    if ( obu_header_extension_flag == 1 ) {
        obu_mlayer_id          f(3)
        obu_xlayer_id          f(5)
    } else {
        obu_mlayer_id = 0
        obu_xlayer_id = ( obu_type == OBU_MSDO || obu_type == OBU_TEMPORAL_DELIMITER )
            ? GLOBAL_XLAYER_ID : 0
    }
}
```

There is **no** `obu_forbidden_bit`, `obu_has_size_field`, temporal/spatial
extension header, or AV1 OBU type table. Do not port AV1 assumptions.
