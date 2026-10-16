# Testing Strategy

## Unit Tests

Each contract has inline unit, authorization, and property test modules for core business logic.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // ... tests
}
```

## Integration Tests

Located in `tests/integration/`. Spin up a Soroban environment and interact with deployed WASM.

## Cross-Contract Composition Tests

Executable multi-contract settlement suites live in [`crates/composition-tests`](../../crates/composition-tests/src/lib.rs), supported by shared SEP-41 and composition primitives in [`crates/test-utils`](../../crates/test-utils/src/lib.rs). Every scenario deploys at least two production contracts plus a real Stellar Asset Contract (SAC) in a shared Soroban `Env`.

### Supported Scenarios

1. **Multi-sig wallet funds an escrow** ([`crates/composition-tests/src/multi_sig_escrow.rs`](../../crates/composition-tests/src/multi_sig_escrow.rs))
   - **Setup helper:** `MultiSigEscrowFixture::new()` deploys a 2-of-3 `MultiSigWallet`, an `Escrow` contract, a SEP-41 `TokenFixture`, and a `BalanceMirror`.
   - **Flows covered:**
     - Wallet custody deposits (`deposit`) followed by threshold-approved withdrawals (`submit_withdrawal` → `confirm` → `execute`) that fund buyer escrow deposits (`create_escrow` → `deposit` → `release`).
     - Direct cross-contract calls (`submit_call`) where the multi-sig wallet acts as the escrow seller (`release_partial`) or arbiter (`resolve_split`).
     - Cross-contract authorization enforcement, sub-threshold/non-owner rejection, downstream escrow revert rollback (`ContractInvocationFailed` leaving the wallet transaction `Pending`), and rolling withdrawal limit checks.

2. **DAO governance dispatches a marketplace royalty action** ([`crates/composition-tests/src/dao_royalties.rs`](../../crates/composition-tests/src/dao_royalties.rs))
   - **Setup helper:** `DaoRoyaltiesFixture::new(bond_amount)` deploys `DaoGovernance` (initialized with governance token and proposal bond configuration), `MarketplaceRoyalties`, `RoyaltyGovernanceDispatcher` (bridging `execute(Bytes)` to typed royalty calls), `TokenFixture`, and `BalanceMirror`.
   - **Flows covered:**
     - Bonded proposal creation (`propose`), balance-weighted voting (`vote`), deadline finalisation (`Succeeded`), and execution (`execute`) dispatching single-recipient (`settle_sale`, `distribute`), multi-recipient split (`set_royalty_splits`), and batch (`settle_sales`) settlements followed by bond refund to the proposer.
     - Proposal dependency chains (`requires`) gating batch settlement behind on-chain royalty configuration.
     - Defeated proposals forfeiting bonds to the DAO treasury (`BondState::Forfeited`) without invoking `MarketplaceRoyalties`.
     - Downstream failure isolation when `MarketplaceRoyalties` returns `NotFound` or `TokenTransferFailed`, leaving the proposal in `ProposalState::Succeeded` and the bond in DAO custody (`BondState::Posted`) for subsequent retry.

3. **Vesting claim funds a subscription payment** ([`crates/composition-tests/src/vesting_subscription.rs`](../../crates/composition-tests/src/vesting_subscription.rs))
   - **Setup helper:** `VestingSubscriptionFixture::new()` deploys `Vesting`, `SubscriptionPayments`, `TokenFixture`, and `BalanceMirror`.
   - **Flows covered:**
     - Linear cliff-and-ramp schedules (`create_schedule`, `claim`) funding both direct recurring subscription charges (`subscribe`, `charge`, `charge_catchup`) and prepaid balance subscriptions (`deposit`, `charge`).
     - Discrete tranche unlock tables (`create_tranche_schedule`) funding metered-usage subscriptions (`set_quotas`, `record_usage`, `charge`).
     - Mid-schedule beneficiary reassignment (`reassign_beneficiary`, `claim_for`) splitting vested payouts across former and current subscribers.
     - Underfunded vesting custody and pre-claim charge failure isolation (`TokenTransferFailed` rollback on `charge_catchup` and `PastDue` transition on `charge` followed by `resume`).

### Setup & Verification Helpers (`soroban-forge-test-utils`)

- **[`TokenFixture`](../../crates/test-utils/src/mocks.rs):** Deterministic SEP-41 Stellar Asset Contract wrapper (`TokenFixture::new`, `mint`, `transfer`, `balance`, `client`, `asset_client`).
- **[`BalanceMirror`](../../crates/test-utils/src/composition.rs):** Independent off-ledger balance bookkeeper (`record_mint`, `record_transfer`, `snapshot`, `assert_conserved`, `assert_unchanged`) that verifies every participant's on-chain balance and total supply conservation after each settlement or rollback.
- **[`SeededRng` & `DEFAULT_COMPOSITION_SEEDS`](../../crates/test-utils/src/composition.rs):** Deterministic `SplitMix64` generator and canonical seed schedule used to execute each composition scenario repeatedly across varied parameters.
- **Event & Auth Assertions ([`composition.rs`](../../crates/test-utils/src/composition.rs)):**
  - `assert_contract_event`, `contract_event_count`, `assert_no_contract_events`, and `assert_sac_transfer_event` inspect emitted contract and SEP-41 events.
  - `assert_auth_invocation` and `has_auth_invocation` inspect `env.auths()` across root and nested cross-contract frames.

### Example

```rust
use soroban_forge_test_utils::{
    assert_sac_transfer_event, new_env, BalanceMirror, TestAccounts, TokenFixture,
};

#[test]
fn token_conservation_example() {
    let env = new_env();
    let accounts = TestAccounts::generate(&env);
    let token = TokenFixture::new(&env, &accounts.deployer);

    let participants = [accounts.user1.clone(), accounts.user2.clone()];
    let mut mirror = BalanceMirror::new(&env, &token.address, &participants);

    token.mint(&env, &accounts.user1, 1_000);
    mirror.record_mint(&accounts.user1, 1_000);

    token.transfer(&env, &accounts.user1, &accounts.user2, 400);
    assert_sac_transfer_event(&env, &token.address, &accounts.user1, &accounts.user2, 400);
    mirror.record_transfer(&accounts.user1, &accounts.user2, 400);
    mirror.assert_conserved(&env);
}
```

## Fuzzing

Consider using `cargo-fuzz` for parsing inputs and complex state machines.

## Chaos Testing

Persistent-storage contracts in this workspace must keep their records alive
between multi-step interactions. A silent failure mode is an entry whose TTL
expires between two legitimate calls: the next read returns `NotFound` (or
worse, a recreated default), corrupting state without surfacing an error.

Use the shared TTL chaos harness in
[`soroban-forge-test-utils::ttl`](../../crates/test-utils/src/ttl.rs) to exercise
contracts across arbitrary ledger gaps:

- [`advance_ledger`](../../crates/test-utils/src/ttl.rs) moves the test [`Env`]
  ledger sequence forward deterministically.
- [`entry_is_live`](../../crates/test-utils/src/ttl.rs) and
  [`entry_ttl`](../../crates/test-utils/src/ttl.rs) introspect a contract's
  persistent storage TTL from the test harness.
- [`chaos_drive`](../../crates/test-utils/src/ttl.rs) runs a seeded schedule of
  randomized operations and ledger advances against any contract that
  implements the [`ChaosTarget`](../../crates/test-utils/src/ttl.rs) trait. The same
  seed reproduces the same trace, so CI failures are replayable.

Demo suites for the two richest persistent-storage contracts are in:

- `crates/escrow/src/ttl_chaos.rs` — create → deposit → release/refund over
  arbitrary ledger gaps.
- `crates/multi-sig-wallet/src/ttl_chaos.rs` — submit → confirm → execute
  over arbitrary ledger gaps.

Each demo asserts the **no `NotFound` during legitimate flows** property and
prints a sample trace for inspection. When a storage refactor breaks TTL
discipline, the harness reports the seed and the exact step so the violation
can be replayed locally.

Guidelines for adding a new contract to the harness:

1. Define a small, closed enum of operations (`Create`, `Deposit`, ...).
2. Implement `ChaosTarget` with `enabled_ops` that only exposes operations
   valid in the current state.
3. In `apply_op`, classify contract errors: expected state-machine failures
   are `ExpectedFailure`; `ForgeError::NotFound` on a record that should exist
   is a `Violation`.
4. In `check_invariants`, assert every persistent record touched by the
   legitimate flow remains live via `entry_is_live`.
5. Pin the seed and assert `assert_no_violations(&trace)`.

## Coverage

Target >= 90% line coverage for stable contracts.

## CI Gates

All PRs must pass:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test -p soroban-forge-test-utils --all-targets --locked`
- `cargo test -p soroban-forge-composition-tests --all-targets --locked`
- `cargo test --workspace --all-targets --locked`
