#!/usr/bin/env bash
# =============================================================================
# openraft-surrealkv local 3-node launcher (development helper)
# =============================================================================

set -euo pipefail

echo "Starting openraft-surrealkv local 3-node setup..."

# Clean previous local artifacts to avoid stale state interference.
rm -rf data{1,2,3} snapshots{1,2,3} temp_* 2>/dev/null || true

# Track node process IDs for graceful shutdown.
PIDS=()

cleanup() {
    echo ""
    echo "Received shutdown signal, stopping all node processes..."
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    echo "All nodes stopped."
    exit 0
}

trap cleanup SIGINT SIGTERM

# Start one node process with per-node env overrides.
start_node() {
    local id=$1
    local grpc=$2
    local http=$3
    local data="data$id"

    echo "Starting Node $id (Raft gRPC: $grpc, HTTP: $http)..."

    RUST_LOG=info,openraft=debug \
    NODE_ID=$id \
    LISTEN_ADDR="127.0.0.1:$grpc" \
    HTTP_PORT=$http \
    DATA_DIR="./$data" \
    cargo run --bin openraft-surrealkv --quiet &

    local pid=$!
    PIDS+=("$pid")
    echo "  -> Node $id started (PID: $pid)"
}

start_node 1 50051 8080
sleep 2
start_node 2 50052 8081
sleep 2
start_node 3 50053 8082

echo ""
echo "Local 3-node process set started."
echo "------------------------------------------------------------"
echo "  Node 1 -> gRPC:50051  HTTP:8080"
echo "  Node 2 -> gRPC:50052  HTTP:8081"
echo "  Node 3 -> gRPC:50053  HTTP:8082"
echo ""
echo "Useful API checks:"
echo "  echo -n 'hello' | curl -sS -X POST http://127.0.0.1:8080/kv/demo -d @-"
echo "  curl -sS http://127.0.0.1:8080/kv/demo"
echo "  curl -sS http://127.0.0.1:8080/status"
echo ""
echo "Press Ctrl+C to stop all nodes."

# Keep parent script alive until one process exits or a signal is received.
wait
