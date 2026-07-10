# Getting Started

This tutorial will walk you through building and testing a Soroban Forge contract locally.

## Prerequisites

- Rust 1.75 or later
- `rustup target add wasm32-unknown-unknown`
- `cargo install soroban-cli`

## Step 1: Build

```bash
cd path/to/soroban-forge
cargo build --workspace
```

## Step 2: Run Tests

```bash
cargo test --workspace
```

## Step 3: Run Lints

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

## Step 4: Deploy

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/soroban_forge_escrow.wasm \
  --source-account GD...
  --network testnet
```

## Next Steps

- Explore the [Architecture](architecture/architecture.md)
- Review [Security Best Practices](best-practices/smart-contract-security.md)
- Read the [Contracts Overview](contracts/index.md)
