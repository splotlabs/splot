# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::NonCanonicalOperatingPointSet { what: &'static str }`.
- [x] `write/operating_point_set.rs`: `write_operating_point_set(writer, ops, obu_xlayer_id)`
      inverting `parse_operating_point_set` (§ 5.10/§ 5.11 + § 5.11.1–5.11.5), reject-before-write
      (byte-align; `payloads.len()` == `ops_cnt`; per-payload/entry `index`; every gated `Option`
      presence vs its gate incl. the global-vs-local branch; primitive field-width / `uvlc` rejects).
      Re-export in `write/mod.rs`.
- [x] `write/dispatch.rs`: route `ParsedObu::OperatingPointSet` to the new writer (threading
      `obu_xlayer_id`) + the generic tail; reject a non-empty passthrough; drop it from the
      `Unimplemented` arm; update the written/unwritten doc counts.

## Tests and proof
- [x] `operating_point_set.rs` writer tests: round-trip (write → `parse_operating_point_set`) for the
      reset (ops_cnt == 0), global-xlayer, local-xlayer, and multi-payload-with-gated-sub-structs
      forms; reject tests for each constructed-model invariant. A dispatch round-trip test
      (`ParsedObu::OperatingPointSet` → `write_complete_obu` → reparse) covering the global branch.

## Matrix and docs
- [x] `AV2-5.10-OPERATING-POINT-SET` write `todo` → `done` (+ the §5.11.x rows as inverted) + note +
      proof; `ENC-BITSTREAM-WRITER` note: six unwritten types remain. Regenerate
      `docs/FEATURE-STATUS.md` (explicit `--output`).

## Checks
- [x] `cargo xtask ci` and `openspec validate operating-point-set-writer --strict`
