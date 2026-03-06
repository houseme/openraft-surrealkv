#!/usr/bin/env bash
set -euo pipefail

# 环境变量检查（生产级）
if [ -z "$NODE_ID" ]; then
  echo "Error: NODE_ID not set"
  exit 1
fi

exec openraft-surrealkv