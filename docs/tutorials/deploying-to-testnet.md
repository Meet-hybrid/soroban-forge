# Deploying to Testnet

## Prerequisites

- Stellar CLI installed.
- Funded account on Testnet.

## Build WASM

```bash
stellar contract build --locked --package soroban-forge-<contract>
```

soroban-sdk 29 contracts must be built with the Stellar CLI (v25.2.0 or
newer); plain `cargo build --target wasm32v1-none` is rejected.

## Deploy

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/soroban_forge_<contract>.wasm \
  --source-account <ACCOUNT_ID> \
  --network testnet
```

## Interact

Use the TypeScript SDK or Stellar CLI to invoke contract methods.
