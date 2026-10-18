# Soroban Forge — Six-Contract Developer Console

A Next.js developer console providing interactive, representative read and write flows for all six Soroban Forge smart contracts on Stellar Testnet.

---

## Overview

The developer console provides interactive panels for every Forge contract with pre-flight client-side input validation, simulation inspection, non-custodial Freighter signing, submission tracking, decoded transaction events, and automatic `ForgeError` decoding.

### Non-Custodial & Testnet-Only Guardrails

- **Testnet Only:** Defaults to the Stellar Testnet RPC (`https://soroban-testnet.stellar.org`) and Testnet Passphrase (`Test SDF Network ; September 2015`).
- **No Key Custody:** The application never requests, stores, or handles private keys or seed phrases. All transaction signing is delegated to the user's Freighter browser extension via signer callback.
- **Pre-Flight Validation:** All write flows validate inputs (addresses, positive amounts, valid bounds, timeouts) *before* triggering simulation or prompting the wallet.

---

## Supported Contracts & Flows

| Contract | Category | Read Flows | Representative Safe Write Flow |
| :--- | :--- | :--- | :--- |
| **Escrow** | 3-Party Custody | `get_escrow(id)`<br/>`get_status(id)`<br/>`escrows_for_participant(addr)` | `create_escrow({ buyer, seller, arbiter, token, amount, timeout })` |
| **Vesting** | Linear + Cliff | `get_status(schedule_id)`<br/>`claimable(schedule_id)` | `create_schedule({ beneficiary, token, total_amount, cliff, duration })` |
| **Multi-Sig Wallet** | M-of-N Treasury | `get_tx_preview(tx_id)`<br/>`get_threshold()`<br/>`get_owners()`<br/>`get_tx(tx_id)` | `submit_withdrawal({ submitter, token, destination, amount })`<br/>`confirm({ signer, tx_id })` |
| **DAO Governance** | Proposal Voting | `get_proposal(id)`<br/>`get_bond_config()`<br/>`get_proposal_count()` | `vote({ proposal_id, voter, support })` |
| **Subscription Payments** | Recurring Billing | `get_subscription(id)`<br/>`get_subscription_count()`<br/>`is_provider_authorized(provider)` | `subscribe({ subscriber, provider, token, amount, period })` |
| **Marketplace Royalties** | EIP-2981 Splits | `get_royalty(collection)`<br/>`get_settlement_summary(collection)` | `set_royalty({ collection, recipient, bps })` |

---

## Transaction Lifecycle Pipeline

Every write interaction moves through a transparent 5-stage lifecycle visualizer:

1. **Validation:** Checks all parameter constraints (Stellar address checksum/format, positive stroop amounts, duration rules) prior to simulation or signing.
2. **Simulation:** Runs Soroban RPC pre-flight simulation to verify resource footprints and authorization trees.
3. **Signing:** Prompts the connected Freighter wallet to authorize the transaction.
4. **Submission:** Transmits the signed envelope to the Stellar Testnet RPC and reports the live transaction hash.
5. **Receipt & Event Decoding:** Displays ledger sequence confirmation, decoded event topics & values, or human-readable `ForgeError` codes (1–13) with recovery options.

### Decoded Forge Errors

On-chain errors from `crates/shared-utils/src/errors.rs` are mapped directly to human-friendly descriptions:

- `ForgeError #1 (Unauthorized)`: Caller not permitted to perform action
- `ForgeError #2 (NotFound)`: Entity does not exist
- `ForgeError #3 (InvalidInput)`: Arguments failed validation
- `ForgeError #4 (InsufficientFunds)`: Custody balance insufficient
- `ForgeError #5 (AlreadyInitialized)` / `#6 (NotInitialized)`: Initialization state mismatch
- `ForgeError #7 (DeadlineReached)`: Action attempted after deadline
- `ForgeError #8 (InsufficientAllowance)`: Required token allowance too low
- `ForgeError #9 (ArithmeticOverflow)`: Arithmetic bounds exceeded
- `ForgeError #10 (Custom)`: Contract-specific error
- `ForgeError #11 (TokenTransferFailed)`: SEP-41 token invocation failure
- `ForgeError #12 (ContractInvocationFailed)`: Target contract invocation reverted
- `ForgeError #13 (WithdrawalLimitExceeded)`: Rolling-window rate limit reached

---

## Quick Start

### 1. Prerequisites

- **Node.js**: `v20+` or `v22+`
- **Freighter Wallet**: Install the [Freighter browser extension](https://www.freighter.app/) and switch network to **Testnet** in settings.

### 2. Install Dependencies

```bash
npm ci
```

### 3. Run Development Server

```bash
npm run dev
```

Open [http://localhost:3000](http://localhost:3000) in your browser.

### 4. Build and Lint

```bash
npm run build
npm run lint
```

### 5. Run Unit Tests

Tests verify browser-independent request construction, parameter validators, event decoding, and `ForgeError` mapping:

```bash
npm test
```

---

## Contract ID Configuration

The console includes default testnet contract IDs for all six contracts and provides an in-app override input for each tab. Developers can point the console at their own testnet deployments without editing source code.
