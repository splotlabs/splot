# Tasks: conformance-corpus-foundation

## 1. Corpus layout

- [x] 1.1 `tests/conformance/vectors/valid/` holds the committed AVM-generated
  valid `.ivf` vectors (bootstrap set already placed); add a `manifest.toml`
  schema mapping each vector → `{ description, expect }` where `expect` is
  `clean` or `{ diagnostics = [rule_id, ...] }`.
- [x] 1.2 `.gitignore`: ensure committed corpus vectors under
  `tests/conformance/` are tracked (add the exception alongside the existing
  `tests/fixtures/` one).

## 2. Runner

- [x] 2.1 Implement the committed conformance runner (`cargo xtask conformance`
  replacing the stub) AND/OR a `splot-validate` integration test that loads
  `manifest.toml`, runs `Validator::validate_bytes` on each committed vector,
  and asserts the expected outcome. NO AVM invocation, NO network. CI-reachable
  (a `#[test]` so `cargo test` covers it).

## 3. Docs + matrix + roadmap

- [x] 3.1 Lift the `docs/VALIDATOR-ROADMAP.md` "do not start yet" fence for the
  conformance corpus only (keep encoder/writer/decoder fenced).
- [x] 3.2 Reshape `docs/CONFORMANCE.md`: committed corpus + manifest + runner;
  AVM is a documented LOCAL oracle/generator (recipe), NOT a build/CI
  dependency.
- [x] 3.3 Update the `CONF-AVM-VALID-STREAMS` row (proof: the committed vectors
  + runner) and reframe `CONF-AVM-DIFF-HARNESS` (local oracle, not CI);
  set `openspec_change` on touched rows.

## 4. Verification

- [x] 4.1 The runner asserts clean on the committed valid vectors (and the
  manifest's `diagnostics` path is exercised, even if by a bootstrap negative,
  so it is not vacuous).
- [x] 4.2 `cargo xtask ci` (bare, exit checked) passes.
