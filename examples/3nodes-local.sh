#!/usr/bin/env bash
# =============================================================================
# openraft-surrealkv 三节点本地启动脚本（生产友好版）
# =============================================================================

set -euo pipefail

echo "🚀 Starting openraft-surrealkv 3-node local cluster..."

# 清理旧数据
rm -rf data{1,2,3} snapshots{1,2,3} temp_* 2>/dev/null || true

# 记录进程 PID 用于优雅退出
PIDS=()

cleanup() {
    echo -e "\n🛑 Received shutdown signal, killing all nodes..."
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    echo "✅ All nodes stopped."
    exit 0
}

trap cleanup SIGINT SIGTERM

# ====================== 启动节点 ======================
start_node() {
    local id=$1
    local grpc=$2
    local http=$3
    local data="data$id"

    echo "Starting Node $id (gRPC:$grpc | HTTP:$http) ..."

    RUST_LOG=info,openraft=debug \
    NODE_ID=$id \
    GRPC_PORT=$grpc \
    HTTP_PORT=$http \
    DATA_DIR=./$data \
    cargo run --bin openraft-surrealkv --quiet &

    local pid=$!
    PIDS+=("$pid")
    echo "   → Node $id started (PID: $pid)"
}

start_node 1 50051 8080
sleep 2
start_node 2 50052 8081
sleep 2
start_node 3 50053 8082

echo ""
echo "✅ 三节点集群已成功启动！"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "   Node 1 → gRPC:50051  HTTP:8080"
echo "   Node 2 → gRPC:50052  HTTP:8081"
echo "   Node 3 → gRPC:50053  HTTP:8082"
echo ""
echo "常用测试命令："
echo "  curl -X POST http://127.0.0.1:8080/write -H 'Content-Type: application/json' -d '{\"Set\":{\"key\":\"name\",\"value\":\"zhigang\"}}'"
echo "  curl http://127.0.0.1:8080/membership"
echo "  curl -X POST http://127.0.0.1:8080/change_membership -H 'Content-Type: application/json' -d '[[2,\"127.0.0.1:50052\"],[3,\"127.0.0.1:50053\"]]'"
echo ""
echo "按 Ctrl+C 停止所有节点..."

# 等待任意节点退出
wait#!/usr/bin/env bash
    # =============================================================================
    # openraft-surrealkv 三节点本地启动脚本（生产友好版）
    # =============================================================================

    set -euo pipefail

    echo "🚀 Starting openraft-surrealkv 3-node local cluster..."

    # 清理旧数据
    rm -rf data{1,2,3} snapshots{1,2,3} temp_* 2>/dev/null || true

    # 记录进程 PID 用于优雅退出
    PIDS=()

    cleanup() {
        echo -e "\n🛑 Received shutdown signal, killing all nodes..."
        for pid in "${PIDS[@]}"; do
            kill "$pid" 2>/dev/null || true
        done
        wait 2>/dev/null || true
        echo "✅ All nodes stopped."
        exit 0
    }

    trap cleanup SIGINT SIGTERM

    # ====================== 启动节点 ======================
    start_node() {
        local id=$1
        local grpc=$2
        local http=$3
        local data="data$id"

        echo "Starting Node $id (gRPC:$grpc | HTTP:$http) ..."

        RUST_LOG=info,openraft=debug \
        NODE_ID=$id \
        GRPC_PORT=$grpc \
        HTTP_PORT=$http \
        DATA_DIR=./$data \
        cargo run --bin openraft-surrealkv --quiet &

        local pid=$!
        PIDS+=("$pid")
        echo "   → Node $id started (PID: $pid)"
    }

    start_node 1 50051 8080
    sleep 2
    start_node 2 50052 8081
    sleep 2
    start_node 3 50053 8082

    echo ""
    echo "✅ 三节点集群已成功启动！"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "   Node 1 → gRPC:50051  HTTP:8080"
    echo "   Node 2 → gRPC:50052  HTTP:8081"
    echo "   Node 3 → gRPC:50053  HTTP:8082"
    echo ""
    echo "常用测试命令："
    echo "  curl -X POST http://127.0.0.1:8080/write -H 'Content-Type: application/json' -d '{\"Set\":{\"key\":\"name\",\"value\":\"zhigang\"}}'"
    echo "  curl http://127.0.0.1:8080/membership"
    echo "  curl -X POST http://127.0.0.1:8080/change_membership -H 'Content-Type: application/json' -d '[[2,\"127.0.0.1:50052\"],[3,\"127.0.0.1:50053\"]]'"
    echo ""
    echo "按 Ctrl+C 停止所有节点..."

    # 等待任意节点退出
    wait