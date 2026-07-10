# Deploying to Testnet

## Prerequisites

- Stellar CLI installed.
- Funded account on Testnet.

## Build WASM

```bash
cargo build --workspace --release
```

## Deploy

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/soroban_forge_<contract>.wasm \
  --source-account <ACCOUNT_ID> \
  --network testnet
```

## Interact

Use the TypeScript SDK or Stellar CLI to invoke contract methods.
