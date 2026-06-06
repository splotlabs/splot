# Spec mapping

Normative reference: **AV2 Bitstream & Decoding Process Specification v1.0.0**
(Final Deliverable, 2026-05-28).

## Sources

- HTML spec: <https://av2.aomedia.org/v1.0.0/index.html>
- PDF spec: <https://av2.aomedia.org/v1.0.0/20260528_38f28e7_AV2_Spec_v1.0.0.pdf>
- Syntax browser: <https://av2.aomedia.org/v1.0.0/syntax_browser.html>
- Additional tables: <https://av2.aomedia.org/v1.0.0/attachments/all_tables.h>
- AVM reference software (oracle): <https://github.com/AOMediaCodec/avm/tree/v1.0.0>

## Rule

Every syntax-element implementation carries a doc comment (or `// TODO(spec): …`)
naming the AV2 section it derives from. Never invent syntax, constants, or
semantics.

## Current mapping

| Module                              | Spec area                         | Status |
|-------------------------------------|-----------------------------------|--------|
| `splot-core::leb128`                | § 4.11.6 LEB128                    | implemented |
| `splot-core::bitio`                 | `f(n)` fixed-width reads          | implemented (entropy coder stubbed) |
| `splot-core::types` (`ObuType`)     | Table 6.1, § 5.2.1 helpers        | implemented |
| `splot-core::obu`                   | § 5.2.2 OBU header                 | implemented |
| `splot-core::annexb`                | Annex B § B.2, § 5.2.1 OBU size   | implemented |
| `splot-validate::checks`            | § 6.2.2 header constraints        | partial (header-only checks) |
| `splot-core::headers`               | § 5.4 sequence / frame headers    | TODO |
| `splot-core::tables`                | § 9 additional tables             | TODO / codegen (`cargo xtask gen-tables`) |

### Implemented § 6.2.2 header checks

All are pure functions of the OBU header (no activated sequence header required):

- `obu-header/global-xlayer-required` — `OBU_MSDO` / `OBU_TEMPORAL_DELIMITER` must
  use `obu_xlayer_id == GLOBAL_XLAYER_ID`.
- `obu-header/global-xlayer-requires-base-layers` — global xlayer ⇒ `obu_mlayer_id`
  and `obu_tlayer_id` are `0`.
- `obu-header/global-xlayer-allowed-types` — only certain types may use the global
  xlayer.
- `obu-header/base-layer-only-types` — sequence header, temporal delimiter, LCR,
  OPS, and atlas segment must have `obu_tlayer_id == obu_mlayer_id == 0`.
- `obu-header/temporal-layer-zero-only-types` — closed/open-loop key, switch, and
  RAS frames must have `obu_tlayer_id == 0`.
- `obu-header/reserved-obu-type` — informational: reserved types are ignored by
  conformant decoders.

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
