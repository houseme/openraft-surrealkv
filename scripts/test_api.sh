#!/bin/bash
# Exercise core HTTP endpoints against a running node.

BASE_URL=${1:-http://localhost:8080}

echo "🧪 Testing OpenRaft-SurrealKV HTTP API..."
echo "Base URL: $BASE_URL"
echo ""

# 1) Liveness check.
echo "1️⃣  Health Check:"
curl -s "$BASE_URL/health" | jq .
echo ""

# 2) Write a key via POST body bytes.
echo "2️⃣  PUT /kv/test_key:"
echo "Hello, OpenRaft!" | curl -s -X POST "$BASE_URL/kv/test_key" -d @- | jq .
echo ""

# 3) Read the key back (value is Base64-encoded in JSON).
echo "3️⃣  GET /kv/test_key:"
curl -s "$BASE_URL/kv/test_key" | jq .
echo ""

# 4) Write another key for multi-key sanity checking.
echo "4️⃣  PUT /kv/another_key:"
echo "Another value" | curl -s -X POST "$BASE_URL/kv/another_key" -d @- | jq .
echo ""

# 5) Query node status (role/term/applied index).
echo "5️⃣  GET /status:"
curl -s "$BASE_URL/status" | jq .
echo ""

# 6) Delete the first key.
echo "6️⃣  DELETE /kv/test_key:"
curl -s -X DELETE "$BASE_URL/kv/test_key" | jq .
echo ""

# 7) Confirm deletion result and HTTP status code.
echo "7️⃣  GET /kv/test_key (should be 404):"
curl -s "$BASE_URL/kv/test_key" -w "\nHTTP Status: %{http_code}\n"
echo ""

echo "✅ API test completed!"
