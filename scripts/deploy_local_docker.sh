#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
COMPOSE_FILE="${ROOT_DIR}/.docker/docker-compose.yml"
GRAFANA_URL="${GRAFANA_URL:-http://127.0.0.1:3000}"
PROM_URL="${PROM_URL:-http://127.0.0.1:9090}"
GRAFANA_USER="${GRAFANA_USER:-admin}"
GRAFANA_PASS="${GRAFANA_PASS:-admin}"
DASHBOARD_FILE="${ROOT_DIR}/.docker/grafana/openraft_dashboard.json"
ALERT_RULES_FILE="${ROOT_DIR}/.docker/grafana/openraft_alert_rules.yaml"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker not found" >&2
  exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "curl not found" >&2
  exit 1
fi

if ! docker compose version >/dev/null 2>&1; then
  echo "docker compose plugin not found" >&2
  exit 1
fi

if [ ! -f "${COMPOSE_FILE}" ]; then
  echo "compose file missing: ${COMPOSE_FILE}" >&2
  exit 1
fi

if [ ! -f "${DASHBOARD_FILE}" ]; then
  echo "dashboard file missing: ${DASHBOARD_FILE}" >&2
  exit 1
fi

if [ ! -f "${ALERT_RULES_FILE}" ]; then
  echo "alert rules file missing: ${ALERT_RULES_FILE}" >&2
  exit 1
fi

echo "Using compose file: ${COMPOSE_FILE}"

docker compose -f "${COMPOSE_FILE}" up -d --build

wait_ready() {
  local url="$1"
  local retries=30
  local i=1
  while [ "$i" -le "$retries" ]; do
    if curl -fsS "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    i=$((i + 1))
  done
  return 1
}

check_metrics() {
  local url="$1"
  local out
  out="$(curl -fsS "${url}")"
  echo "${out}" | grep -q "raft_current_term"
  echo "${out}" | grep -q "raft_state"
}

import_grafana_dashboard() {
  if curl -fsS "${GRAFANA_URL}/api/health" >/dev/null 2>&1; then
    echo "Grafana detected, importing dashboard..."
    curl -fsS -u "${GRAFANA_USER}:${GRAFANA_PASS}" \
      -H "Content-Type: application/json" \
      -X POST "${GRAFANA_URL}/api/dashboards/db" \
      --data @"${DASHBOARD_FILE}" >/dev/null

    echo "Validating dashboard import..."
    curl -fsS -u "${GRAFANA_USER}:${GRAFANA_PASS}" \
      "${GRAFANA_URL}/api/search?query=OpenRaft%20SurrealKV" | grep -q "OpenRaft SurrealKV"

    echo "Grafana dashboard import success"
    echo "Alert rules file ready: ${ALERT_RULES_FILE}"
  else
    echo "Grafana not reachable at ${GRAFANA_URL}, skip import (non-fatal)."
  fi
}

check_prometheus_targets() {
  if curl -fsS "${PROM_URL}/-/healthy" >/dev/null 2>&1; then
    echo "Prometheus detected, validating scrape targets..."
    local payload
    payload="$(curl -fsS "${PROM_URL}/api/v1/targets")"
    echo "${payload}" | grep -q 'openraft-node-1'
    echo "${payload}" | grep -q 'openraft-node-2'
    echo "${payload}" | grep -q 'openraft-node-3'
    echo "${payload}" | grep -q '"health":"up"'
    echo "Prometheus targets validation success"
  else
    echo "Prometheus not reachable at ${PROM_URL}, skip target validation (non-fatal)."
  fi
}

echo "\nWaiting for readiness..."
wait_ready "http://127.0.0.1:8080/ready"
wait_ready "http://127.0.0.1:8081/ready"
wait_ready "http://127.0.0.1:8082/ready"

echo "Validating /metrics output..."
check_metrics "http://127.0.0.1:8080/metrics"
check_metrics "http://127.0.0.1:8081/metrics"
check_metrics "http://127.0.0.1:8082/metrics"

if command -v promtool >/dev/null 2>&1; then
  echo "Running promtool syntax check..."
  promtool check rules "${ALERT_RULES_FILE}"
else
  echo "promtool not found, skip rule syntax check (optional)."
fi

check_prometheus_targets
import_grafana_dashboard

echo "\nCluster is ready. quick checks:"
echo "curl -s http://127.0.0.1:8080/ready | cat"
echo "curl -s http://127.0.0.1:8081/ready | cat"
echo "curl -s http://127.0.0.1:8082/ready | cat"
echo "curl -s http://127.0.0.1:8080/metrics | grep -E 'raft_current_term|raft_state' | cat"
echo "curl -s ${PROM_URL}/api/v1/targets | cat"
echo "\nStop with:"
echo "docker compose -f ${COMPOSE_FILE} down"
