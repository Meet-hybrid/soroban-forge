# Soroban Forge — Next.js Escrow Explorer

An interactive web application showcasing escrow lifecycle operations on
Stellar smart contracts built with Soroban Forge and `@soroban-forge/escrow-client`.

> [!WARNING]
> **Testnet-Only & Unaudited Demonstration**
> This example application is intended strictly for developer exploration and
> testing on **Stellar Testnet**. The contracts, TypeScript client libraries,
> and web interface are **unaudited**.
>
> **Do not use this example with Stellar Mainnet keys, accounts, or real funds.**
> Mainnet support is explicitly out of scope.

---

## Key Features

### 1. Distinct Execution Environments (Demo vs Live)

Users can toggle between two modes using the top-bar switcher:

| Feature | Demo / Local Mode (`demo`) | Live Testnet Mode (`testnet`) |
| :--- | :--- | :--- |
| **Execution Target** | In-browser memory store | Stellar Testnet (`CC227UDF…`) |
| **Wallet Requirement** | None (simulated demo keys) | Freighter, xBull, Albedo, Bitget |
| **Network RPC** | None (offline simulation) | `https://soroban-testnet.stellar.org` |
| **Network Passphrase** | N/A | `Test SDF Network ; September 2015` |
| **Gas / Fees** | Free / simulated balance | Testnet XLM (via Friendbot) |
| **UI Indicators** | Purple `[DEMO / LOCAL]` badges | Emerald `[LIVE / TESTNET]` badges |
| **Log Prefixes** | `[DEMO]` | `[LIVE]` |

### 2. Explicit Transaction State Machine

All contract actions follow a visible, 4-stage lifecycle:

1. **`simulating`**: Pre-flight validation against contract rules and resource budgets.
2. **`signing`**: Cryptographic authorization request (local simulated key in Demo mode; browser wallet window prompt in Live mode).
3. **`submitting`**: Transaction broadcast to the target environment.
4. **`confirmed` / `failed`**:
   - **Confirmed**: Displays transaction hash, summary, and direct link to StellarExpert Testnet explorer.
   - **Failed**: Displays parsed contract errors and actionable resolution steps.

### 3. Live Testnet Prerequisites Checklist

When in Live mode, a dedicated checklist verifies:
- **Connector readiness**: Stellar Wallets Kit initialized in browser context.
- **Account connection**: Wallet extension connected and authorized.
- **Network target**: Stellar Testnet (`Test SDF Network ; September 2015`).
- **Account funding**: Sufficient testnet XLM for fees (includes one-click Friendbot button).
- **Asset trustline**: SEP-41 testnet demo token (`CBJQ53EOHB…`).

---

## Getting Started

### Prerequisites

- Node.js >= 20
- Built TypeScript SDK in `packages/typescript-sdk`

### Build the SDK First

From the repository root:

```bash
npm --prefix packages/typescript-sdk run build
```

### Install and Run the Example

```bash
cd packages/nextjs-example
npm install
npm run dev
```

Open [http://localhost:3000](http://localhost:3000) in your browser.

---

## Testing & Verification

The state machine, environment switching, prerequisite evaluation, and escrow lifecycle transitions are covered by deterministic tests using Node's native test runner:

```bash
npm test
```

To run linting and production build checks:

```bash
npm run lint
npm run build
```
