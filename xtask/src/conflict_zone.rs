// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Conflict-zone guard for the validator productization stream.
//!
//! `cargo xtask check-conflict-zone` compares the working branch's committed diff
//! against `main` (merge-base relative; equivalent to
//! `git diff --name-only main...HEAD`, computed as `merge-base(main,HEAD)..HEAD`)
//! to a committed denylist of decoder-owned paths and fails if any changed path
//! falls inside the conflict zone. This lets each validator/inspector change prove
//! mechanically that it touches nothing the concurrent decoder stream owns.
//!
//! The guard is scoped to the validator stream and never breaks the decoder
//! stream: it skips (returns `Ok` with a notice) when there is no resolvable
//! `main` base, when the diff is empty, when the branch is a decoder-stream
//! branch (a `decode`/`recon` name token — resolved from `SPLOT_PR_HEAD_REF` in
//! CI, where the PR checkout is a detached HEAD, otherwise from the local
//! branch), or when `SPLOT_SKIP_CONFLICT_ZONE=1` is set.

use std::path::Path;

use anyhow::{Result, bail};

use crate::git_util::run_git;

/// Decoder-owned directory trees / path prefixes. A changed path that starts with
/// any of these is in the conflict zone.
const FORBIDDEN_PREFIXES: &[&str] = &[
    "crates/splot-decode/",
    "crates/splot-recon/",
    "docs/DECODER-",
    "fuzz/fuzz_targets/decode",
];

/// Decoder-owned exact files.
const FORBIDDEN_EXACT: &[&str] = &[
    "docs/LOCAL-REFERENCE-EVIDENCE.toml",
    "crates/splot-cli/src/commands/decode.rs",
];

/// Workspace code/build roots under which a new `avm`/`dav2d` path is treated as a
/// forbidden integration attempt. Scoping the AVM/dav2d match to these roots keeps
/// it from firing on unrelated material such as `openspec/changes/avm-*`.
const AVM_SCAN_ROOTS: &[&str] = &[
    "crates/", "scripts/", "tools/", "fuzz/", "xtask/", ".github/",
];

/// Environment variable that, when set to `1`, makes the guard skip entirely. The
/// explicit escape hatch for any legitimate conflict-zone edit.
const SKIP_ENV: &str = "SPLOT_SKIP_CONFLICT_ZONE";

/// Environment variable carrying the PR head branch name. CI sets this from
/// `github.head_ref` because a `pull_request` checkout is a detached HEAD where
/// the branch name is not locally derivable; it takes precedence over the local
/// branch for the decoder-stream exemption.
const PR_HEAD_REF_ENV: &str = "SPLOT_PR_HEAD_REF";

/// Candidate refs for the `main` base, tried in order.
const BASE_REFS: &[&str] = &["origin/main", "main", "FETCH_HEAD"];

/// Classifies a forward-slash repo-relative path against the conflict-zone
/// denylist, returning a stable reason when the path is forbidden.
fn is_forbidden(path: &str) -> Option<&'static str> {
    if FORBIDDEN_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return Some("decoder-owned path");
    }
    if FORBIDDEN_EXACT.contains(&path) {
        return Some("decoder-owned file");
    }
    if AVM_SCAN_ROOTS.iter().any(|root| path.starts_with(root)) && mentions_avm_or_dav2d(path) {
        return Some("adds AVM/dav2d integration");
    }
    None
}

/// Returns `true` when any path segment, split into alphanumeric tokens, equals
/// `avm` or `dav2d` (case-insensitive). Tokenizing means `av2` (the codec name)
/// never matches and only a real `avm`/`dav2d` component does.
fn mentions_avm_or_dav2d(path: &str) -> bool {
    path.split('/').any(|segment| {
        segment
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|token| {
                let token = token.to_ascii_lowercase();
                token == "avm" || token == "dav2d"
            })
    })
}

/// Returns the current branch name, or `None` when HEAD is detached (CI checks out
/// a detached merge ref, where `rev-parse --abbrev-ref HEAD` yields `HEAD`).
fn current_branch(root: &Path) -> Option<String> {
    let name = run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    let name = name.trim().to_owned();
    if name.is_empty() || name == "HEAD" {
        None
    } else {
        Some(name)
    }
}

/// Resolves the branch name used for the decoder-stream exemption: the
/// `SPLOT_PR_HEAD_REF` env (set by CI from `github.head_ref`) when present and
/// non-empty, otherwise the local branch. CI checks out a detached merge ref, so
/// the env is the only reliable source there.
fn branch_for_exemption(root: &Path) -> Option<String> {
    if let Ok(head_ref) = std::env::var(PR_HEAD_REF_ENV) {
        let head_ref = head_ref.trim().to_owned();
        if !head_ref.is_empty() {
            return Some(head_ref);
        }
    }
    current_branch(root)
}

/// Returns `true` when `branch` is a decoder-stream branch, which the guard
/// exempts. Matches `decode`/`recon` as whole name *tokens* (split on
/// non-alphanumeric chars), not bare substrings, so a validator branch such as
/// `fix/reconcile-validator-output` is not falsely exempted. A `decode*` token
/// (decode/decoder/decoded/decoding) or an exact `recon`/`reconstruct[ion]` token
/// marks a decoder-stream branch.
fn is_decoder_branch(branch: &str) -> bool {
    branch
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| {
            matches!(
                token.to_ascii_lowercase().as_str(),
                "decode"
                    | "decoder"
                    | "decoded"
                    | "decoding"
                    | "decodes"
                    | "recon"
                    | "reconstruct"
                    | "reconstruction"
            )
        })
}

/// Resolves the merge-base of the first available `main` candidate ref with HEAD,
/// or `None` when no base is resolvable (e.g. a shallow clone with no `main`).
fn resolve_base(root: &Path) -> Option<String> {
    for candidate in BASE_REFS {
        let commitish = format!("{candidate}^{{commit}}");
        if run_git(root, &["rev-parse", "--verify", "--quiet", &commitish]).is_err() {
            continue;
        }
        if let Ok(base) = run_git(root, &["merge-base", candidate, "HEAD"]) {
            let base = base.trim().to_owned();
            if !base.is_empty() {
                return Some(base);
            }
        }
    }
    None
}

/// Lists the committed paths changed between `base` and HEAD (`base..HEAD`).
///
/// `--no-renames` decomposes a rename into a delete + add so a decoder-owned file
/// renamed *out* of the conflict zone still surfaces its deleted decoder path
/// (default rename detection would coalesce it and list only the new path).
/// `core.quotepath=false` keeps non-ASCII paths raw (git would otherwise C-quote
/// them, defeating the prefix match).
fn changed_paths(root: &Path, base: &str) -> Result<Vec<String>> {
    let output = run_git(
        root,
        &[
            "-c",
            "core.quotepath=false",
            "diff",
            "--no-renames",
            "--name-only",
            base,
            "HEAD",
            "--",
        ],
    )?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Verifies the working branch's diff against `main` touches no decoder-owned
/// path. Decoder-safe: skips with a notice rather than failing when the guard does
/// not apply (see the module docs).
pub(crate) fn check_conflict_zone(root: &Path) -> Result<()> {
    if std::env::var(SKIP_ENV).is_ok_and(|value| value == "1") {
        eprintln!("check-conflict-zone: ok ({SKIP_ENV}=1; skipped)");
        return Ok(());
    }
    if let Some(branch) = branch_for_exemption(root)
        && is_decoder_branch(&branch)
    {
        eprintln!("check-conflict-zone: ok (decoder-stream branch `{branch}`; not applicable)");
        return Ok(());
    }
    let Some(base) = resolve_base(root) else {
        eprintln!("check-conflict-zone: ok (no `main` base to compare against; skipped)");
        return Ok(());
    };
    let changed = changed_paths(root, &base)?;
    if changed.is_empty() {
        eprintln!("check-conflict-zone: ok (no changes vs main)");
        return Ok(());
    }

    let offenders: Vec<(String, &'static str)> = changed
        .iter()
        .filter_map(|path| is_forbidden(path).map(|reason| (path.clone(), reason)))
        .collect();

    if offenders.is_empty() {
        eprintln!(
            "check-conflict-zone: ok ({} changed file(s), none in the decoder conflict zone)",
            changed.len()
        );
        Ok(())
    } else {
        for (path, reason) in &offenders {
            eprintln!("conflict-zone violation: {path} ({reason})");
        }
        bail!(
            "{} change(s) touch the decoder conflict zone vs main",
            offenders.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use anyhow::{Result, ensure};

    use super::{changed_paths, is_decoder_branch, is_forbidden, mentions_avm_or_dav2d, run_git};

    #[test]
    fn forbids_decoder_owned_paths() {
        for path in [
            "crates/splot-decode/src/lib.rs",
            "crates/splot-recon/src/frame.rs",
            "docs/DECODER-ROADMAP.md",
            "docs/DECODER-SUPPORT-MATRIX.toml",
            "docs/LOCAL-REFERENCE-EVIDENCE.toml",
            "crates/splot-cli/src/commands/decode.rs",
            "fuzz/fuzz_targets/decode_plan_bytes.rs",
            "scripts/run-dav2d.sh",
            "crates/splot-validate/src/avm_oracle.rs",
            "tools/avm/wrapper.py",
        ] {
            assert!(
                is_forbidden(path).is_some(),
                "expected `{path}` to be forbidden"
            );
        }
    }

    #[test]
    fn allows_validator_and_shared_paths() {
        for path in [
            "crates/splot-validate/src/checks/mod.rs",
            "crates/splot-cli/src/commands/validate.rs",
            "crates/splot-cli/src/commands/inspect.rs",
            "crates/splot-cli/src/commands/explain.rs",
            "fuzz/fuzz_targets/parse_obu.rs",
            "xtask/src/conflict_zone.rs",
            "tests/fixtures/MANIFEST.toml",
            "docs/FIXTURES.md",
            "docs/VALIDATOR-DIAGNOSTICS.md",
            // `av2` is the codec name, not the `avm` reference impl.
            "docs/spec/av2/1.0.0/index.md",
            "crates/splot-core/src/av2_stream.rs",
            // OpenSpec material is outside the AVM scan roots.
            "openspec/changes/avm-differential-harness/proposal.md",
        ] {
            assert!(
                is_forbidden(path).is_none(),
                "expected `{path}` to be allowed"
            );
        }
    }

    #[test]
    fn avm_token_match_is_segment_scoped() {
        assert!(mentions_avm_or_dav2d("crates/foo/avm.rs"));
        assert!(mentions_avm_or_dav2d("crates/foo/avm_oracle.rs"));
        assert!(mentions_avm_or_dav2d("scripts/run-dav2d.sh"));
        assert!(!mentions_avm_or_dav2d("docs/spec/av2/1.0.0/index.md"));
        // `av2` (codec name) and `cavm`-style substrings must not match the `avm` token.
        assert!(!mentions_avm_or_dav2d("crates/foo/av2.rs"));
        assert!(!mentions_avm_or_dav2d("crates/splot-core/src/cavm.rs"));
    }

    #[test]
    fn decoder_branches_are_detected() {
        assert!(is_decoder_branch("codex/decode-cli-planner-handoff"));
        assert!(is_decoder_branch("feat/recon-frame-store"));
        assert!(is_decoder_branch("feat/decoded-frame-plane-model-contract"));
        assert!(is_decoder_branch("feat/minimal-decode-tier-contract"));
        assert!(is_decoder_branch("feat/decoder-diagnostic-registry"));
        assert!(!is_decoder_branch("feat/validator-conflict-zone-guard"));
        assert!(!is_decoder_branch("feat/validate-output-controls"));
        // Token match, not bare substring: these validator names must NOT be exempted.
        assert!(!is_decoder_branch("fix/reconcile-validator-output"));
        assert!(!is_decoder_branch("feat/decoy-handling"));
    }

    /// Runs `git` in `repo` with a hermetic identity, failing the test on a
    /// non-zero exit.
    fn git(repo: &Path, args: &[&str]) -> Result<()> {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args([
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "user.name=test",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .status()?;
        ensure!(status.success(), "git {args:?} failed");
        Ok(())
    }

    /// A decoder-owned file renamed *out* of the conflict zone must still surface
    /// its deleted decoder path (the `--no-renames` requirement; openspec
    /// `validator-conflict-zone-guard` "deletes any denylisted path" scenario).
    #[test]
    fn changed_paths_flags_rename_away_and_deletion_of_decoder_file() -> Result<()> {
        let repo = std::env::temp_dir().join(format!("xtask-cz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("crates/splot-decode/src"))?;
        git(&repo, &["init", "-q"])?;
        std::fs::write(repo.join("crates/splot-decode/src/lib.rs"), "// decoder\n")?;
        git(&repo, &["add", "-A"])?;
        git(&repo, &["commit", "-q", "-m", "base"])?;
        let base = run_git(&repo, &["rev-parse", "HEAD"])?.trim().to_owned();

        // Rename the decoder file out of the conflict zone, with an edit so git
        // would otherwise score it as a rename and hide the deleted path.
        std::fs::create_dir_all(repo.join("crates/splot-validate/src"))?;
        std::fs::remove_file(repo.join("crates/splot-decode/src/lib.rs"))?;
        std::fs::write(
            repo.join("crates/splot-validate/src/moved.rs"),
            "// moved and edited\n",
        )?;
        git(&repo, &["add", "-A"])?;
        git(&repo, &["commit", "-q", "-m", "rename-away"])?;

        let changed = changed_paths(&repo, &base)?;
        assert!(
            changed
                .iter()
                .any(|path| path == "crates/splot-decode/src/lib.rs"),
            "deleted decoder path not listed: {changed:?}"
        );
        assert!(
            changed.iter().any(|path| is_forbidden(path).is_some()),
            "rename-away of a decoder file was not flagged: {changed:?}"
        );

        let _ = std::fs::remove_dir_all(&repo);
        Ok(())
    }
}
