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

## Post-deployment smoke check

`scripts/deploy-testnet.sh` records a `read_only` invocation for every
contract in the receipt. Verify the deployed contracts after a live run with:

```bash
soroban-forge deploy-check \
  --manifest artifacts/testnet-deployment-manifest.json
```

The command reads `contracts[]` from the deployment receipt, checks that each
contract's code is reachable (`stellar contract fetch`), then performs the
manifest's read-only invocation with `stellar contract invoke --send no`.
Invocations are simulated only, so the check never submits a transaction or
signs with a secret key. A public simulation source is supplied for the
transaction envelope; the default is the all-zero account and it can be
overridden with `--source-account G...`.

Each contract entry may configure its check explicitly:

```json
{
  "key": "multi-sig-wallet",
  "contract_id": "C...",
  "read_only": {
    "method": "get_threshold",
    "args": []
  }
}
```

`args` are raw Stellar CLI arguments passed after the method name. Contracts
without a `read_only` entry use the built-in defaults for
`multi-sig-wallet`, `dao-governance`, and `subscription-payments`; any other
contract must configure one. A missing contract, missing method, failed read,
or unconfigured contract ID fails the command and names the contract.

Useful flags:

```bash
# Check only selected contracts
soroban-forge deploy-check --manifest <PATH> --contract escrow,vesting

# Machine-readable summary for CI
soroban-forge deploy-check --manifest <PATH> --json
```

The text summary is deterministic and ordered by contract key, for example:

```text
deployment smoke check: network=testnet contracts=6 passed=6 failed=0
PASS escrow (C...) code=passed read=get_escrow=passed
PASS multi-sig-wallet (C...) code=passed read=get_threshold=passed
```

Dry-run receipts contain placeholder contract IDs and are not deployable;
run the smoke check against a live deployment receipt.

## Existing escrow proof

The escrow-specific live proof and receipt remain available through
`scripts/demo-testnet.sh`; use `--verify-deployed` for a read-only check or
`--dry-run` for a credential-free plan.
