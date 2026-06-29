// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::git_util::sha256_hex;

const FEATURE_ID: &str = "DOC-DETERMINISTIC-FRAME-HASH-CONTRACT";
const ROW_ID: &str = "deterministic-frame-hash";
const FIXTURE_PATH: &str = "tests/decoder-reference/input.ivf";
const DIGEST: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn empty_committed_manifest_is_valid() -> Result<()> {
    let root = temp_root("empty")?;
    write_manifest(
        &root,
        "manifest_version = 1\nlast_reviewed = \"2026-06-13\"\n",
    )?;

    run_check_reference_evidence(&root)?;

    finish(root)
}

#[test]
fn valid_manifest_with_fixture_and_digest_assertion_passes() -> Result<()> {
    let root = temp_root("valid")?;
    let data = write_fixture(&root)?;
    write_matrices(&root)?;
    write_manifest(&root, &valid_manifest(&data))?;

    run_check_reference_evidence(&root)?;

    finish(root)
}

#[test]
fn duplicate_evidence_ids_are_rejected() -> Result<()> {
    let root = valid_root("dup-evidence")?;
    let mut text = std::fs::read_to_string(manifest_path(&root))?;
    text.push_str("\n[[evidence]]\nid = \"lref-sample\"\nfeature_id = \"DOC-DETERMINISTIC-FRAME-HASH-CONTRACT\"\ndecoder_support_rows = [\"deterministic-frame-hash\"]\nkind = \"reference-output-agreement\"\nsummary = \"duplicate\"\nrecorded_at = \"2026-06-13\"\n");
    std::fs::write(manifest_path(&root), text)?;

    let err = run_check_reference_evidence(&root).expect_err("duplicate id should fail");
    assert!(err.to_string().contains("duplicate evidence id"));
    finish(root)
}

#[test]
fn invalid_feature_and_decoder_support_rows_are_rejected() -> Result<()> {
    let root = valid_root("bad-refs")?;
    replace_manifest(&root, FEATURE_ID, "CLI-DECODE")?;
    replace_manifest(&root, ROW_ID, "not-real-row")?;

    let err = run_check_reference_evidence(&root).expect_err("bad refs should fail");
    let message = err.to_string();
    assert!(message.contains("unknown feature_id"));
    assert!(message.contains("unknown decoder support row"));
    finish(root)
}

#[test]
fn local_and_absolute_paths_are_rejected() -> Result<()> {
    for (name, path) in [
        ("unix", "/Users/me/vector.ivf"),
        ("home", "~/vector.ivf"),
        ("file", "file:///tmp/vector.ivf"),
        ("windows", "C:/tmp/vector.ivf"),
        ("parent", "../vector.ivf"),
        ("env", "$HOME/vector.ivf"),
        ("colon", "cwd:/Users/me/vector.ivf"),
    ] {
        let root = valid_root(name)?;
        replace_manifest(&root, FIXTURE_PATH, path)?;
        let err = run_check_reference_evidence(&root).expect_err("path should fail");
        assert!(
            err.to_string().contains("path"),
            "{path} produced unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(root);
    }
    Ok(())
}

#[test]
fn command_summary_rejects_paths_and_shell_composition() -> Result<()> {
    for (name, command) in [
        ("local-command", "/Users/me/avm/build/avmdec input.ivf"),
        ("assignment", "AVM=/home/me/avmdec --rawvideo"),
        ("colon", "cwd:/Users/me/avmdec input.ivf"),
        ("relative-dot", "./build/avmdec input.ivf"),
        ("relative-dir", "tools/dav2d input.ivf"),
        ("pipe", "avmdec input.ivf | md5sum"),
    ] {
        let root = valid_root(name)?;
        replace_manifest(
            &root,
            "avmdec {fixture:input} as raw decoder output",
            command,
        )?;
        let err = run_check_reference_evidence(&root).expect_err("command should fail");
        assert!(
            err.to_string().contains("command_summary"),
            "{command} produced unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(root);
    }
    Ok(())
}

#[test]
fn stale_fixture_hash_and_length_are_rejected() -> Result<()> {
    let root = valid_root("stale-fixture")?;
    replace_manifest(&root, "size_bytes = 13", "size_bytes = 12")?;
    replace_manifest(
        &root,
        &sha256_hex(b"fixture bytes"),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )?;

    let err = run_check_reference_evidence(&root).expect_err("stale fixture should fail");
    let message = err.to_string();
    assert!(message.contains("size_bytes"));
    assert!(message.contains("sha256"));
    finish(root)
}

#[test]
fn symlink_fixture_is_rejected() -> Result<()> {
    let root = valid_root("symlink")?;
    let link = root.join(FIXTURE_PATH);
    std::fs::remove_file(&link)?;
    symlink_fixture(root.join("target.ivf"), &link)?;

    let err = run_check_reference_evidence(&root).expect_err("symlink should fail");
    assert!(err.to_string().contains("must not contain symlinks"));
    finish(root)
}

#[cfg(unix)]
#[test]
fn intermediate_symlink_fixture_is_rejected() -> Result<()> {
    let root = valid_root("parent-symlink")?;
    let outside = temp_root("outside")?;
    std::fs::write(outside.join("input.ivf"), b"outside")?;
    std::os::unix::fs::symlink(&outside, root.join("tests/decoder-reference/link"))?;
    replace_manifest(
        &root,
        FIXTURE_PATH,
        "tests/decoder-reference/link/input.ivf",
    )?;

    let err = run_check_reference_evidence(&root).expect_err("parent symlink should fail");
    assert!(err.to_string().contains("must not contain symlinks"));
    let _ = std::fs::remove_dir_all(outside);
    finish(root)
}

#[test]
fn git_worktree_requires_tracked_fixture() -> Result<()> {
    let root = valid_root("git-tracked")?;
    if !git_available() {
        return finish(root);
    }
    git(&root, &["init"])?;
    let err = run_check_reference_evidence(&root).expect_err("untracked fixture should fail");
    assert!(err.to_string().contains("must be tracked by git"));

    git(&root, &["add", FIXTURE_PATH])?;
    run_check_reference_evidence(&root)?;
    finish(root)
}

#[test]
fn malformed_digest_and_broken_assertions_are_rejected() -> Result<()> {
    let root = valid_root("bad-digest")?;
    replace_manifest(&root, DIGEST, "not-md5")?;
    replace_manifest(
        &root,
        "right = \"dav2d-raw-md5\"",
        "right = \"missing-digest\"",
    )?;

    let err = run_check_reference_evidence(&root).expect_err("bad digest should fail");
    let message = err.to_string();
    assert!(message.contains("output_digest"));
    assert!(message.contains("unknown right digest id"));
    finish(root)
}

#[test]
fn duplicate_reference_run_ids_are_rejected() -> Result<()> {
    let root = valid_root("dup-run")?;
    replace_manifest(&root, "id = \"dav2d\"", "id = \"avm\"")?;

    let err = run_check_reference_evidence(&root).expect_err("duplicate run id should fail");
    assert!(err.to_string().contains("duplicate reference_run id"));
    finish(root)
}

#[test]
fn tautological_digest_assertions_are_rejected() -> Result<()> {
    let root = valid_root("tautological")?;
    replace_manifest(
        &root,
        "right = \"dav2d-raw-md5\"",
        "right = \"avm-raw-md5\"",
    )?;

    let err = run_check_reference_evidence(&root).expect_err("same digest id should fail");
    assert!(err.to_string().contains("must compare distinct digest ids"));
    finish(root)
}

#[test]
fn unequal_digest_assertions_are_rejected() -> Result<()> {
    let root = valid_root("unequal")?;
    replace_manifest(
        &root,
        &format!(
            "output_digest_id = \"dav2d-raw-md5\"\noutput_digest_algorithm = \"md5\"\noutput_digest = \"{DIGEST}\""
        ),
        "output_digest_id = \"dav2d-raw-md5\"\noutput_digest_algorithm = \"md5\"\noutput_digest = \"fedcba9876543210fedcba9876543210\"",
    )?;

    let err = run_check_reference_evidence(&root).expect_err("unequal digest should fail");
    assert!(err.to_string().contains("compares unequal digests"));
    finish(root)
}

#[allow(clippy::unnecessary_wraps)]
fn finish(root: PathBuf) -> Result<()> {
    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

fn valid_root(name: &str) -> Result<PathBuf> {
    let root = temp_root(name)?;
    let data = write_fixture(&root)?;
    write_matrices(&root)?;
    write_manifest(&root, &valid_manifest(&data))?;
    Ok(root)
}

fn write_fixture(root: &Path) -> Result<Vec<u8>> {
    let path = root.join(FIXTURE_PATH);
    std::fs::create_dir_all(path.parent().expect("fixture has parent"))?;
    let data = b"fixture bytes".to_vec();
    std::fs::write(path, &data)?;
    Ok(data)
}

fn write_matrices(root: &Path) -> Result<()> {
    let docs = root.join("docs");
    std::fs::create_dir_all(&docs)?;
    std::fs::write(
        docs.join("IMPLEMENTATION-MATRIX.toml"),
        format!("[[feature]]\nid = \"{FEATURE_ID}\"\n"),
    )?;
    std::fs::write(
        docs.join("DECODER-SUPPORT-MATRIX.toml"),
        format!("[[row]]\nid = \"{ROW_ID}\"\n"),
    )?;
    Ok(())
}

fn write_manifest(root: &Path, text: &str) -> Result<()> {
    let path = manifest_path(root);
    std::fs::create_dir_all(path.parent().expect("manifest has parent"))?;
    std::fs::write(path, text)?;
    Ok(())
}

fn replace_manifest(root: &Path, from: &str, to: &str) -> Result<()> {
    let path = manifest_path(root);
    let text = std::fs::read_to_string(&path)?.replace(from, to);
    std::fs::write(path, text)?;
    Ok(())
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join(MANIFEST_PATH)
}

fn valid_manifest(fixture_bytes: &[u8]) -> String {
    format!(
        r#"manifest_version = 1
last_reviewed = "2026-06-13"
[[evidence]]
id = "lref-sample"
feature_id = "{FEATURE_ID}"
decoder_support_rows = ["{ROW_ID}"]
kind = "reference-output-agreement"
summary = "AVM and dav2d raw decoder output digests agreed."
recorded_at = "2026-06-13"
[[evidence.fixture]]
id = "input"
role = "input-bitstream"
path = "{FIXTURE_PATH}"
sha256 = "{}"
size_bytes = {}
format = "ivf"
provenance_kind = "project-owned-synthetic-input"
provenance_summary = "Generated locally from project-owned synthetic input; AVM is not vendored."
license = "PolyForm-Noncommercial-1.0.0"
[[evidence.reference_run]]
id = "avm"
tool = "avm"
executable_name = "avmdec"
tool_role = "decode-reference"
source_url = "https://github.com/AOMediaCodec/avm"
revision = "abcdef1234567890"
version_summary = "sanitized version output"
command_summary = "avmdec {{fixture:input}} as raw decoder output"
output_digest_id = "avm-raw-md5"
output_digest_algorithm = "md5"
output_digest = "{DIGEST}"
output_scope = "reference raw decoder output, not splot-dfh-sha256-v1"
[[evidence.reference_run]]
id = "dav2d"
tool = "dav2d"
executable_name = "dav2d"
tool_role = "decode-reference"
source_url = "https://code.videolan.org/videolan/dav2d"
revision = "1234567890abcdef"
version_summary = "sanitized version output"
command_summary = "dav2d {{fixture:input}} as raw decoder output"
output_digest_id = "dav2d-raw-md5"
output_digest_algorithm = "md5"
output_digest = "{DIGEST}"
output_scope = "reference raw decoder output, not splot-dfh-sha256-v1"
[[evidence.assertion]]
kind = "digest-equality"
left = "avm-raw-md5"
right = "dav2d-raw-md5"
"#,
        sha256_hex(fixture_bytes),
        fixture_bytes.len()
    )
}

fn temp_root(name: &str) -> Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "splot-reference-evidence-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    assert!(status.success(), "git {args:?} failed with {status}");
    Ok(())
}

#[cfg(unix)]
fn symlink_fixture(target: PathBuf, link: &Path) -> Result<()> {
    std::fs::write(&target, b"target")?;
    std::os::unix::fs::symlink(target, link)?;
    Ok(())
}

#[cfg(windows)]
fn symlink_fixture(target: PathBuf, link: &Path) -> Result<()> {
    std::fs::write(&target, b"target")?;
    std::os::windows::fs::symlink_file(target, link)?;
    Ok(())
}
