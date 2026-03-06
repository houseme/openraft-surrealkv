#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${1:-http://127.0.0.1:8080}"
DURATION="${2:-20s}"
CONCURRENCY="${3:-100}"
THREADS="${4:-4}"

echo "== OpenRaft-SurrealKV Benchmark =="
echo "target:      ${BASE_URL}"
echo "duration:    ${DURATION}"
echo "concurrency: ${CONCURRENCY}"
echo "threads:     ${THREADS}"

if command -v wrk >/dev/null 2>&1; then
  echo "[1/3] warm up"
  wrk -t"${THREADS}" -c"${CONCURRENCY}" -d5s "${BASE_URL}/health" | cat

  echo "[2/3] read benchmark: GET /kv/bench_key"
  wrk -t"${THREADS}" -c"${CONCURRENCY}" -d"${DURATION}" "${BASE_URL}/kv/bench_key" | cat

  echo "[3/3] status benchmark: GET /status"
  wrk -t"${THREADS}" -c"${CONCURRENCY}" -d"${DURATION}" "${BASE_URL}/status" | cat
else
  echo "wrk not found, fallback to curl loop (coarse benchmark)."
  start=$(date +%s)
  count=0
  end=$((start + 10))
  while [ "$(date +%s)" -lt "${end}" ]; do
    curl -s "${BASE_URL}/health" >/dev/null || true
    count=$((count + 1))
  done
  now=$(date +%s)
  elapsed=$((now - start))
  qps=$((count / (elapsed == 0 ? 1 : elapsed)))
  echo "approx_qps=${qps} requests=${count} elapsed=${elapsed}s"
fi

printf "\nTip: collect Prometheus metrics during benchmark:\n"
echo "curl -s ${BASE_URL}/metrics | grep -E 'http_requests_total|http_request_duration_ms' | cat"

