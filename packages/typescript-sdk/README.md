# @soroban-forge/escrow-client (Soroban Forge TypeScript SDK)

Multi-contract TypeScript SDK workspace for **Soroban Forge** smart contracts on Stellar.

Provides dedicated, fully-typed client wrappers and network configurations for all contracts in the repository:
- **escrow** (deployed on Stellar testnet: `CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB`)
- **subscription-payments**
- **multi-sig-wallet**
- **marketplace-royalties**
- **dao-governance**
- **vesting**

The SDK features a clean boundary between raw generated Stellar contract bindings (`src/generated/`), typed client modules with network configs (`src/contracts/`), and fixture-driven drift tests (`src/tests/` & `src/fixtures/`).

## Installation

```bash
npm install
```

## Build

Compile the TypeScript source to `dist/`:

```bash
npm run build
```

The compiled output in `dist/` is what consumers import. `npm run build` compiles all client wrappers and generated bindings in a single unified pass.

## Usage

### 1. Subpath Imports (Recommended)

Each contract is accessible via dedicated subpath exports:

```ts
import { Client as EscrowClient, networks as escrowNetworks } from "@soroban-forge/escrow-client/escrow";
import { Client as MultiSigClient, networks as multiSigNetworks } from "@soroban-forge/escrow-client/multi-sig-wallet";
import { Client as SubscriptionClient, networks as subscriptionNetworks } from "@soroban-forge/escrow-client/subscription-payments";
import { Client as RoyaltiesClient, networks as royaltiesNetworks } from "@soroban-forge/escrow-client/marketplace-royalties";
import { Client as GovernanceClient, networks as governanceNetworks } from "@soroban-forge/escrow-client/dao-governance";
import { Client as VestingClient, networks as vestingNetworks } from "@soroban-forge/escrow-client/vesting";

// Instantiate escrow client:
const escrowClient = new EscrowClient({
  ...escrowNetworks.testnet,
  rpcUrl: "https://soroban-testnet.stellar.org",
});

// Instantiate multi-sig wallet client:
const multiSigClient = new MultiSigClient({
  ...multiSigNetworks.testnet,
  rpcUrl: "https://soroban-testnet.stellar.org",
});
```

### 2. Top-Level Namespace Imports

Import all contract modules through top-level namespaces:

```ts
import { escrow, multiSigWallet, subscriptionPayments } from "@soroban-forge/escrow-client";

const client = new escrow.Client({
  ...escrow.networks.testnet,
  rpcUrl: "https://soroban-testnet.stellar.org",
});
```

### 3. Backward-Compatible Root Imports (Escrow)

Existing escrow consumers (such as `packages/nextjs-example`) can import directly from the package root:

```ts
import { Client, networks, type EscrowData } from "@soroban-forge/escrow-client";

const client = new Client({
  ...networks.testnet,
  rpcUrl: "https://soroban-testnet.stellar.org",
});
```

## Network Configuration and Placeholder Contract IDs

The `networks.testnet` configuration object exposes:
- `networkPassphrase`: `"Test SDF Network ; September 2015"`
- `contractId`:
  - For **escrow**: the deployed testnet contract ID (`CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB`).
  - For undeployed contracts: valid, deterministic placeholder IDs that can be overridden via environment variables:
    - `SUBSCRIPTION_PAYMENTS_CONTRACT_ID`
    - `MULTI_SIG_WALLET_CONTRACT_ID`
    - `MARKETPLACE_ROYALTIES_CONTRACT_ID`
    - `DAO_GOVERNANCE_CONTRACT_ID`
    - `VESTING_CONTRACT_ID`

## Shared Error Registry (`ForgeError`)

Shared contract error codes (1–13) defined across all contracts in `crates/shared-utils/src/errors.rs` are exported centrally:

```ts
import { FORGE_ERRORS, forgeErrorName, ForgeError } from "@soroban-forge/escrow-client";

console.log(FORGE_ERRORS[1]); // "Unauthorized"
console.log(forgeErrorName(4)); // "InsufficientFunds"
```

## Offline Test Suite & Drift Detection

The test suite runs completely offline without network access or testnet credentials:

```bash
npm test
```

The test runner executes:
1. **Escrow Client Suite** (`src/tests/escrow-client.test.ts`): All 25 original test cases covering construction, methods, fromJSON, arg encoding, events, and compile-time types.
2. **Fixture-Driven Drift Tests** (`src/tests/drift.test.ts`): Validates contract client specs (function names, parameter names and types, and event definitions) across all contracts against checked-in JSON fixtures in `src/fixtures/`.
3. **Centralized Error Suite** (`src/tests/forge-errors.test.ts`): Verifies error codes 1–13, `FORGE_ERRORS`, `forgeErrorName`, and runtime `ForgeError`.

## Client & Fixture Regeneration

To deterministically regenerate client bindings and fixtures:

```bash
# From workspace root:
bash scripts/generate-clients.sh

# Or from packages/typescript-sdk:
npm run regen
```

To regenerate only the JSON expectation fixtures from current bindings:

```bash
npm run regen:fixtures
```

Checked-in fixtures are stored under `src/fixtures/<contract>.fixture.json`. If a contract's public ABI or events change, running `npm test` will catch the drift unless the fixtures are deliberately regenerated.

## Optional Testnet Verification

```bash
# Verify testnet read call against live deployed escrow:
ESCROW_ID=1 npm run verify:testnet
```
