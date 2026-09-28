#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Soroban Forge — TypeScript client generation for all six contracts.
#
# Single repeatable source of truth for the multi-contract SDK under
# `packages/typescript-sdk`. Builds each contract WASM locally for the
# repository's `wasm32v1-none` target and runs the pinned Stellar CLI's
# `contract bindings typescript --wasm` to emit fully typed bindings
# **offline** (no deployed contract id needed), writes them into
# `packages/typescript-sdk/src/generated/`, and regenerates drift test fixtures.
#
# The script is safe to run repeatedly: generated bindings and fixtures are
# deterministically regenerated and produce a clean diff.
#
# Requires:
#   - stable Rust + the wasm32v1-none target (see rust-toolchain.toml)
#   - the Stellar CLI (`stellar`), pinned at 25.1.0 in the workspace
#
# Usage:
#   bash scripts/generate-clients.sh
# ---------------------------------------------------------------------------
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET="wasm32v1-none"
RELEASE_DIR="target/$TARGET/release"
SDK_DIR="$ROOT/packages/typescript-sdk"
GENERATED_DIR="$SDK_DIR/src/generated"

# Contract crates mapping: (crate name, wasm basename, sdk generated filename)
declare -a CONTRACTS=(
  "soroban-forge-escrow|soroban_forge_escrow.wasm|escrow"
  "soroban-forge-vesting|soroban_forge_vesting.wasm|vesting"
  "soroban-forge-multi-sig-wallet|soroban_forge_multi_sig_wallet.wasm|multi-sig-wallet"
  "soroban-forge-marketplace-royalties|soroban_forge_marketplace_royalties.wasm|marketplace-royalties"
  "soroban-forge-subscription-payments|soroban_forge_subscription_payments.wasm|subscription-payments"
  "soroban-forge-dao-governance|soroban_forge_dao_governance.wasm|dao-governance"
)

mkdir -p "$GENERATED_DIR"

if command -v stellar >/dev/null 2>&1; then
  echo "==> Building contract WASMs for target '$TARGET'"
  cargo build --locked --release --target "$TARGET" \
    --package soroban-forge-escrow \
    --package soroban-forge-vesting \
    --package soroban-forge-multi-sig-wallet \
    --package soroban-forge-marketplace-royalties \
    --package soroban-forge-subscription-payments \
    --package soroban-forge-dao-governance

  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "$TMP_DIR"' EXIT

  for entry in "${CONTRACTS[@]}"; do
    IFS='|' read -r _crate wasm client_name <<< "$entry"
    wasm_path="$RELEASE_DIR/$wasm"
    if [ ! -f "$wasm_path" ]; then
      echo "error: expected WASM missing: $wasm_path" >&2
      exit 1
    fi
    echo "==> Generating bindings for $client_name from $wasm_path"
    out_dir="$TMP_DIR/$client_name"
    rm -rf "$out_dir"
    mkdir -p "$out_dir"
    stellar contract bindings typescript \
      --wasm "$wasm_path" \
      --output-dir "$out_dir" \
      --overwrite

    # Copy the generated raw TypeScript file into packages/typescript-sdk/src/generated/
    if [ -f "$out_dir/src/index.ts" ]; then
      cp "$out_dir/src/index.ts" "$GENERATED_DIR/${client_name}.ts"
    elif [ -f "$out_dir/index.ts" ]; then
      cp "$out_dir/index.ts" "$GENERATED_DIR/${client_name}.ts"
    fi
  done
else
  echo "==> Stellar CLI ('stellar') not found in PATH."
  echo "    Skipping WASM compilation and using pinned bindings in $GENERATED_DIR."
fi

# Regenerate fixtures and verify build in typescript-sdk
echo "==> Regenerating ABI expectation fixtures in packages/typescript-sdk"
(cd "$SDK_DIR" && npm run regen:fixtures)

echo "==> Building packages/typescript-sdk distribution"
(cd "$SDK_DIR" && npm run build)

echo "==> Client regeneration complete."