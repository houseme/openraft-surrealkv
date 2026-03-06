#!/bin/bash
# 测试 HTTP API

BASE_URL=${1:-http://localhost:8080}

echo "🧪 Testing OpenRaft-SurrealKV HTTP API..."
echo "Base URL: $BASE_URL"
echo ""

# 健康检查
echo "1️⃣  Health Check:"
curl -s "$BASE_URL/health" | jq .
echo ""

# 写入键值
echo "2️⃣  PUT /kv/test_key:"
echo "Hello, OpenRaft!" | curl -s -X POST "$BASE_URL/kv/test_key" -d @- | jq .
echo ""

# 读取键值
echo "3️⃣  GET /kv/test_key:"
curl -s "$BASE_URL/kv/test_key" | jq .
echo ""

# 再写入一个键
echo "4️⃣  PUT /kv/another_key:"
echo "Another value" | curl -s -X POST "$BASE_URL/kv/another_key" -d @- | jq .
echo ""

# 集群状态
echo "5️⃣  GET /status:"
curl -s "$BASE_URL/status" | jq .
echo ""

# 删除键
echo "6️⃣  DELETE /kv/test_key:"
curl -s -X DELETE "$BASE_URL/kv/test_key" | jq .
echo ""

# 验证删除
echo "7️⃣  GET /kv/test_key (should be 404):"
curl -s "$BASE_URL/kv/test_key" -w "\nHTTP Status: %{http_code}\n"
echo ""

echo "✅ API test completed!"

