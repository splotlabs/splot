# Agent Audit Workflows

Use repo-local audit skills instead of expanding audit protocols in `AGENTS.md`.

## Documentation and Guidance Audits

Use one of:

```text
.codex/skills/splot-doc-audit/SKILL.md
.claude/skills/splot-doc-audit/SKILL.md
```

Feature ID: `DOC-AUDIT-PROTOCOLS`.

This audit covers project-authored guidance and documentation. It excludes
hand-editing the AV2 spec mirror body.

## Heavy AV2 Conformance Audits

Use one of:

```text
.codex/skills/splot-av2-conformance-audit/SKILL.md
.claude/skills/splot-av2-conformance-audit/SKILL.md
```

Feature IDs: `XTASK-AUDIT-SCOPE`, `DOC-AUDIT-PROTOCOLS`.

Heavy audits must start with:

```bash
cargo xtask audit-scope --format json
```

Use `audit-scope` so changed files, force-wide triggers, future workspace
members, and audit ledger state are selected deterministically.

Do not rely on `.agents/skills/` as the only project skill location; mirror or
generate into the agent-specific project skill paths above.

## Ledger Updates

Scheduled or completed heavy audits may write the ledger through:

```bash
cargo xtask audit-scope --write-ledger --format json
```

Only write ledger state when the audit result is known and the maintainer expects
the ledger update.
