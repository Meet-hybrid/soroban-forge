# Vercel deployment

Soroban Forge's Next.js example can be deployed to Vercel without publishing
the TypeScript SDK to npm. The repository-level `vercel.json` installs and
builds the local SDK first, then builds the app, so the SDK remains a private
workspace package.

> [!WARNING]
> **Testnet-Only & Unaudited Demonstration**
> The deployed example application is strictly intended for experimentation on
> **Stellar Testnet**. The smart contracts, client bindings, and example UI are
> **unaudited**. It must **not** be deployed or used with mainnet credentials,
> mainnet RPC endpoints, or real assets. Mainnet support is explicitly out of scope.

## Dashboard setup

1. Import the `Meet-hybrid/soroban-forge` repository into Vercel.
2. Leave **Root Directory** set to the repository root (`./`).
3. Keep the Next.js framework preset. The repository `vercel.json` supplies
   the install, build, and output settings.
4. Deploy.

The app supports two execution modes:
- **Demo / Local Mode**: Fully simulated in-memory contract execution for testing
  UI and escrow state transitions without a browser wallet or network latency.
- **Live Testnet Mode**: Connects to the deployed Stellar Testnet escrow contract
  (`CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB`) using Freighter
  or any Stellar Wallets Kit connector.

No secret environment variables are required.

## Execution modes and state indicators

Visitors can distinguish simulated actions from live on-chain operations:

1. **Environment Indicators**:
   - The UI top bar displays an active mode badge (`DEMO / LOCAL` vs `LIVE / TESTNET`).
   - Each action card, input form, and participant listing displays a contextual
     badge indicating whether the target is in-memory local state or live contract RPC.
   - Activity log entries are explicitly tagged `[DEMO]` or `[LIVE]`.

2. **Transaction Lifecycle States**:
   - Every transaction transitions through distinguishable stages:
     - `1. Simulate`: Verifies contract preconditions and parameter validation.
     - `2. Sign`: Requests authorization (simulated key in Demo mode; browser wallet in Live mode).
     - `3. Submit`: Submits transaction envelope (in-memory commit or Stellar Testnet RPC broadcast).
     - `4. Status`: Shows a distinguishable `Confirmed` (with transaction hash and explorer link)
       or `Failed` card (with actionable error messages).

3. **Prerequisites for Live Transactions**:
   - The UI features a real-time prerequisites checklist for live transactions:
     - Browser wallet connector initialized.
     - Compatible wallet installed (Freighter, xBull, Albedo, Bitget).
     - Network configured to **Stellar Testnet** (`Test SDF Network ; September 2015`).
     - Funded account with testnet XLM (one-click Friendbot button provided).

## Why npm publication is optional

The SDK is currently consumed by the example app through the local
`file:../typescript-sdk` dependency. That is sufficient for the Vercel build
because the repository root is the build context.

Publishing `@soroban-forge/escrow-client` to npm is a separate distribution
step. It is only needed if external developers should install the SDK directly
with:

```bash
npm install @soroban-forge/escrow-client
```

Deploying the app does not modify GitHub issues, labels, comments, or project
history.
