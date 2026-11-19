# Deploying to Testnet

## Prerequisites

- Stellar CLI 25.2.0 or newer.
- Caller-provided funded identities for live deployment.
- An existing SEP-41 token ID for contract smoke flows.

## Build WASM

```bash
stellar contract build --locked --profile release --optimize=false
```

The pinned workspace builds six deployable contract artifacts. `cargo build
--target wasm32v1-none` is not the supported build path for Soroban SDK 29.

## Dry run

The deployment workflow is credential-free and offline in dry-run mode:

```bash
bash scripts/deploy-testnet.sh --dry-run \
  --receipt artifacts/testnet-deployment-manifest.json
```

This writes a six-contract JSON template and performs no CLI, network, build,
funding, or key operations.

## Live deployment

Provide existing Stellar CLI identities; this script never creates, prints,
funds, stores, or commits private keys.

```bash
SOURCE_IDENTITY=<FUNDED_IDENTITY> \
TOKEN_ID=<SEP41_CONTRACT_ID> \
ESCROW_ID=<OPTIONAL_EXISTING_ID> \
# Add one identity variable per contract smoke flow...
bash scripts/deploy-testnet.sh \
  --receipt artifacts/testnet-deployment-manifest.json
```

The script builds artifacts, validates their hashes against
`provenance-manifest.json`, deploys or selects each contract independently,
then runs one minimal smoke flow. Success and failure receipts include the
contract ID, WASM hash, deployment transaction, verification status, smoke
transaction, expected event list, and key state.

Supported contracts:

- escrow
- vesting
- multi-sig-wallet
- dao-governance
- subscription-payments
- marketplace-royalties

Run `bash scripts/deploy-testnet.sh --help` for the complete environment and
identity configuration.

## Existing escrow proof

The escrow-specific live proof and receipt remain available through
`scripts/demo-testnet.sh`; use `--verify-deployed` for a read-only check or
`--dry-run` for a credential-free plan.
