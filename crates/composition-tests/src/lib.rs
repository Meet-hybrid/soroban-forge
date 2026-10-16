#![cfg(test)]

//! Cross-contract composition integration tests for Soroban Forge.
//!
//! This crate exercises real Soroban [`Env`] deployments of multiple workspace
//! contracts interacting across contract boundaries against deterministic
//! SEP-41 Stellar Asset Contract fixtures ([`TokenFixture`]):
//!
//! - [`multi_sig_escrow`]: A multi-signature wallet custodies SEP-41 tokens,
//!   collects threshold approvals from its owner set, funds an escrow, and
//!   drives release, partial release, refund, and arbitration flows via
//!   cross-contract calls.
//! - [`dao_royalties`]: A DAO governance contract accepts bonded proposals,
//!   tallies token-weighted votes, and dispatches marketplace royalty
//!   settlements (`settle_sale`, `distribute`, `settle_sales`) across single-
//!   and multi-recipient creator splits before refunding or forfeiting proposal
//!   bonds.
//! - [`vesting_subscription`]: Linear and tranche vesting schedules release
//!   tokens to a beneficiary who funds recurring, prepaid, and metered
//!   subscriptions charged by a service provider.
//!
//! ## Invariants verified in every scenario
//!
//! 1. **Real multi-contract execution**: Every test deploys and invokes at
//!    least two production contracts plus a real SEP-41 token contract.
//! 2. **Cross-contract authorization**: Both positive call-graph checks
//!    (`env.auths()` via [`assert_auth_invocation`]) and negative host-level
//!    enforcement (`env.mock_auths` / `env.set_auths`) verify that unauthorized
//!    callers are rejected.
//! 3. **Failure isolation & rollback**: Failed downstream contract calls leave
//!    upstream contract state and all SEP-41 balances untouched.
//! 4. **Event verification**: Contract lifecycle events and SEP-41 `transfer`
//!    events are asserted on every state transition.
//! 5. **Independent balance conservation**: [`BalanceMirror`] tracks expected
//!    balances independently of on-chain storage and verifies that individual
//!    balances and total token supply are conserved across all participants,
//!    including repeated runs over [`DEFAULT_COMPOSITION_SEEDS`].

use soroban_forge_escrow::{Escrow, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_multi_sig_wallet::{MultiSigWallet, SorobanForgeMultiSigWalletClient, TxStatus};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{Env, IntoVal, Symbol, Vec};

pub use soroban_forge_test_utils::{
    assert_auth_invocation, assert_contract_event, assert_no_contract_events,
    assert_sac_transfer_event, contract_event_count, has_auth_invocation, new_env, BalanceMirror,
    SeededRng, TestAccounts, TokenFixture, DEFAULT_COMPOSITION_SEEDS,
};

mod dao_royalties;
mod multi_sig_escrow;
mod vesting_subscription;

/// Baseline ledger timestamp used by composition fixtures.
pub const INITIAL_LEDGER_TIMESTAMP: u64 = 1_720_000_000;

/// Create a deterministic [`Env`], named [`TestAccounts`], and a deployed
/// SEP-41 [`TokenFixture`] administered by `accounts.deployer`.
pub fn setup_env() -> (Env, TestAccounts, TokenFixture) {
    let env = new_env();
    env.ledger().set_timestamp(INITIAL_LEDGER_TIMESTAMP);
    let accounts = TestAccounts::generate(&env);
    let token = TokenFixture::new(&env, &accounts.deployer);
    (env, accounts, token)
}

#[test]
fn composition_harness_multi_contract_smoke() {
    let (env, accounts, token) = setup_env();

    let wallet_id = env.register(MultiSigWallet, ());
    let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);

    let mut owners = Vec::new(&env);
    owners.push_back(accounts.user1.clone());
    owners.push_back(accounts.user2.clone());
    wallet.initialize(&owners, &2);

    let participants = [
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.user3.clone(),
        wallet_id.clone(),
        escrow_id.clone(),
    ];
    let mut mirror = BalanceMirror::new(&env, &token.address, &participants);

    token.mint(&env, &accounts.user1, 2_000);
    mirror.record_mint(&accounts.user1, 2_000);
    mirror.assert_conserved(&env);

    // Deposit into multi-sig custody, then withdraw to buyer and fund escrow.
    wallet.deposit(&token.address, &accounts.user1, &1_500);
    mirror.record_transfer(&accounts.user1, &wallet_id, 1_500);
    mirror.assert_conserved(&env);

    let tx_id = wallet.submit_withdrawal(&accounts.user1, &token.address, &accounts.user1, &1_000);
    wallet.confirm(&tx_id, &accounts.user1);
    wallet.confirm(&tx_id, &accounts.user2);
    wallet.execute(&tx_id);
    assert_eq!(wallet.get_tx(&tx_id).status, TxStatus::Executed);
    mirror.record_transfer(&wallet_id, &accounts.user1, 1_000);
    mirror.assert_conserved(&env);

    let deal_id = escrow.create_escrow(
        &accounts.user1,
        &wallet_id,
        &accounts.arbiter,
        &token.address,
        &1_000,
        &3_600,
    );
    escrow.deposit(&deal_id);
    mirror.record_transfer(&accounts.user1, &escrow_id, 1_000);
    mirror.assert_conserved(&env);

    // Multi-sig wallet is the seller of the escrow and dispatches `release`
    // via a threshold-approved cross-contract `submit_call`.
    let release_args = soroban_sdk::vec![&env, deal_id.into_val(&env)];
    let release_tx = wallet.submit_call(
        &accounts.user1,
        &escrow_id,
        &Symbol::new(&env, "release"),
        &release_args,
        &None,
    );
    wallet.confirm(&release_tx, &accounts.user1);
    wallet.confirm(&release_tx, &accounts.user2);
    wallet.execute(&release_tx);

    assert_eq!(escrow.get_status(&deal_id), EscrowStatus::Completed);
    // Payout from escrow goes to seller (`wallet_id`).
    mirror.record_transfer(&escrow_id, &wallet_id, 1_000);
    mirror.assert_conserved(&env);
}
