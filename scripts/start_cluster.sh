#!/bin/bash
# 启动 3 节点本地集群

set -e

echo "🚀 Starting OpenRaft-SurrealKV 3-node cluster..."

# 清理旧数据（可选）
if [ "$1" = "clean" ]; then
    echo "🧹 Cleaning old data..."
    rm -rf data/node_*
fi

# 创建数据目录
mkdir -p data/node_1 data/node_2 data/node_3

# 节点 1
echo "📦 Starting Node 1 (port 50051, HTTP 8080)..."
NODE_ID=1 \
LISTEN_ADDR=127.0.0.1:50051 \
DATA_DIR=./data/node_1 \
HTTP_PORT=8080 \
LOG_LEVEL=info \
cargo run --release &

sleep 2

# 节点 2
echo "📦 Starting Node 2 (port 50052, HTTP 8081)..."
NODE_ID=2 \
LISTEN_ADDR=127.0.0.1:50052 \
DATA_DIR=./data/node_2 \
HTTP_PORT=8081 \
LOG_LEVEL=info \
cargo run --release &

sleep 2

# 节点 3
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

# 等待中断信号
wait
