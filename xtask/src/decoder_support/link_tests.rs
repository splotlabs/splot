// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

const SAMPLE: &str = r#"
matrix_version = 1
last_reviewed = "2026-06-13"

[[row]]
id = "dec-b-row"
name = "B row"
feature_id = ""
spec_sections = ["7.1"]
parser_source = "crates/splot-core/src/stream.rs"
decode_module = "crates/splot-decode/src/context.rs"
tier = "tier-0"
status = "todo"
self_contained_tests = []
diagnostics = ["decode/unsupported"]
local_reference_evidence = ["AVM commit f6f0b9c89 raw hash metadata"]
notes = "planned"

[[row]]
id = "dec-a-row"
name = "A row"
feature_id = "DEC-A-ROW"
spec_sections = []
parser_source = "crates/splot-core/src/obu.rs"
decode_module = "crates/splot-decode/src/context.rs"
tier = "tier-1"
status = "supported"
self_contained_tests = ["cargo test -p xtask decoder_support"]
fixtures = ["tests/conformance/vectors/valid/syn-key-intra-64x64.ivf"]
diagnostics = []
local_reference_evidence = []
notes = "done"
"#;

const LINKED_FEATURE_ID: &str = "DOC-DETERMINISTIC-FRAME-HASH-CONTRACT";
const LINKED_EVIDENCE_ID: &str = "lref-sample";
const LINKED_FIXTURE_PATH: &str = "tests/decoder-reference/input.ivf";
const LINKED_DIGEST: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn check_decoder_support_accepts_reciprocal_manifest_pointer() -> Result<()> {
    let root = temp_root("decoder-support-linked-valid")?;
    let matrix = sample_with_reference_evidence(&format!(
        "{}{}",
        crate::reference_evidence::MANIFEST_POINTER_PREFIX,
        LINKED_EVIDENCE_ID
    ));
    write_linked_decoder_support_root(&root, &matrix, "dec-b-row")?;

    run_check_decoder_support(&root)?;

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn check_decoder_support_rejects_missing_manifest_pointer() -> Result<()> {
    let root = temp_root("decoder-support-linked-missing")?;
    let matrix = sample_with_reference_evidence(&format!(
        "{}missing-evidence",
        crate::reference_evidence::MANIFEST_POINTER_PREFIX
    ));
    write_linked_decoder_support_root(&root, &matrix, "dec-b-row")?;

    let err = run_check_decoder_support(&root).expect_err("missing evidence id should fail");
    assert!(err.to_string().contains("reference evidence link problem"));

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn check_decoder_support_rejects_non_reciprocal_manifest_pointer() -> Result<()> {
    let root = temp_root("decoder-support-linked-nonreciprocal")?;
    let matrix = sample_with_reference_evidence(&format!(
        "{}{}",
        crate::reference_evidence::MANIFEST_POINTER_PREFIX,
        LINKED_EVIDENCE_ID
    ));
    write_linked_decoder_support_root(&root, &matrix, "dec-a-row")?;

    let err = run_check_decoder_support(&root).expect_err("non-reciprocal link should fail");
    assert!(err.to_string().contains("reference evidence link problem"));

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn check_decoder_support_rejects_malformed_manifest_pointers() -> Result<()> {
    for (name, evidence) in [
        (
            "empty-id",
            crate::reference_evidence::MANIFEST_POINTER_PREFIX.to_owned(),
        ),
        (
            "parent-id",
            format!(
                "{}../lref-sample",
                crate::reference_evidence::MANIFEST_POINTER_PREFIX
            ),
        ),
        (
            "shell-suffix",
            format!(
                "{}{} && cmd",
                crate::reference_evidence::MANIFEST_POINTER_PREFIX,
                LINKED_EVIDENCE_ID
            ),
        ),
        (
            "file-url",
            "file:///tmp/LOCAL-REFERENCE-EVIDENCE.toml::lref-sample".to_owned(),
        ),
    ] {
        let root = temp_root(name)?;
        let matrix = sample_with_reference_evidence(&evidence);
        write_linked_decoder_support_root(&root, &matrix, "dec-b-row")?;

        let err = run_check_decoder_support(&root).expect_err("bad pointer should fail");
        let message = err.to_string();
        assert!(
            message.contains("reference evidence link problem")
                || message.contains("decoder support matrix problem"),
            "{evidence} produced unexpected error: {message}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
    Ok(())
}

fn sample_with_reference_evidence(evidence: &str) -> String {
    SAMPLE.replace(
        r#"local_reference_evidence = ["AVM commit f6f0b9c89 raw hash metadata"]"#,
        &format!("local_reference_evidence = [\"{evidence}\"]"),
    )
}

fn write_linked_decoder_support_root(
    root: &Path,
    matrix: &str,
    evidence_row_id: &str,
) -> Result<()> {
    let docs = root.join("docs");
    std::fs::create_dir_all(&docs)?;
    std::fs::write(docs.join("DECODER-SUPPORT-MATRIX.toml"), matrix)?;
    let expected = match validate_matrix(parse_matrix(matrix)?) {
        Ok(checked) => render_markdown(&checked),
        Err(_) => "unused because matrix validation fails\n".to_owned(),
    };
    std::fs::write(docs.join("DECODER-SUPPORT-STATUS.md"), expected)?;
    std::fs::write(
        docs.join("IMPLEMENTATION-MATRIX.toml"),
        format!("[[feature]]\nid = \"{LINKED_FEATURE_ID}\"\n"),
    )?;

    let fixture = b"fixture bytes";
    let fixture_path = root.join(LINKED_FIXTURE_PATH);
    std::fs::create_dir_all(fixture_path.parent().expect("fixture has parent"))?;
    std::fs::write(&fixture_path, fixture)?;

    std::fs::write(
        docs.join("LOCAL-REFERENCE-EVIDENCE.toml"),
        linked_manifest(evidence_row_id, fixture),
    )?;
    Ok(())
}

fn linked_manifest(evidence_row_id: &str, fixture: &[u8]) -> String {
    format!(
        r#"manifest_version = 1
last_reviewed = "2026-06-13"
[[evidence]]
id = "{LINKED_EVIDENCE_ID}"
feature_id = "{LINKED_FEATURE_ID}"
decoder_support_rows = ["{evidence_row_id}"]
kind = "reference-output-agreement"
summary = "AVM and dav2d raw decoder output digests agreed."
recorded_at = "2026-06-13"
[[evidence.fixture]]
id = "input"
role = "input-bitstream"
path = "{LINKED_FIXTURE_PATH}"
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
command_summary = "avm decoded {{fixture:input}} as raw decoder output"
output_digest_id = "avm-raw-md5"
output_digest_algorithm = "md5"
output_digest = "{LINKED_DIGEST}"
output_scope = "reference raw decoder output, not splot-dfh-sha256-v1"
[[evidence.reference_run]]
id = "dav2d"
tool = "dav2d"
executable_name = "dav2d"
tool_role = "decode-reference"
source_url = "https://code.videolan.org/videolan/dav2d"
revision = "1234567890abcdef"
version_summary = "sanitized version output"
command_summary = "dav2d decoded {{fixture:input}} as raw decoder output"
output_digest_id = "dav2d-raw-md5"
output_digest_algorithm = "md5"
output_digest = "{LINKED_DIGEST}"
output_scope = "reference raw decoder output, not splot-dfh-sha256-v1"
[[evidence.assertion]]
kind = "digest-equality"
left = "avm-raw-md5"
right = "dav2d-raw-md5"
"#,
        crate::git_util::sha256_hex(fixture),
        fixture.len()
    )
}

fn temp_root(name: &str) -> Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    Ok(root)
}
