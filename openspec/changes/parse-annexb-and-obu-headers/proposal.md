# Change: parse-annexb-and-obu-headers

## Feature IDs

- `AV2-4.11.6-LEB128`
- `AV2-5.2.2-OBU-HEADER`
- `AV2-5.2.1-OBU-TYPE`
- `AV2-B-ANNEXB-OBU-ENVELOPE`

## Why

The validator-first milestone needs a safe, spec-faithful parse of the AV2 Annex B
envelope and OBU headers, including the strongly-typed `obu_type` and layer ids.

## Scope

- Spec sections: § 4.11.6, § 5.2.1, § 5.2.2, § 6.2.2, Annex B.
- Crates/modules: `splot-core` (`leb128`, `obu`, `types`, `annexb`, `bitio`).
- Tests: unit (positive/negative/EOF) plus a `parsers_never_panic` proptest.

## Non-goals

- No payload/header-body parsing (sequence header, frame header, tile group).
- No bitstream writer.
- No AV1 OBU header fields or AV1 OBU type table.

## Acceptance criteria

- [x] LEB128 (§ 4.11.6) decodes with the 8-byte / `u32` / non-minimal rules.
- [x] AV2 OBU header (§ 5.2.2) parses, with no-extension xlayer inference.
- [x] `ObuType` (Table 6.1) and § 5.2.1 predicates exist and round-trip.
- [x] Annex B envelope parses; header parsing is bounded to the declared OBU size.
- [x] Positive, negative, and EOF tests exist; proptest covers no-panic.
- [x] Matrix rows and proof are recorded.
