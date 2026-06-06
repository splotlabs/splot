# Code review checklist

A concise checklist for humans and agents reviewing changes to `splot`.

## Spec correctness

- [ ] Is the AV2 spec section cited (doc comment or `// TODO(spec)`)?
- [ ] No AV1 leakage (OBU header is § 5.2.2; no AV1 OBU type table, forbidden bit,
      or size-field assumptions)?
- [ ] No invented syntax, constants, or table contents?

## Error handling

- [ ] No `unwrap` / `expect` / `panic!` / `todo!` / `unimplemented!` reachable in
      library code?
- [ ] Stubs return `Error::Unimplemented { feature }` or a structured `Diagnostic`?
- [ ] Parsers return errors (never panic) on malformed/truncated input?

## Diagnostics

- [ ] Does each validator finding have a stable `rule_id`, `severity`, `spec_section`,
      byte/bit offset (where known), and a clear `message`?

## Tests

- [ ] Positive, negative, and EOF cases for parser changes?
- [ ] Property/fuzz coverage where relevant (parsers never panic)?

## Boundaries

- [ ] Crate dependency graph unchanged (`cargo xtask check-dependency-direction`)?
- [ ] Library-first: no codec/validation logic leaked into `splot-cli`?

## Hygiene

- [ ] SPDX header on every `.rs` file (`cargo xtask check-license-headers`)?
- [ ] Public items documented?
- [ ] `cargo xtask ci` passes?
