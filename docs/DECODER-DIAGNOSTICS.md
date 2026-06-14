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
`crates/splot-decode/src`. Emitted decoder diagnostics are owned by
`splot-decode` and rendered by the CLI decode command after input bytes reach
the plan-only byte stream planner.
`splot-recon` is shared reconstruction infrastructure for future decoder and
encoder roundtrip work, so it is not scanned as a decoder diagnostic root unless
a future change adds a narrower decoder-owned reconstruction emission path.
Future emissions from scanner roots must be added only in the same change that
adds source emission and tests.

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
| `decode/malformed-source` | Error | optional parser section | `DECODE-BYTE-STREAM-PLANNER` | `decode-byte-stream-planner` | decode source is malformed and could not be planned. | Check the AV2 Annex B or IVF source bytes before retrying `splot decode`. |
| `decode/resource-limit` | Error | optional policy / § 5.2.1 / § 7.1 | `DOC-DECODE-LIMITS-CONTRACT` | `decode-limits-budget` | decode planning stopped because a configured resource limit was exceeded. | Use a smaller input or raise the decode limit policy before retrying. |
| `decode/unsupported-feature` | Error | § 7.1 or planner section | `CLI-DECODE` / `DECODE-STREAM-STATE-PLANNER` | `cli-decode-entrypoint` / `decode-stream-state` | Byte stream planning succeeded, but `splot decode` runtime output is not implemented yet. | Use `splot validate` or `splot inspect` for bitstream analysis until `CLI-DECODE` implements output. |

<!-- diagnostics-registry:end -->

## Detail fields

`decode/malformed-source` includes `detail_kind`, `source_issue_kind`,
`parser_rule_id`, `byte_offset`, `ivf_frame_index`, and `parser_message` when
known. Annex B wrapper errors and IVF container errors leave `spec_section`
unset unless the underlying parser exposes one AV2 section precisely enough to
cite.

`decode/resource-limit` includes `detail_kind`, `limit_name`, `limit`, `actual`,
`unit`, `byte_offset`, and `bit_offset`. Resource limits are `splot` policy over
measured planner values, not AV2 conformance failures; policy-only limits such
as `max_input_bytes` leave `spec_section` unset.

Planner-level `decode/unsupported-feature` includes `detail_kind`,
`unsupported_reason`, `obu_type`, and `byte_offset`.

Runtime-deferral `decode/unsupported-feature` includes `detail_kind`,
`bitstream_format`, `input_len_bytes`, `obu_count`, `frame_candidate_count`,
`source_warning_count`, and selected base-layer ids.
