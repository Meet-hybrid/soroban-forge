# PRD: #208: feat(typescript-sdk): multi-contract SDK workspace with fixture-based drift tests

## 1. Overview
`packages/typescript-sdk` currently only exports a single generated client for the escrow contract (`@soroban-forge/escrow-client`) and uses hand-maintained arrays in its test suite for drift detection. As the repository has expanded to six Soroban contracts (escrow, subscription-payments, marketplace-royalties, multi-sig-wallet, dao-governance, and vesting), new contract features currently trigger manual test updates or allow client drift to go undetected until late in CI.

This feature converts `packages/typescript-sdk` into a multi-contract TypeScript SDK workspace providing:
1. Dedicated client wrappers for each contract with their own networks and configuration surfaces.
2. A clean architectural separation between generated bindings (`src/generated/`) and handwritten client logic/glue.
3. Fixture-based drift tests utilizing checked-in JSON fixtures representing contract function signatures, parameters, types, and events.
4. A centralized ForgeError test suite eliminating duplicate error tables.
5. Unified `npm run build` and `npm test` covering all clients, preserving existing CI workflows.
6. A deterministic regeneration workflow producing byte-stable output from contract build artifacts.

## 2. Core Decisions & Architecture
### 2.1 ABI Source & Determinism
- **Decision**: Built WASM artifacts (`target/wasm32v1-none/release/*.wasm`) are the authoritative ABI source of truth.
- **Rationale**: Enables fully offline generation and testing without requiring live testnet credentials, matching CI constraints and the repository's provenance model.
- **Contract IDs**: The deployed testnet ID is embedded for escrow (`CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB`). Contracts without current testnet deployments receive valid deterministic placeholder IDs (configurable via environment variables) allowing offline client instantiation and testing.

### 2.2 Package Layout & Export Strategy
- **Decision**: Hybrid export layout supporting both subpath exports (`./escrow`, `./subscription-payments`, `./multi-sig-wallet`, etc.) and top-level namespace exports, while retaining backward compatibility for `@soroban-forge/escrow-client` and `packages/nextjs-example`.
- **Layout Structure**:
  ```text
  packages/typescript-sdk/
  ├── package.json
  ├── tsconfig.json
  ├── tsconfig.test.json
  ├── src/
  │   ├── index.ts                     # Root exports (namespaces + shared errors + backward-compat escrow)
  │   ├── errors.generated.ts          # Generated ForgeError registry
  │   ├── contracts/
  │   │   ├── escrow/
  │   │   ├── subscription-payments/
  │   │   ├── multi-sig-wallet/
  │   │   ├── marketplace-royalties/
  │   │   ├── dao-governance/
  │   │   └── vesting/
  │   ├── generated/                   # Pinned generated contract bindings from WASM
  │   ├── fixtures/                    # Checked-in JSON ABI/event expectation fixtures
  │   └── tests/
  │       ├── escrow-client.test.ts    # 25 adapted escrow tests
  │       ├── drift.test.ts            # Fixture-based drift tests across contracts
  │       └── forge-errors.test.ts     # Centralized ForgeError tests (codes 1–13)
  ```

### 2.3 Fixture Format & Drift Architecture
- **Decision**: JSON fixtures (`src/fixtures/<contract>.fixture.json`) checked into version control.
- **Structure**:
  ```json
  {
    "contract": "escrow",
    "functions": [
      {
        "name": "create_escrow",
        "inputs": [
          { "name": "buyer", "type": "scSpecTypeAddress" },
          ...
        ]
      }
    ],
    "events": ["EscrowCreated", "Deposited", ...]
  }
  ```
- **Drift Test**: Compares the instantiated `client.spec.funcs()` and `client.spec.events()` against the fixture JSON. Any contract ABI change without regenerating the fixture triggers an immediate, clear failure in `npm test`.

## 3. Detailed Requirements & Acceptance Criteria
1. **Multi-Contract Support**: Clients generated for escrow, subscription-payments, and multi-sig-wallet (plus marketplace-royalties, dao-governance, vesting), each exposing typed clients and `networks`.
2. **Fixture-Based Tests**: Hand-written arrays replaced with assertions loaded from `src/fixtures/`.
3. **Preserved Escrow Suite**: All 25 original test assertions for escrow pass in adapted form.
4. **Centralized Error Testing**: ForgeError codes (1–13) tested in `src/tests/forge-errors.test.ts`.
5. **Single Build & Test**: `npm run build` and `npm test` in `packages/typescript-sdk` compile and test all clients; `ci.yml` requires no modifications.
6. **Deterministic Regeneration**: `scripts/generate-clients.sh` (or `npm run regen`) executes reproducibly and produces a clean git diff on the existing tree.
7. **Ecosystem Compatibility**: `packages/nextjs-example` typechecks cleanly.
8. **Clean Rust & CI Gate**: Full workspace cargo checks (`fmt`, `clippy -D warnings`, `test`) remain green.