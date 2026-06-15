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
the byte stream planner and, for hash mode, the minimal runtime tier gate.
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
| `decode/output-error` | Error | none | `DECODE-Y4M-RUNTIME-OUTPUT` | `decode-y4m-runtime-output` | decode output could not be serialized or written. | Check the output destination and retry the decode operation. |
| `decode/resource-limit` | Error | optional policy / § 5.2.1 / § 7.1 | `DOC-DECODE-LIMITS-CONTRACT` | `decode-limits-budget` | decode planning stopped because a configured resource limit was exceeded. | Use a smaller input or raise the decode limit policy before retrying. |
| `decode/unsupported-feature` | Error | § 7.1 or planner / tile section | `CLI-DECODE` / `DECODE-STREAM-STATE-PLANNER` / `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` / `DECODE-TILE-PAYLOAD-BOUNDARY` | `cli-decode-entrypoint` / `decode-stream-state` / `minimal-decode-tier-contract` / `tile-payload-decode` | Byte stream planning succeeded, but `splot decode` runtime output, the requested runtime tier, or tile syntax traversal is not supported. | Use `splot validate` or `splot inspect` for bitstream analysis, or use a stream inside the supported minimal hash tier. |

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

Runtime-tier `decode/unsupported-feature` includes `detail_kind`,
`unsupported_reason`, `tier_id`, and optional `byte_offset`.

`decode/output-error` includes `detail_kind`, `output_operation`,
`output_source_kind`, and `output_source_message` in CLI JSON/text rendering.
The operation is stable; filesystem publication code must not include
nondeterministic temporary filename suffixes in diagnostic details.

Tile-payload-boundary `decode/unsupported-feature` metadata is crate-private
until a later runtime decode path surfaces it through CLI diagnostics. The
boundary records the stable unsupported reason, matrix row
`tile-payload-decode`, Feature ID `DECODE-TILE-PAYLOAD-BOUNDARY`, optional tile
number, and byte offset.
