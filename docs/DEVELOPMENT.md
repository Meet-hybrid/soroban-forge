# Development Guide

This guide covers the development workflow for Soroban Forge. All commands documented here are tested and work in the current repository.

## Prerequisites

### Developer CLI (soroban-forge)

Build and run the developer CLI:

```bash
# Build CLI binary
cargo build --locked -p soroban-forge-cli

# Scaffold a new Soroban contract crate
cargo run -p soroban-forge-cli -- new my-token

# Scaffold with custom path
cargo run -p soroban-forge-cli -- new my-token --path ./custom/path/my-token
```

The generated crate is a standalone project that compiles with `cargo check`
without depending on the workspace layout or a stale shared-utils version pin.

Run the contributor preflight before opening a pull request or deploying a contract:

```bash
# Check the expected develop base, stellar, cargo, and the wasm32v1-none target
cargo run -p soroban-forge-cli -- doctor

# Also verify that a selected artifact exists and is non-empty
cargo run -p soroban-forge-cli -- doctor \
  --wasm target/wasm32v1-none/release/soroban_forge_escrow.wasm
```

`doctor` identifies `develop` as the expected PR base branch and never installs tools or performs network checks. It runs every check in order and prints
PASS/FAIL rows plus an exact remediation command for each failure. `stellar --version` and
`cargo --version` are reported when available; if an existing binary returns a non-zero status or
no usable version, the check remains PASS with `version unknown`. The target check only passes when
`rustup target list --installed` succeeds and includes `wasm32v1-none`. The optional artifact check
distinguishes a missing path, directory, empty file, and filesystem error. The command exits `0`
only when every required check passes; any required failure exits non-zero and lists the failed
checks in the summary.

Validate a transaction plan before signing or submitting it:

```bash
# Human-readable report
cargo run -p soroban-forge-cli -- validate-plan \
  --plan crates/cli/fixtures/plans/valid-create-escrow.json \
  --expected-network testnet

# Machine-readable report (schema soroban-forge/tx-plan-validation-report@1)
cargo run -p soroban-forge-cli -- validate-plan \
  --plan crates/cli/fixtures/plans/invalid-multi-error.json \
  --json
```

`validate-plan` (alias `plan`) accepts JSON or TOML plans and reports all independent findings
together. The stable JSON categories are `missing_required_field`, `network_mismatch`,
`invalid_contract_id`, `unsupported_method`, `malformed_argument`, `sensitive_input_rejected`, and
`schema_mismatch`. Text output includes the category, field, message, and a remediation hint.
Secrets are never echoed; suspected secret values are reported as `[REDACTED]`. Valid plans exit
with code `0` and invalid plans exit with code `1`.

```bash
# Install or update Rust
rustup update stable

# Install required components
rustup component add rustfmt clippy

# Add WASM target required by soroban-sdk
rustup target add wasm32v1-none
```

### Stellar CLI

Required to build contracts to WASM (soroban-sdk 29 rejects plain
`cargo build --target wasm32v1-none`), and for deployment and contract
interaction. Use v25.2.0 or newer:

```bash
cargo install --locked stellar-cli
```

### soroban-sdk Version

The project uses soroban-sdk 27.x. See `rust-toolchain.toml` for the exact pinned version.

## Clone and Setup

```bash
# Clone the repository
git clone https://github.com/Meet-hybrid/soroban-forge.git
cd soroban-forge

# Verify workspace loads
cargo metadata --locked --no-deps --format-version 1 > /dev/null
```

## Build Commands

### Full Workspace Build

```bash
make build
# or: cargo build --workspace --all-targets --locked
```

### Release Build (WASM Artifacts)

```bash
cargo build --locked --release --target wasm32v1-none -p soroban-forge-escrow
# Builds: target/wasm32v1-none/release/soroban_forge_escrow.wasm
```

### Build All Contracts for Release

```bash
cargo build --locked --release --target wasm32v1-none \
  --package soroban-forge-escrow \
  --package soroban-forge-vesting \
  --package soroban-forge-multi-sig-wallet \
  --package soroban-forge-dao-governance \
  --package soroban-forge-subscription-payments \
  --package soroban-forge-marketplace-royalties
```

### Build and Check WASM with the Developer CLI

The CLI can build one contract or all six contract packages for the required
`wasm32v1-none` target. It runs
`stellar contract build --locked --profile release --optimize=false`, the same
command CI and `scripts/provenance.sh` use, so its artifacts hash the same as
theirs. This needs stellar-cli v25.2.0 or newer on `PATH`: soroban-sdk rejects
a plain `cargo build --target wasm32v1-none`. Use `--package` to select a single
contract; without it, the CLI builds all contract packages and skips the
host-only CLI and shared utility crates.

```bash
# Build all contract WASM artifacts
cargo run -p soroban-forge-cli -- build --wasm

# Build one contract and verify its artifact against the 150,000-byte budget
cargo run -p soroban-forge-cli -- build --wasm --check-size \
  --package soroban-forge-escrow

# Build all contracts and print a size table; --check-size implies --wasm
cargo run -p soroban-forge-cli -- build --check-size
```

Size verification inspects `.wasm` files under
`target/wasm32v1-none/release/` (or `$CARGO_TARGET_DIR/wasm32v1-none/release/`
when `CARGO_TARGET_DIR` is set). It reports each artifact's byte and KB size,
the 150,000-byte budget, and a `PASS` or `FAIL` status. If any artifact exceeds
the budget, or no artifact is found for the selected package, the command exits
with an error. If the target is missing, install it with
`rustup target add wasm32v1-none`; if the Stellar CLI is missing, install it with
`cargo install --locked stellar-cli`; then rerun the build.

Example output:

```text
Contract                                  Size (bytes)   Size (KB)   Budget (bytes)  Status
----------------------------------------  ------------  ----------  ---------------  ------
soroban_forge_dao_governance.wasm                27188       26.55           150000  PASS
soroban_forge_escrow.wasm                        45850       44.77           150000  PASS
soroban_forge_marketplace_royalties.wasm         26330       25.71           150000  PASS
soroban_forge_multi_sig_wallet.wasm              39651       38.72           150000  PASS
soroban_forge_subscription_payments.wasm         36643       35.78           150000  PASS
soroban_forge_vesting.wasm                       29836       29.13           150000  PASS
```

### Verify WASM Artifacts against the Provenance Manifest

`provenance-manifest.json` (generated by `scripts/provenance.sh`) records the
lowercase SHA-256 of every contract artifact built from a clean rebuild. The
`verify` subcommand rebuilds a package with the same `stellar contract build`
command as `scripts/provenance.sh` and compares the result against a supplied
artifact, an expected hash, or the manifest.

```bash
# Compare a local artifact with a fresh rebuild of its inferred package
cargo run -p soroban-forge-cli -- verify --wasm target/wasm32v1-none/release/soroban_forge_escrow.wasm

# Rebuild a package and check its fresh artifact matches an expected hash
cargo run -p soroban-forge-cli -- verify --expected <64-hex-sha256> --package soroban-forge-escrow

# Check every artifact in the manifest (the default path is provenance-manifest.json)
cargo run -p soroban-forge-cli -- verify --manifest provenance-manifest.json
```

With `--wasm`, `--package` can override package inference from the artifact
name, and `--expected` can require both hashes to match a known value.
With `--expected` alone and no package, the command checks every contract.

For offline release automation, run from the source checkout with the required
Rust toolchain, WASM target, Stellar CLI, and Cargo dependencies already installed
or cached. Use a local copy of the release manifest. No credentials are required.

```bash
CARGO_NET_OFFLINE=true soroban-forge verify --manifest provenance-manifest.json --json > verification.json
```

The command writes one JSON object to stdout with an overall boolean `ok` and an
`artifacts` array in manifest order. Each entry has `name`, `status`,
`expected_sha256`, `actual_sha256`, `expected_bytes`, `actual_bytes`, and
`size_drift`. Status is `match`, `mismatch` (hash), `size_drift` (matching hash
with different size), or `missing`. If both hash and size differ, the status is
`mismatch` and `size_drift` is true. Unavailable hash or size fields are null.
Artifact failures still produce the report and exit nonzero. Input or build
errors exit nonzero with a diagnostic on stderr.

### Decode Failed Invocations

The `decode-failure` helper explains raw Stellar CLI/RPC errors or decodes a
base64 `TransactionResult` receipt. Include the receipt's optional
`DiagnosticEvents` XDR to reveal Soroban contract or host error details:

```bash
# Explain the error text printed by stellar contract invoke
cargo run -p soroban-forge-cli -- decode-failure \
  --error 'HostError: Error(Contract, #3)'
# Soroban contract error #3: ForgeError::InvalidInput. Check the contract's preconditions and authorization requirements.

# Decode a failed transaction result and its diagnostic events
cargo run -p soroban-forge-cli -- decode-failure \
  --result-xdr '<base64-transaction-result-xdr>' \
  --diagnostic-events-xdr '<base64-diagnostic-events-xdr>'
```

Contract codes 1–15 are named using the shared `ForgeError` registry; unknown
contract codes are reported without guessing their meaning. Host traps without
diagnostic events are identified as host aborts, but cannot be attributed to a
specific contract error. Recognizable connection and RPC failures are reported
as transport issues. Unrecognized error text and malformed XDR produce a clear
diagnostic rather than being treated as a successful decode. The helper does
not retry the invocation or change the behavior of `invoke`.

## Test Commands

### Full Test Suite

```bash
make test
# or: cargo test --workspace --all-targets --locked
```

### Test Individual Contract

```bash
# Escrow
cargo test --workspace --package soroban-forge-escrow --locked

# Vesting
cargo test --workspace --package soroban-forge-vesting --locked

# Multi-Sig Wallet
cargo test --workspace --package soroban-forge-multi-sig-wallet --locked

# DAO Governance
cargo test --workspace --package soroban-forge-dao-governance --locked

# Subscription Payments
cargo test --workspace --package soroban-forge-subscription-payments --locked

# Marketplace Royalties
cargo test --workspace --package soroban-forge-marketplace-royalties --locked
```

## Lint and Format

### Code Formatting

```bash
make format
# or: cargo fmt --all
```

### Format Check (CI)

```bash
make format-check
# or: cargo fmt --all -- --check
```

### Linting

```bash
make lint
# or: cargo clippy --workspace --all-targets --locked -- -D warnings
```

### Security Audit

```bash
make audit
# or: cargo audit
```

## Documentation

### Generate Rust Documentation

```bash
make doc
# or: cargo doc --workspace --no-deps --locked --document-private-items
```

### Run Full Release Checks

```bash
make release
# Runs: format, lint, audit, test, build-release
```

## Repository Commands Reference

| Command        | Purpose                 |
| -------------- | ----------------------- |
| `make build`   | Build workspace         |
| `make test`    | Run all tests           |
| `make format`  | Format code             |
| `make lint`    | Run clippy              |
| `make audit`   | Check dependencies      |
| `make doc`     | Generate docs           |
| `make clean`   | Clean build artifacts   |
| `make release` | Full pre-release checks |

## TypeScript Clients

All six contracts ship generated TypeScript bindings under `packages/*-client`
(`@soroban-forge/escrow-client` lives in `packages/typescript-sdk`, the five
newer contracts each have their own package directory). The bindings are
produced **offline** from each contract's WASM — no deployed contract id is
needed — using the pinned Stellar CLI's `contract bindings typescript --wasm`.

Regenerate everything with a single command:

```bash
bash scripts/generate-clients.sh
```

The script:

1. Builds all six contract crates for `wasm32v1-none` (`--locked --release`).
2. For each contract, wipes its package directory and runs
   `stellar contract bindings typescript --wasm <wasm> --output-dir <pkg> --overwrite`.
3. Leaves the generated output ready to commit; consumers do not need the
   Soroban toolchain to install or build the packages.

Build a client package (or the Next.js demo that depends on the escrow client):

```bash
cd packages/typescript-sdk && npm install && npm run build
cd packages/nextjs-example && npm install && npm run build
```

The mapping of contract → package directory lives in
`scripts/generate-clients.sh`; keep it in sync with `scripts/provenance.sh`
(the crate list is duplicated deliberately so the build stays explicit).

## Contract Testing Notes

Use `soroban_forge_test_utils::advance_ledger_to` to drive TTL and timestamp
boundaries on an existing Soroban test environment. Pass an absolute ledger
sequence and a Unix timestamp in seconds. Both may stay equal to their
current value, and either moving backwards panics before changing the ledger.

```rust
use soroban_forge_test_utils::{advance_ledger_to, new_env};

let env = new_env();
advance_ledger_to(&env, env.ledger().sequence() + 12, env.ledger().timestamp() + 60);
```

For sequence-only tests, `advance_ledger(&env, 12)` keeps the timestamp unchanged.

### Contract resource regression reports

The CI resource-report workflow runs one deterministic, storage-writing
entrypoint scenario for each contract and uploads a machine-readable report.
For pull requests, it compares each metric with the baseline on the PR's base
branch and lists deltas by entrypoint in the workflow summary. On the initial
rollout, when the base branch has no report baseline, CI bootstraps from the
current measured report; the uploaded report can then be committed as the
checked-in baseline for future base-branch comparisons.

The report includes host-estimated ledger entry accesses (read and write
counts), CPU instructions, metered memory, and an estimated transaction fee.
`footprint_entries` is the sum of the host-reported read and write entry
counts; this native test-host approximation is not the exact XDR footprint.
Soroban SDK's native test host undercounts WASM VM costs and the fee estimate
does not model transaction size or RPC simulation. These reports are for
relative regression detection only; they are not wall-clock benchmarks or
mainnet cost guarantees.

Any individual metric increase greater than **20%** fails comparison. Missing,
malformed, or duplicated scenario metrics fail report generation rather than
passing silently. To reproduce the checked-in baseline from a clean checkout,
run:

```bash
python3 scripts/contract-resource-report.py update-baseline
```

Review the resulting `reports/contract-resource-baseline.json` diff and commit
it only when the measured resource change is expected. To produce a report
without changing the baseline, run:

```bash
python3 scripts/contract-resource-report.py generate --output contract-resource-report.json
```

If the local machine cannot link native Rust tests, run the
**Contract Resource Report** workflow manually and use
`contract-resource-report.json` from its artifact as the measured baseline.

- Tests use Soroban SDK's `Env` test harness
- Mock authentication (`mock_all_auths`) for positive tests
- Integration tests use Stellar Asset Contract (SAC) fixtures
- Escrow includes property testing for conservation invariant

## Escrow Storage, TTL, and Token Trust

### Persistent storage and TTL

Escrow records are stored as per-id **persistent** entries. Each record is
extended to a **30-day TTL** when it is written. State-changing escrow
operations that update the record extend the entry's TTL using the same
threshold-and-extend-to pattern. `touch_ttl` is permissionless and can extend
an existing entry while it remains present in persistent storage.

An active escrow with no state-changing activity can eventually reach expiry.
Once the persistent escrow entry has expired, `touch_ttl` cannot recover it:
the current implementation calls `load_escrow` before attempting the TTL
extension, and a missing entry is reported as `NotFound` (the id never
existed, or its persistent entry was archived). `ttl_info(escrow_id)` is a
read-only keeper view of the remaining ledgers; poll it and call
`touch_ttl` while the escrow is still present and its TTL approaches the
29-day bump threshold. SDK 27 does not expose host TTL introspection to
contracts, so the contract mirrors each escrow expiration ledger in a
companion persistent key and updates it alongside the escrow's 30-day TTL
bumps. The test-only `get_ttl` host helper checks this mirror in tests.

If `ttl_info` or `touch_ttl` returns `NotFound`, determine whether the id
never existed or its escrow entry was archived. A keeper cannot restore an
archived entry through a contract call. Prepare a transaction whose Soroban
footprint includes the escrow data key in `readWrite`, simulate it to populate
resource and fee data, and submit a standalone `RestoreFootprintOp`; after
restoration confirms, invoke the contract again to read the record and
continue the recovery flow. On Protocol 23 and later, a simulated invocation
may include archived entries in its restore list and restore them
automatically; the standalone operation is useful when restoration fees
should be paid separately. Restoring the record does not move escrow funds:
they remain at the escrow contract's address in the token contract, and any
payout still requires a successful contract invocation against restored
state. Keeper calls are permissionless.

### Subscription records

`subscription-payments` stores each `DataKey::Subscription(id)` record in
persistent storage and extends it to 30 days whenever subscribe, charge,
charge catch-up, pause, resume, or cancel writes the record. The threshold is
29 days, following the same extend-to pattern as escrow. The `Count` counter
and the subscriber/provider enumeration indexes remain in instance storage.
Any account may call `touch_ttl(subscription_id)` to extend a present record;
it returns `NotFound` when the id is absent. Keepers should touch long-lived
subscriptions before their TTL approaches expiry. This storage cutover
assumes no deployed mainnet instance contains live subscription records.

### Token trust model

`create_escrow` accepts a user-specified token address. The escrow does not
validate whether that address is a deployed token contract and does not itself
enforce SEP-41 compliance. The design assumes the supplied token follows the
expected SEP-41 interface and behavior. Token transfer failures are handled
through the existing `ForgeError::TokenTransferFailed` path. A malicious or
non-compliant token is an external trust assumption, not a condition that the
escrow currently validates against.

## CI Pipeline

The CI workflow (`.github/workflows/ci.yml`) runs:

1. **rustfmt** - Code formatting check
2. **clippy** - Linting with strict warnings
3. **build** - Full workspace build + docs
4. **test** - Test suite execution
5. **audit** - Dependency vulnerability scan
6. **dependency-policy** - License, source, and ban checks
7. **wasm-size** - Contract size budget enforcement
8. **provenance** - Build reproducibility verification

ForgeBot (`.github/workflows/forgebot.yml`, `scripts/forgebot/`) reports these
results back to each pull request as a single sticky comment and publishes an
informational `ForgeBot / ready-for-review` commit status. It does not add or
replace any check, and it never approves or merges a pull request. See
[ForgeBot](FORGEBOT.md) for details.

### Dependency Policy (cargo-deny)

Soroban Forge enforces a dependency policy via [cargo-deny](https://embarkstudios.github.io/cargo-deny/) to ensure compliance with license compatibility, source integrity, and ban policies.

**Policy:**

- **Licenses**: Only permissive licenses (MIT, Apache-2.0, BSD, ISC, Unicode, CC0) are allowed. GPL/AGPL/SSPL are denied.
- **Sources**: All dependencies must come from crates.io; git and path dependencies are banned to prevent supply-chain surprises.
- **Bans**: Multiple versions of the same crate are flagged; justified exceptions are documented in `deny.toml`.

**Configuration:**
The policy is defined in [deny.toml](../deny.toml) at the workspace root. Each non-default choice is commented to explain the rationale.

**Running checks locally:**

```bash
# Install cargo-deny
cargo install --locked cargo-deny

# Check licenses
cargo deny check licenses

# Check sources
cargo deny check sources

# Check bans and duplicates
cargo deny check bans
```

**Adding exceptions:**
If a dependency legitimately requires an exception (e.g., an older crate with an undeclared license, or a tool-only dev dependency), add it to `deny.toml` with a clear comment explaining why it is justified. Example:

```toml
[licenses]
exceptions = [
    # { name = "crate-name", allow = ["MIT"] },  # Reason: <justification>
]
```

All exceptions must be reviewed and approved before merge.

## Common Issues

### WASM Build Fails

Ensure `wasm32v1-none` target is installed:

```bash
rustup target add wasm32v1-none
```

### Lock File Conflicts

Use `--locked` flag to ensure reproducible builds:

```bash
cargo build --locked
```

### Clippy Failures

The project uses `-D warnings` (fail on warnings). Fix issues reported by clippy or document intentional deviations.

## WASM Size Budget

Contracts have a maximum size of 150,000 bytes. This is enforced in CI.

### Deployment attestation bundles

Release provenance is emitted by `scripts/provenance.sh build` as
`provenance-manifest.json` using schema `soroban-forge/provenance@2`. The bundle
contains the source revision, Rust/Cargo/Stellar tool versions, target and
profile, Cargo.lock SHA-256, artifact SHA-256/size values, optional deployment
receipt entries, and a canonical bundle SHA-256.

A clean checkout is reproducible: the manifest timestamp is derived from the
current commit rather than wall-clock time. Tampering with any field covered by
`bundle_sha256` causes local verification to fail. The digest is integrity
metadata, not cryptographic signing.

```bash
# Build, then verify locally without network access
scripts/provenance.sh build
scripts/provenance.sh verify
scripts/provenance.sh verify --json

# Optionally match deployment contract IDs from a receipt against live WASM
DEPLOYMENT_RECEIPT=docs/testnet/testnet-deployment.json \
STELLAR_RPC_URL=https://soroban-testnet.stellar.org \
NETWORK_PASSPHRASE='Test SDF Network ; September 2015' \
scripts/provenance.sh verify --network
```

Network verification is opt-in and requires an RPC endpoint. It fetches each
configured contract ID and compares the deployed WASM SHA-256 with the matching
artifact in the bundle. Provenance does not replace an audit, does not provide
cryptographic signing, and does not assert that a deployment is safe or
authorized.
