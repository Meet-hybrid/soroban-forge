# Vercel deployment

Soroban Forge's Next.js example can be deployed to Vercel without publishing
the TypeScript SDK to npm. The repository-level `vercel.json` installs and
builds the local SDK first, then builds the app, so the SDK remains a private
workspace package.

## Dashboard setup

1. Import the `Meet-hybrid/soroban-forge` repository into Vercel.
2. Leave **Root Directory** set to the repository root (`./`).
3. Keep the Next.js framework preset. The repository `vercel.json` supplies
   the install, build, and output settings.
4. Deploy.

The app connects to the configured Stellar testnet escrow contract and uses
Freighter in the visitor's browser. No secret environment variables are
currently required. The deployed app is a testnet demo; it does not represent
an audited or mainnet deployment.

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
