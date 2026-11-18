# Soroban Forge Next.js example

The example app under `src/app` exercises the escrow TypeScript client against
the configured testnet contract.

## Local demo state

The `Local demo state` panel controls browser-only form values used while
testing the demo:

- `soroban-forge/demo-state@1` is the versioned JSON schema.
- `soroban-forge:nextjs-demo-state:v1` is the `localStorage` key.
- Export produces a JSON document with `schema`, `kind`, `version`, `label`,
  `exportedAt`, and `state`.
- `state` is `null` when there is no saved demo state.

Exports contain only local form values. They do not include wallet keys,
signed transaction payloads, chain results, network configuration, or contract
state. The UI labels exported data as local browser demo state.

Stored JSON and exported documents are validated before use. Unknown fields,
malformed values, secret-looking values, a different schema, or a different
version are rejected. Invalid stored data is removed and the demo falls back to
defaults.

`Reset local demo state` requires confirmation. It clears the demo form state
and restores defaults. It does not disconnect wallets, change the network or
contract, or modify on-chain data.

## Development

```sh
npm install
npm run dev
```

Verification:

```sh
npm run lint
npm run test
npm run build
```
