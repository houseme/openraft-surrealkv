#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

STRICT=false
if [[ "${1:-}" == "--strict" ]]; then
  STRICT=true
fi

# Allowed first verbs for public doc comments.
VERBS="Create|Return|Record|Build|Load|Parse|Resolve|Validate|Install|Apply|Define|Wrap|Serve|Handle|Run|Enable|Disable|Read|Write|Delete|Append|Decode|Encode|Expose|Initialize|Check|Ensure"

DOC_PATTERN="^///\\s+(?!(${VERBS})\\b)[A-Za-z]"
NUM_MAIN_PATTERN="^\\s*//\\s*[0-9]+(\\.[0-9]+)?"

echo "[doc-scan] scanning for non-verb-led public doc comments..."
DOC_HITS=$(rg -n --pcre2 "$DOC_PATTERN" src --glob '*.rs' || true)

if [[ -n "$DOC_HITS" ]]; then
  echo "[doc-scan] found candidates:"
  echo "$DOC_HITS"
else
  echo "[doc-scan] no non-verb-led public doc comments found."
fi

echo "[doc-scan] scanning src/main.rs for numbered lifecycle comments..."
MAIN_HITS=$(rg -n "$NUM_MAIN_PATTERN" src/main.rs || true)

if [[ -n "$MAIN_HITS" ]]; then
  echo "[doc-scan] found numbered comments in src/main.rs:"
  echo "$MAIN_HITS"
else
  echo "[doc-scan] no numbered lifecycle comments found in src/main.rs."
fi

if [[ -n "$DOC_HITS" || -n "$MAIN_HITS" ]]; then
  if [[ "$STRICT" == "true" ]]; then
    echo "[doc-scan] strict mode enabled: failing due to findings."
    exit 1
  fi
  echo "[doc-scan] non-blocking mode: findings reported only."
fi

