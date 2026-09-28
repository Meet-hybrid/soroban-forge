#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SDK_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT_DIR="$(cd "$SDK_DIR/../.." && pwd)"

cd "$SDK_DIR"

# If stellar CLI is available, optionally re-run generate-clients.sh from root
if command -v stellar >/dev/null 2>&1; then
  echo "==> Stellar CLI detected. Running root client generator..."
  bash "$ROOT_DIR/scripts/generate-clients.sh"
else
  echo "==> Stellar CLI not found in PATH; updating fixtures from pinned generated bindings..."
  npm run regen:fixtures
fi
