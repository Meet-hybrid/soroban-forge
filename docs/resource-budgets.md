# Resource & Budget Regression Gates

CI enforces a WASM size budget for every contract artifact, but byte size says
nothing about execution cost. The **resource budget gate** catches *substantial*
execution-cost regressions in contract entrypoints before they ship, and
stores the accepted costs as a versioned, reviewable baseline.

- Harness crate: [`crates/resource-budget`](../crates/resource-budget)
- Baseline file: [`budgets/resource-baseline.json`](../budgets/resource-baseline.json)
- Gate test: [`crates/resource-budget/tests/resource_gate.rs`](../crates/resource-budget/tests/resource_gate.rs)
  (runs as part of `cargo test --workspace --all-targets --locked` and as the
  dedicated **Resource Budget Gate** CI job)
- Updater: [`scripts/update-resource-baseline.sh`](../scripts/update-resource-baseline.sh)

## What is measured

Each contract has at least one **representative benchmark flow** (see
`crates/resource-budget/src/flows/`): a deterministic fixture (fixed ledger
timestamp, deterministic mock accounts, Stellar Asset Contract token) followed
by a single measured invocation of a representative entrypoint. Fixture steps
run before the measurement; the entrypoint under test is always the final
top-level invocation, and its resources are captured immediately afterwards —
before any post-hoc assertions, which are themselves top-level invocations.

For the measured invocation the gate captures everything Stellar tooling
exposes in-process, via `Env::cost_estimate()` (soroban-sdk 28):

| Surface | Metrics |
| --- | --- |
| Budget metering | `instructions`, `mem_bytes` |
| Transaction resources | footprint: `memory_read_entries`, `disk_read_entries`, `write_entries`, `disk_read_bytes`, `write_bytes`; events: `contract_events_size_bytes`; rent: `persistent_entry_rent_bumps`, `temporary_entry_rent_bumps`, `persistent_rent_ledger_bytes`, `temporary_rent_ledger_bytes` |
| Fee snapshot | `fee_total_stroops` (mainnet fee rates snapshot embedded in the SDK) |

Current flows:

| package | entrypoint | scenario |
| --- | --- | --- |
| `soroban-forge-escrow` | `release` | `create_deposit_release` |
| `soroban-forge-vesting` | `claim` | `create_schedule_claim_mid_vesting` |
| `soroban-forge-multi-sig-wallet` | `execute` | `submit_confirm_execute` |
| `soroban-forge-dao-governance` | `execute` | `propose_vote_execute` |
| `soroban-forge-subscription-payments` | `charge` | `subscribe_then_charge` |
| `soroban-forge-marketplace-royalties` | `settle_sale` | `set_royalty_settle_sale` |

## Determinism

Flows are deterministic and network-independent by construction:

- `Env::default()` fixes the base PRNG seed and ledger info; mock accounts are
  derived from it, so addresses are reproducible.
- Fixtures pin the ledger timestamp and mint fixed token amounts — no
  wall-clock time, no randomness, no RPC/testnet access.
- Cost accounting in the Soroban host is model-based (per-operation models),
  so the measured numbers are reproducible on a given toolchain.

This is deliberately *not* a wall-clock benchmark: timing is noisy and says
nothing about on-chain resource limits.

## Thresholds

Thresholds live in the baseline file and are applied to every metric:

```
allowed = baseline + max(baseline * max_regression_percent / 100,
                         min_absolute_delta[metric])
```

A metric **fails the gate** when the measured value exceeds `allowed` — i.e. a
regression must clear *both* its percentage budget and the per-metric absolute
floor. The floors keep tiny absolute wiggles on small numbers (a few extra
entries, a few hundred bytes) from failing CI, while substantial jumps always
do. Improvements never fail the gate; they simply sit below the baseline until
the next intentional refresh.

When the gate fails, the report (job summary, `target/resource-budget-report.md`,
and the test's assertion output) names the affected **package, entrypoint,
scenario, and metric** with baseline/measured/allowed values.

## Updating a baseline intentionally

Baselines are only ever updated through the maintainer-controlled path:

1. Make the contract change (or flow change) that justifies a new budget.
2. Run `scripts/update-resource-baseline.sh`. This re-runs every flow with
   `UPDATE_RESOURCE_BASELINE=1` and rewrites `budgets/resource-baseline.json`
   from the fresh measurements (thresholds are preserved).
3. Review the diff of the baseline file — every changed number should be
   explainable by the change in the same PR. Note the reason in the baseline's
   `note` field if useful.
4. Commit the baseline change **in the same PR** as the code change.

CI never sets `UPDATE_RESOURCE_BASELINE`, so a PR that shifts costs without an
explicit baseline update fails the gate; a PR that updates the baseline gets
the diff reviewed like any other code change.

## Caveats

- **Native metering.** Flows run contracts natively in the test host, not as
  deployed Wasm. VM instantiation, Wasm-code reads, and code-entry rent bumps
  are therefore not part of the numbers. This is fine for a *regression* gate
  (the measurement is stable and sensitive to contract-level cost changes) but
  the absolute numbers are not mainnet performance claims.
- **Fee figures** use the mainnet fee-rate snapshot embedded in soroban-sdk
  and may drift from live network settings; treat them as a relative signal.
- **No optimization in this change.** The gate only detects regressions; it
  does not make any contract cheaper.
- The WASM size check in CI is unchanged and remains the byte-size authority.

## Adding a flow for a new contract

1. Add a module under `crates/resource-budget/src/flows/` with a
   deterministic `pub(crate) fn run() -> ResourceSample`.
2. Register it in `flows::FLOWS` and add the package to
   `gate::CONTRACT_PACKAGES` (the gate fails while any contract package lacks
   a flow).
3. Run `scripts/update-resource-baseline.sh` and commit the baseline entry.
