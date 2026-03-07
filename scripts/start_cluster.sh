#!/bin/bash
# Start a local 3-node OpenRaft-SurrealKV cluster for manual testing.

set -e

echo "🚀 Starting OpenRaft-SurrealKV 3-node cluster..."

# Optional cleanup: remove old node data when invoked as `./scripts/start_cluster.sh clean`.
if [ "$1" = "clean" ]; then
    echo "🧹 Cleaning old data..."
    rm -rf data/node_*
fi

# Ensure per-node data directories exist.
mkdir -p data/node_1 data/node_2 data/node_3

# Node 1 process.
echo "📦 Starting Node 1 (port 50051, HTTP 8080)..."
NODE_ID=1 \
LISTEN_ADDR=127.0.0.1:50051 \
DATA_DIR=./data/node_1 \
HTTP_PORT=8080 \
LOG_LEVEL=info \
cargo run --release &

sleep 2

# Node 2 process.
echo "📦 Starting Node 2 (port 50052, HTTP 8081)..."
NODE_ID=2 \
LISTEN_ADDR=127.0.0.1:50052 \
DATA_DIR=./data/node_2 \
HTTP_PORT=8081 \
LOG_LEVEL=info \
cargo run --release &

sleep 2

# Node 3 process.
echo "📦 Starting Node 3 (port 50053, HTTP 8082)..."
NODE_ID=3 \
LISTEN_ADDR=127.0.0.1:50053 \
DATA_DIR=./data/node_3 \
HTTP_PORT=8082 \
LOG_LEVEL=info \
cargo run --release &

echo "✅ All nodes started!"
echo ""
echo "📊 HTTP API endpoints:"
echo "  Node 1: http://localhost:8080"
echo "  Node 2: http://localhost:8081"
echo "  Node 3: http://localhost:8082"
echo ""
echo "🔍 Health checks:"
echo "  curl http://localhost:8080/health"
echo "  curl http://localhost:8081/health"
echo "  curl http://localhost:8082/health"
echo ""
echo "🔍 Probes:"
echo "  curl http://localhost:8080/health"
echo "  curl http://localhost:8080/ready"
echo "  curl http://localhost:8081/health"
echo "  curl http://localhost:8081/ready"
echo "  curl http://localhost:8082/health"
echo "  curl http://localhost:8082/ready"
echo ""
echo "📈 Metrics:"
echo "  curl http://localhost:9090/metrics"
echo ""
echo "Press Ctrl+C to stop all nodes"

# Wait until interrupted so child node processes keep running.
wait
