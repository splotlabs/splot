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

The initial emitted-source root is
`crates/splot-cli/src/commands/decode.rs` because no decoder library crate has
been approved yet. Future emissions from `crates/splot-decode/src` or
`crates/splot-recon/src` must be added only after the corresponding dependency
graph change is explicitly approved.

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
