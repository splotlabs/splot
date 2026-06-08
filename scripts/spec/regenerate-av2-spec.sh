#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#
# Regenerate the committed AV2 specification mirror under docs/spec/av2/<version>/
# from the official AOM PDF. This is the ONLY supported way to (re)generate the
# mirror; the spec text is © Alliance for Open Media (see the mirror README).
#
# The conversion uses poppler's `pdftotext -layout` (NOT markitdown), which
# preserves the spec's pseudocode/table alignment. The downloaded PDF's sha256 is
# verified against the pinned value before anything is written.
#
# Usage:
#   scripts/spec/regenerate-av2-spec.sh \
#     --version 1.0.0 \
#     --url https://av2.aomedia.org/v1.0.0/20260528_38f28e7_AV2_Spec_v1.0.0.pdf \
#     --sha256 e9916f091e4e83446aad6b4601641c5b292e569c144c4163b26a4497573b533f \
#     [--verify] [--outdir DIR] [--pdf PATH]
#
#   --verify   Regenerate into a temp dir and diff against the committed mirror;
#              non-zero exit on any difference (does not modify the repo).
#   --outdir   Override the output directory (default docs/spec/av2/<version>).
#   --pdf      Use an existing local PDF instead of downloading (sha256 still checked).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

VERSION=""
URL=""
SHA256=""
OUTDIR=""
PDF_PATH=""
VERIFY=0

die() { echo "regenerate-av2-spec: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  # Value-taking flags must be followed by a value; check before reading $2 so a
  # trailing "--version" with no value fails clearly instead of via `set -u`.
  case "$1" in
    --version|--url|--sha256|--outdir|--pdf)
      [ $# -ge 2 ] || die "option $1 requires a value" ;;
  esac
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --url)     URL="$2"; shift 2 ;;
    --sha256)  SHA256="$2"; shift 2 ;;
    --outdir)  OUTDIR="$2"; shift 2 ;;
    --pdf)     PDF_PATH="$2"; shift 2 ;;
    --verify)  VERIFY=1; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$VERSION" ] || die "missing --version"
[ -n "$SHA256" ] || die "missing --sha256"
[ -n "$URL" ] || [ -n "$PDF_PATH" ] || die "missing --url (or --pdf)"
[ -n "$OUTDIR" ] || OUTDIR="$REPO_ROOT/docs/spec/av2/$VERSION"

command -v pdftotext >/dev/null 2>&1 || die "pdftotext not found. Install poppler (e.g. 'brew install poppler' or 'apt-get install poppler-utils')."
command -v python3 >/dev/null 2>&1 || die "python3 not found."

# sha256 helper: prefer shasum (macOS), fall back to sha256sum (Linux).
sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# 1. Obtain the PDF.
if [ -n "$PDF_PATH" ]; then
  cp "$PDF_PATH" "$WORK/spec.pdf"
else
  echo "Downloading $URL ..." >&2
  curl -sSL -o "$WORK/spec.pdf" "$URL" || die "download failed"
fi

# 2. Verify sha256 BEFORE doing anything with the bytes.
GOT="$(sha256_of "$WORK/spec.pdf")"
if [ "$GOT" != "$SHA256" ]; then
  die "PDF sha256 mismatch: expected $SHA256, got $GOT (refusing to continue)"
fi
echo "PDF sha256 verified: $GOT" >&2

# 3. Convert with poppler, capturing the exact version for provenance.
# `sed -n 1p` consumes all of pdftotext's output (no early-close SIGPIPE under
# `set -o pipefail`, unlike `head -1`).
POPPLER_VERSION="$(pdftotext -v 2>&1 | sed -n '1p')"
pdftotext -layout "$WORK/spec.pdf" "$WORK/raw.txt"

# 4. Structure into the mirror (or a temp dir, in --verify mode).
TARGET="$OUTDIR"
[ "$VERIFY" -eq 1 ] && TARGET="$WORK/mirror"

python3 "$SCRIPT_DIR/build_av2_mirror.py" \
  --raw "$WORK/raw.txt" \
  --outdir "$TARGET" \
  --version "$VERSION" \
  --pdf-url "${URL:-local:$PDF_PATH}" \
  --pdf-sha256 "$SHA256" \
  --poppler-version "$POPPLER_VERSION"

# 5. In --verify mode, diff against the committed mirror.
if [ "$VERIFY" -eq 1 ]; then
  if [ ! -d "$OUTDIR" ]; then
    die "--verify: committed mirror $OUTDIR does not exist"
  fi
  if diff -ru "$OUTDIR" "$TARGET" >/dev/null; then
    echo "verify: committed mirror matches a fresh regeneration." >&2
  else
    echo "verify: committed mirror DIFFERS from a fresh regeneration:" >&2
    diff -ru "$OUTDIR" "$TARGET" >&2 || true
    exit 1
  fi
else
  echo "Wrote mirror to $OUTDIR" >&2
fi
