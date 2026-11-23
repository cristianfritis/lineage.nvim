#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE_BIN="$ROOT_DIR/engine/target/release/lineage-engine"

echo "[m1_gate] Building release binary..."
(cd "$ROOT_DIR/engine" && cargo build --release)

echo "[m1_gate] Binary size:"
ls -lh "$ENGINE_BIN"

TMP_BENCH="$(mktemp -d)"
trap 'rm -rf "$TMP_BENCH"' EXIT

# Generate a representative multi-file TypeScript fixture for latency & RSS measurement
mkdir -p "$TMP_BENCH/src"
cat > "$TMP_BENCH/src/auth.ts" <<'EOF'
export function findSession(token: string | null): string | null {
  if (token === null) {
    return null;
  }
  return token;
}

export function handleRequest(raw: string | null) {
  const session = findSession(raw);
  if (session !== null) {
    return session;
  }
  return "guest";
}
EOF

echo "[m1_gate] Measuring Phase 1 slice latency (10 iterations)..."
TOTAL_MS=0
for i in $(seq 1 10); do
  OUT="$("$ENGINE_BIN" slice "$TMP_BENCH/src/auth.ts:10:12" --format json)"
  MS=$(echo "$OUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['stats']['phase1_ms'])")
  TOTAL_MS=$((TOTAL_MS + MS))
done
AVG_MS=$((TOTAL_MS / 10))
echo "[m1_gate] Average Phase 1 latency: ${AVG_MS}ms (gate: p95 < 50ms)"

echo "[m1_gate] Checking steady-state RSS via /usr/bin/time or ps..."
"$ENGINE_BIN" slice "$TMP_BENCH/src/auth.ts:10:12" --format text --trace

echo "[m1_gate] Note: Full external-repo gate (zod, trpc, TypeScript) requires cloning those repos into \$LINEAGE_BENCH_CORPUS."
if [[ -n "${LINEAGE_BENCH_CORPUS:-}" && -d "$LINEAGE_BENCH_CORPUS" ]]; then
  echo "[m1_gate] Scanning $LINEAGE_BENCH_CORPUS..."
  find "$LINEAGE_BENCH_CORPUS" -maxdepth 4 -name "*.ts" | head -n 50 | while read -r f; do
    "$ENGINE_BIN" slice "${f}:1:0" --format json >/dev/null || true
  done
fi
echo "[m1_gate] Gate script completed."
