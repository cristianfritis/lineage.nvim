#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "[stage_a_gate] Building release engine (default allocator) and dogfood_gate..."
(cd "$ROOT_DIR/engine" && cargo build --release && cargo build --release --example dogfood_gate)

echo "[stage_a_gate] Running Layer 1 dogfood_gate (default allocator)..."
"$ROOT_DIR/engine/target/release/examples/dogfood_gate" \
  --manifest "$ROOT_DIR/benchmarks/corpus_manifest.json" \
  --engine-bin "$ROOT_DIR/engine/target/release/lineage-engine" \
  --cache-dir "$ROOT_DIR/.dogfood-cache" \
  --report "$ROOT_DIR/benchmarks/reports/stage_a_report.json"

echo "[stage_a_gate] Building release engine with --features mimalloc and verifying RSS..."
(cd "$ROOT_DIR/engine" && cargo build --release --features mimalloc)
"$ROOT_DIR/engine/target/release/examples/dogfood_gate" \
  --manifest "$ROOT_DIR/benchmarks/corpus_manifest.json" \
  --engine-bin "$ROOT_DIR/engine/target/release/lineage-engine" \
  --cache-dir "$ROOT_DIR/.dogfood-cache" \
  --report "$ROOT_DIR/benchmarks/reports/stage_a_report_mimalloc.json"

echo "[stage_a_gate] All Stage A Layer 1 gates PASSED."
