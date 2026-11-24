#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 <engine-tag>" >&2
  exit 1
fi

ENGINE_TAG="$1"
REPO="cristianfritis/lineage.nvim"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
URL="https://github.com/${REPO}/releases/download/${ENGINE_TAG}/checksums.json"

TMP_FILE="$(mktemp "${ROOT_DIR}/checksums.json.tmp.XXXXXX")"
trap 'rm -f "$TMP_FILE"' EXIT

echo "[sync_checksums] Downloading ${URL}..."
curl -fsSL --connect-timeout 15 --max-time 60 -o "$TMP_FILE" "$URL"

python3 - "$TMP_FILE" "$ENGINE_TAG" "$REPO" <<'PY'
import json
import re
import sys

path, expected_tag, expected_repo = sys.argv[1:4]
with open(path, "r", encoding="utf-8") as f:
    data = json.load(f)

if data.get("repo") != expected_repo:
    raise SystemExit(f"repo mismatch: expected {expected_repo}, got {data.get('repo')}")
if data.get("engine_tag") != expected_tag:
    raise SystemExit(f"engine_tag mismatch: expected {expected_tag}, got {data.get('engine_tag')}")

expected_targets = [
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
]
targets = data.get("targets") or {}
for t in expected_targets:
    entry = targets.get(t)
    if not isinstance(entry, dict):
        raise SystemExit(f"missing target: {t}")
    sha = entry.get("sha256", "")
    if not re.fullmatch(r"[0-9a-f]{64}", sha):
        raise SystemExit(f"invalid sha256 for {t}: {sha!r}")

print(f"[sync_checksums] Verified 4 targets for {expected_tag}")
PY

mv "$TMP_FILE" "${ROOT_DIR}/checksums.json"
echo "[sync_checksums] Updated ${ROOT_DIR}/checksums.json"
