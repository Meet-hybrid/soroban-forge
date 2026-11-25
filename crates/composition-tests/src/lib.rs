#![cfg(test)]

//! Cross-contract composition integration tests for Soroban Forge.
//!
//! This crate tests actual cross-contract interactions rather than individual
//! contract behavior. These tests verify that the contracts work correctly
//! together in realistic scenarios.
//!
//! Each scenario deploys the participating contracts into one Soroban
//! environment and checks their real token balances.

use soroban_sdk::token::Client as TokenClient;
use soroban_sdk::{Address, Env};

mod boundary_auth_rollback;
mod dao_dispatch;
mod dao_dispute;
mod full_lifecycle_conservation;
mod harness;
mod multi_sig_escrow;
mod royalty_settlement;
mod vesting_subscription;
mod wallet_escrow_settlement;
mod wallet_subscription_settlement;

/// Helper to create a test environment for composition tests.
fn setup_env() -> (Env, soroban_forge_test_utils::TestAccounts) {
    let env = soroban_forge_test_utils::new_env();
    // Composition scenarios exercise authorization through nested contract calls.
    env.mock_all_auths_allowing_non_root_auth();
    let accounts = soroban_forge_test_utils::TestAccounts::generate(&env);

    (env, accounts)
}

fn assert_balances(
    token: &TokenClient<'_>,
    minted_supply: i128,
    holders: &[Address],
    expected: &[i128],
) {
    assert_eq!(holders.len(), expected.len());

    let mut total = 0_i128;
    for (holder, expected_balance) in holders.iter().zip(expected) {
        let balance = token.balance(holder);
        assert_eq!(balance, *expected_balance);
        total += balance;
    }

    assert_eq!(total, minted_supply);
}

#[test]
fn composition_test_infrastructure() {
    let (env, accounts) = setup_env();

    assert_eq!(accounts.all(&env).len(), 6);
    assert_ne!(accounts.user1, accounts.user2);
}
