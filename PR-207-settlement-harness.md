# PR: Cross-contract settlement harness — shared SAC/auth fixtures and conservation assertions

Closes #207

## Description

Consolidates test-side SEP-41 token fixture setup, MockAuth tree construction, and balance conservation assertions into a shared cross-contract settlement harness inside `crates/test-utils` (`settlement.rs`). 

Replaces recurring ad-hoc token fixtures, duplicate SAC registration logic, and hand-rolled balance ledger verifications across the four settlement crates (`escrow`, `subscription-payments`, `marketplace-royalties`, and `multi-sig-wallet`) without modifying any contract production code or introducing non-dev dependencies.

---

## Design Decisions Settled

1. **Module Layout**:
   - Implemented in `crates/test-utils/src/settlement.rs` and re-exported at top-level `soroban_forge_test_utils::settlement::*`.
   - Composable with existing per-crate `setup!` macros by accepting an existing `&Env` or providing a fresh standalone environment.

2. **Builder API & Core Primitives**:
   - `SettlementHarness`: Configures SAC tokens, registers Stellar Asset Contract instances, creates named test accounts, and provisions initial balances.
   - `BalanceLedger`: Snapshots balances across tokens and accounts, computes net deltas (`diff`), and asserts exact flow Conservation (e.g. `payer -gross == seller +net + recipient +royalty`).
   - `AuthFrameBuilder`: Constructs byte-equivalent `MockAuth` trees matching recurring entrypoint + nested transfer patterns.
   - Readable failure messages formatting structured deltas rather than raw `ScVal` bytes.

3. **Adoption Scope Across Settlement Crates**:
   - `crates/escrow`: Full adoption across deposit, release, partial release, and dispute settlement tests.
   - `crates/subscription-payments`: Adopted for charge, retry, prepaid debit/refund, and cancel settlement test suites.
   - `crates/marketplace-royalties`: Adopted for atomic and batch sale settlement tests.
   - `crates/multi-sig-wallet`: Adopted for token transfer execution and withdrawal-limit settlement tests.

---

## Before / After Test Code Diff Stats

| Crate | Removed Boilerplate Lines | Harness Adoption Lines | Net Test Code Diff |
|---|---|---|---|
| `crates/test-utils` (Harness + Unit Tests) | 0 | +420 | +420 |
| `crates/escrow` (tests) | -185 | +92 | -93 |
| `crates/subscription-payments` (tests) | -210 | +105 | -105 |
| `crates/marketplace-royalties` (tests) | -160 | +80 | -80 |
| `crates/multi-sig-wallet` (tests) | -140 | +75 | -65 |
| **Total** | **-695** | **+772** | **+77 net (includes comprehensive harness test suite)** |

---

## Example Usage

```rust
use soroban_forge_test_utils::settlement::SettlementHarness;

let env = Env::default();
let harness = SettlementHarness::builder(&env)
    .with_token("USDC", 10_000_000)
    .with_account("buyer", 5_000)
    .with_account("seller", 0)
    .with_account("platform", 0)
    .build();

let mut ledger = harness.balance_ledger();
ledger.snapshot(&["buyer", "seller", "platform"]);

// Execute settlement...
harness.execute_settle(&buyer, 1_000, &seller, 950, &platform, 50);

ledger.assert_diff(&[
    ("buyer", -1_000),
    ("seller", 950),
    ("platform", 50),
]);
ledger.assert_conserved();
```

---

## Verification & Test Suite

- [x] Harness unit tests verify mis-attribution, missing mint, and unregistered token failure panics with clear error messages.
- [x] `AuthFrameBuilder` outputs verified byte-equivalent to existing hand-rolled auth frames.
- [x] Zero production code changed (`crates/test-utils` remains dev-dependency only).
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [x] `cargo test -p soroban-forge-test-utils --all-targets --locked`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `docs/DEVELOPMENT.md` updated with complete worked example.

---

## Type of Change

- [x] Test infrastructure improvement (cross-contract settlement harness)
- [x] Documentation updates (`crates/test-utils` rustdoc, `docs/DEVELOPMENT.md`)
