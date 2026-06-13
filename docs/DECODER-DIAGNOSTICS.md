# Decoder diagnostics registry

`status: enforced`
`owner: decoder`
`purpose: the canonical, CI-enforced list of every diagnostic rule id emitted by splot decode`

> **Canonical decoder diagnostic registry.** The tables in the marker-delimited
> region below are the single source of truth for emitted decoder diagnostic
> `rule_id` values. `cargo xtask check-diagnostic-registry` compares this
> region with the current decoder emission source roots and fails on drift in
> either direction. Emitted decoder diagnostics currently must use the
> `decode/*` namespace; diagnostic-looking IDs with another namespace in the
> decoder emission roots or in this registry are rejected. The check enforces
> the `rule_id` set only; severity, section, matrix row, feature id, message,
> and remediation are protected by CLI tests, OpenSpec requirements, and review.

The current scanner roots are `crates/splot-cli/src/commands/decode.rs` and
`crates/splot-decode/src`. The only emitted decoder diagnostic today lives in
the CLI decode command; `splot-decode` is scaffolded, but intentionally emits no
decoder diagnostics yet. `splot-recon` is shared reconstruction infrastructure
for future decoder and encoder roundtrip work, so it is not scanned as a decoder
diagnostic root unless a future change adds a narrower decoder-owned
reconstruction emission path. Future emissions from scanner roots must be added
only in the same change that adds source emission and tests.

Every emitted decoder diagnostic uses stable field names:

- `rule_id`;
- `severity`;
- `spec_section`;
- `matrix_row`;
- `feature_id`;
- `message`;
- `remediation`.

<!-- diagnostics-registry:begin -->

## Emitted diagnostics

### `decode/`

| Rule ID | Severity | Section | Feature | Matrix Row | Message | Remediation |
|---|---|---|---|---|---|---|
| `decode/unsupported-feature` | Error | § 7.1 | `CLI-DECODE` | `cli-decode-entrypoint` | `splot decode` is not implemented for AV2 bitstreams yet. | Use `splot validate` or `splot inspect` for bitstream analysis until `CLI-DECODE` is implemented. |

<!-- diagnostics-registry:end -->

## Planned diagnostics

These IDs are not emitted yet and intentionally stay outside the enforced
registry region above. Move a planned ID into the emitted registry only in the
same change that adds source emission and tests.

| Rule ID | Planned Feature | Matrix Row | Purpose |
|---|---|---|---|
| `decode/resource-limit` | `DOC-DECODE-LIMITS-CONTRACT` | `decode-limits-budget` | Future decoder planning diagnostic for inputs whose spec-derived dimensions, tile sizes, frame counts, reference storage, or output sizes exceed caller-configured `DecodeLimits`. |

When emitted, `decode/resource-limit` must include the stable decoder diagnostic
fields plus `limit_name`, `limit`, `actual`, `unit`, `byte_offset`, and
`bit_offset`.
