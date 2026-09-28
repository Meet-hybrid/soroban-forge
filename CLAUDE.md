# Task Workspace Context Protocol

## Workspace Sandbox Root
`/Users/fangqq/.bounty_agent_platform/workspaces/gh-5608024056`

## Task Info
- Repository: Meet-hybrid/soroban-forge
- Issue: #208
- Title: feat(typescript-sdk): multi-contract SDK workspace with fixture-based drift tests

## Git Branch Protocol
- Feature Branch: `feat/issue-208`
- Target Base Branch: `main`

## Issue Description


## Step 2 PRD Specifications
- Root Cause: packages/typescript-sdk is currently structured as an isolated, single-contract package for escrow (@soroban-forge/escrow-client) with hand-coded assertions for ABI methods, parameters, types, error codes, and events. The other five contracts in the workspace have either unmaintained standalone packages without drift test suites or lack integrated SDK export coverage. Furthermore, tsconfig.json lacks an explicit rootDir setting (causing compiler failures under modern TypeScript), test assertions rely on brittle manual array declarations rather than checked-in fixture expectations, and client generation wipes directories without cleanly separating generated bindings from client configurations and shared error structures.
- Proposed Fix: Restructure packages/typescript-sdk into a unified multi-contract SDK supporting escrow, subscription-payments, multi-sig-wallet, marketplace-royalties, dao-governance, and vesting. Establish a src/generated/ boundary for machine-generated Stellar contract bindings, pair each with a typed client wrapper exposing networks and configuration (with deployed IDs for testnet contracts and deterministic placeholder IDs for undeployed contracts), and export them via both subpath exports and top-level namespace exports. Implement a fixture-driven drift test suite that reads generated, checked-in JSON fixtures to validate ABI function surfaces, parameter names and types, and event definitions for all contracts, while centralizing ForgeError verification into a single test suite. Update tsconfig.json, tsconfig.test.json, and package.json scripts to ensure npm run build and npm test cover all clients in one pass, update scripts/generate-clients.sh to deterministically regenerate bindings and fixtures, and document the architecture and workflows in packages/typescript-sdk/README.md and docs/DEVELOPMENT.md.
- Test Command/Strategy: cargo test

## Demand-to-Code Mapping
- Requirement: Per-contract clients generated for escrow + at least two more event-emitting contracts with networks/config, building under one tsc project
  Affected: ["packages/typescript-sdk/src/index.ts", "packages/typescript-sdk/src/contracts/*", "packages/typescript-sdk/src/generated/*", "packages/typescript-sdk/tsconfig.json", "packages/typescript-sdk/package.json"]
  Action: Restructure packages/typescript-sdk/src to house per-contract client modules (escrow, subscription-payments, multi-sig-wallet, marketplace-royalties, dao-governance, vesting) with distinct networks/config objects and export them from the SDK root and subpath exports.
- Requirement: Fixture-based drift tests with generated-checked-in fixtures rather than hand-maintained arrays
  Affected: ["packages/typescript-sdk/src/fixtures/*.json", "packages/typescript-sdk/src/tests/drift.test.ts", "packages/typescript-sdk/src/tests/escrow-client.test.ts", "scripts/generate-fixtures.ts"]
  Action: Create checked-in JSON fixtures generated directly from contract WASM/Spec artifacts representing expected functions, parameter names, types, and event names. Refactor drift tests to assert against these fixtures.
- Requirement: The escrow client's current 25 tests pass in adapted form without weakening assertions
  Affected: ["packages/typescript-sdk/src/tests/escrow-client.test.ts"]
  Action: Adapt the 25 escrow test cases to import from the restructured client module and fixture-backed drift assertions without weakening any assertions (client construction, networks, all 11 methods, fromJSON, arg encoding, events, and compile-time type checks).
- Requirement: npm run build / npm test cover all clients in one run; ci.yml runs unchanged
  Affected: ["packages/typescript-sdk/package.json", "packages/typescript-sdk/tsconfig.json", "packages/typescript-sdk/tsconfig.test.json", ".github/workflows/ci.yml"]
  Action: Fix tsconfig.json with explicit rootDir and include glob, update package.json test script to run all test files in dist-test/tests/**/*.test.js, ensuring ci.yml executes without modification.
- Requirement: Documented, deterministic regeneration procedure producing a clean diff
  Affected: ["scripts/generate-clients.sh", "packages/typescript-sdk/scripts/regen-fixtures.ts", "packages/typescript-sdk/package.json"]
  Action: Update or provide a regeneration script (e.g. scripts/generate-clients.sh and npm run regen in typescript-sdk) that builds contract WASMs, generates TypeScript bindings into src/generated/, and extracts ABI/event fixtures deterministically.
- Requirement: ForgeError shared table tested once, centrally (codes 1-13)
  Affected: ["packages/typescript-sdk/src/tests/forge-errors.test.ts", "packages/typescript-sdk/src/tests/escrow-client.test.ts"]
  Action: Extract ForgeError testing into a dedicated test file (forge-errors.test.ts) that verifies error codes 1-13 and their mappings against errors.json or generated constants once centrally.
- Requirement: Restructure import paths reflected in README and dependent packages (nextjs-example) still typecheck
  Affected: ["packages/typescript-sdk/README.md", "docs/DEVELOPMENT.md", "packages/nextjs-example/package.json", "packages/nextjs-example/src/app/page.tsx"]
  Action: Maintain backward-compatible re-exports or update package exports and subpaths so nextjs-example continues to typecheck, and update README.md and DEVELOPMENT.md to document multi-contract usage.
- Requirement: Full CI matrix stays green (Rustfmt, Clippy, Build, Test, WASM size, Provenance)
  Affected: ["Cargo.toml", "crates/*"]
  Action: Ensure zero breaking contract changes or unformatted Rust files so cargo fmt, clippy, test, and provenance jobs pass seamlessly.

## Actionable Task Checklist
1. Task 1: Resolve three core architectural decisions: (1) ABI source = built contract WASMs (wasm32v1-none) for deterministic offline builds; (2) Package layout = subpath exports (e.g. @soroban-forge/sdk/escrow, /subscription-payments, /multi-sig-wallet) plus top-level namespace re-exports with backward-compatible root exports for escrow; (3) Fixture format = checked-in JSON fixtures for each contract (functions, inputs, types, events).
2. Task 2: Fix packages/typescript-sdk/tsconfig.json and tsconfig.test.json (explicit rootDir: './src', updated include patterns) and update package.json with subpath exports and multi-client test script.
3. Task 3: Establish src/generated/ boundary containing raw bindings for escrow, subscription-payments, multi-sig-wallet, marketplace-royalties, dao-governance, and vesting.
4. Task 4: Create per-contract client wrapper modules under src/contracts/ (or per-contract directories) encapsulating Client, networks/config (testnet deployed ID for escrow, placeholder/env-driven IDs for others), types, and re-exports.
5. Task 5: Create deterministic fixture generation tool/script that parses ContractSpec to output JSON fixtures of function names, inputs, types, and event names into src/fixtures/.
6. Task 6: Implement fixture-based drift tests for escrow and other event-emitting contracts (subscription-payments, multi-sig-wallet) comparing live ContractSpec against checked-in fixtures.
7. Task 7: Refactor escrow-client.test.ts to preserve all 25 test cases in adapted form and extract shared ForgeError tests into src/tests/forge-errors.test.ts.
8. Task 8: Update scripts/generate-clients.sh and typescript-sdk scripts to provide an end-to-end, deterministic regeneration command, and verify dry-run produces a clean diff.
9. Task 9: Verify nextjs-example typechecks and builds with the updated SDK package layout.
10. Task 10: Update packages/typescript-sdk/README.md and docs/DEVELOPMENT.md with multi-contract usage, subpath imports, and regeneration documentation.
11. Task 11: Execute complete local verification: npm test & npm run build in typescript-sdk, and full cargo workspace gate.

## Strict Guidelines
1. All modifications MUST be made inside `/Users/fangqq/.bounty_agent_platform/workspaces/gh-5608024056`.
2. Implement complete code with ZERO stubs or placeholders.
3. Run `cargo test` to verify all tests pass.
