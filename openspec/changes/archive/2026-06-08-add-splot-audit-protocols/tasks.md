## 1. Tracking

- [x] 1.1 Add matrix row `DOC-AUDIT-PROTOCOLS` for the repo-local audit skills and guidance.
- [x] 1.2 Add matrix row `XTASK-AUDIT-SCOPE` for deterministic audit scope and ledger tooling.
- [x] 1.3 Regenerate `docs/FEATURE-STATUS.md` after matrix changes.

## 2. Documentation Audit Skill

- [x] 2.1 Create `.codex/skills/splot-doc-audit/SKILL.md` with metadata that triggers for scheduled or on-demand `splot` documentation audits.
- [x] 2.2 Create the matching `.claude/skills/splot-doc-audit/SKILL.md` and keep its protocol content aligned with the Codex skill.
- [x] 2.3 Adapt the attached knowledge-base audit protocol to `splot` paths, licensing exceptions, OpenSpec-generated assistant integrations, and the read-only AV2 spec mirror.
- [x] 2.4 Define documentation audit outputs: small doc-only PR/report, blocking conflicts, claim summary, evidence, recommendations, and no auto-merge.

## 3. AV2 Conformance Audit Skill

- [x] 3.1 Create `.codex/skills/splot-av2-conformance-audit/SKILL.md` with metadata that triggers for heavy AV2 conformance audits.
- [x] 3.2 Create the matching `.claude/skills/splot-av2-conformance-audit/SKILL.md` and keep its protocol content aligned with the Codex skill.
- [x] 3.3 Define the coordinator workflow: run audit-scope first, select reviewer lanes, merge findings, and require human review for ambiguous AV2 interpretation.
- [x] 3.4 Define reviewer lanes for spec citation, parser safety, encoder/decoder/writer/inspector behavior, validator diagnostics, feature matrix/OpenSpec consistency, tests/fuzz/conformance, and safety/boundary rules.
- [x] 3.5 Specify that heavy audit findings do not directly change parser/validator/encoder/decoder behavior; implementation defects become issues or follow-up OpenSpec changes unless the user explicitly requests a fix.

## 4. Deterministic Audit Scope

- [x] 4.1 Add audit-scope tooling under `xtask` or a script invoked by `xtask`, without adding third-party dependencies.
- [x] 4.2 Support PR/diff mode that selects files changed from a supplied base revision.
- [x] 4.3 Support scheduled mode that compares tracked file content hashes against `docs/audits/av2-conformance-ledger.json`.
- [x] 4.4 Emit machine-readable JSON containing candidate files, reasons, impacted Feature IDs when known, and force-wide-review triggers.
- [x] 4.5 Add deterministic ledger update output containing protocol version, audited commit, file paths, content hashes, impacted Feature IDs, and outcome.
- [x] 4.6 Discover workspace members and codec-facing paths dynamically so future encoder, decoder, writer, inspector, conformance, fuzz, and automation files are selected without hardcoded crate-name updates.

## 5. Repo Guidance

- [x] 5.1 Add a short pointer in `AGENTS.md` to use the documentation audit skill for guidance/doc audits and the AV2 conformance audit skill for heavy spec-fidelity audits.
- [x] 5.2 Keep `CLAUDE.md` and `.github/copilot-instructions.md` aligned only if their pointer content needs to change.
- [x] 5.3 Decide whether matching `.github/skills/` or scheduled prompt files are needed for GitHub-hosted agents; add them only if they remain isolated under the approved assistant-integration paths.
- [x] 5.4 Do not rely on `.agents/skills/` as the only repo skill location; if a shared source is introduced, generate or mirror into `.codex/skills/` and `.claude/skills/`.

## 6. Tests and Validation

- [x] 6.1 Add unit tests for audit-scope file classification, changed-hash detection, force-wide triggers, dynamic workspace-member discovery, and deterministic ledger output.
- [x] 6.2 Run `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [x] 6.3 Run `cargo xtask check-feature-status`.
- [x] 6.4 Run `cargo xtask ci`.
