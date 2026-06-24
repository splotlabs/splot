# AV2 Spec and Diagnostics

Use this file when touching AV2 syntax, semantics, parser behavior, validation,
diagnostics, or spec citations.

## Source of Truth

The committed spec mirror is the local source of truth for AV2 claims:

```text
docs/spec/av2/1.0.0/
```

Find sections through:

```text
docs/spec/av2/1.0.0/index.md
```

Cite AV2 syntax and semantics as `§ N.M` plus the mirror path, for example:

```text
§ 5.16, docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-16
```

The PDF remains normative; the mirror is the byte-faithful navigation and
citation aid. Treat the mirror as read-only third-party material. Regenerate it
with `scripts/spec/regenerate-av2-spec.sh`; do not hand-edit it.

Enforcement:

```bash
cargo xtask check-spec-mirror
```

See [../SPEC-MAPPING.md](../SPEC-MAPPING.md).

## AV2 Is Not AV1

Never invent AV2 syntax, constants, table contents, or semantics.

The AV2 OBU header is from **§ 5.2.2**. Do not copy AV1 OBU fields, the AV1 OBU
type table, `obu_forbidden_bit`, or `obu_has_size_field`.

Treat AVM as the differential-testing oracle:

```text
https://github.com/AOMediaCodec/avm
```

## Spec TODOs

For intentionally unmapped AV2 details, use:

```rust
// TODO(spec: <FEATURE-ID>): short topic
```

The Feature ID must exist in `docs/IMPLEMENTATION-MATRIX.toml`.

Enforcement:

```bash
cargo xtask check-feature-status
```

## Validator Diagnostics

Diagnostics are the product. Every validator finding is structured data:

- stable `rule_id`
- `severity`
- optional `spec_section`
- optional byte or bit offset
- human-readable `message`

The validator never "just logs" a finding.

Diagnostic registries:

- [../VALIDATOR-DIAGNOSTICS.md](../VALIDATOR-DIAGNOSTICS.md)
- [../DECODER-DIAGNOSTICS.md](../DECODER-DIAGNOSTICS.md)

Enforcement:

```bash
cargo xtask check-diagnostic-registry
```
