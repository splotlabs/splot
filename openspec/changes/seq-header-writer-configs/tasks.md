# Tasks

## Implementation
- [x] Add `crates/splot-core/src/write/segment.rs` (`write_seg_info` + check helper).
- [x] Add `crates/splot-core/src/write/seq_config.rs` (the six config writers, each with an
      up-front `check_*_encodable` validating every field before any bit).
- [x] Register the modules + re-export the public writers in `write/mod.rs`.

## Tests and proof
- [x] Per-config semantic round-trip property tests (all branches) via the public parsers.
- [x] Byte-exact unit tests; one rejection test per `WriteError` path (asserting `bit_len()==0`).
- [x] Never-panics property tests.

## Matrix and docs
- [x] Advance `write` `todo -> done` on the six § 5.4.3–§ 5.4.8 config rows and
      `AV2-5.4.9-SEGMENT-INFO`, with proof. Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate seq-header-writer-configs --strict`
