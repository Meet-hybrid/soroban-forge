# Deployment and storage compatibility

## Deploy (testnet → mainnet)

1. Install the Stellar CLI and configure a network (`testnet` first).
2. Fund the deployer account on testnet.
3. Build each contract crate to WASM with the repository's standard release profile.
4. Deploy with `stellar contract deploy` (or the project wrapper scripts if present).
5. Record the contract IDs and init/admin parameters used.

## Verify deployed WASM

- Compare the on-chain WASM hash with your local build artifact.
- Smoke-call a read-only entrypoint after deploy.

## Storage layout

Each contract crate owns its storage keys (usually under `src/` as `DataKey` / similar enums).
When changing storage:

- Additive optional keys are generally upgrade-safe.
- Renaming, retyping, or reusing key discriminants is a **breaking** storage change.
- Breaking changes need a migration path or a new contract instance.

## Upgrade compatibility checklist

- [ ] No existing key discriminant reused for a different type
- [ ] New fields are optional or defaultable
- [ ] Migration/tests cover read of pre-upgrade data
