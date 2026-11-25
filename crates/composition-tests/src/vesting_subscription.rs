//! Vesting claims fund subscription charges until the beneficiary runs out.

use super::*;
use soroban_forge_subscription_payments::{
    SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus,
};
use soroban_forge_vesting::{SorobanForgeVestingClient, Vesting, VestingStatus};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::Address;

const START: u64 = 2_000_000;
const TOTAL_VESTED: i128 = 120;
const CHARGE: i128 = 60;
const BILLING_PERIOD: u64 = 10;

#[test]
fn vesting_claim_pays_subscription_until_funds_run_out() {
    let (env, accounts) = setup_env();
    env.ledger().set_timestamp(START);

    let token_admin_id = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin_id.clone())
        .address();
    let token_admin = StellarAssetClient::new(&env, &token_id);
    let token = TokenClient::new(&env, &token_id);
    let vesting_id = env.register(Vesting, ());
    let vesting = SorobanForgeVestingClient::new(&env, &vesting_id);
    let subscription_id = env.register(SubscriptionPayments, ());
    let subscriptions = SorobanForgeSubscriptionPaymentsClient::new(&env, &subscription_id);

    token_admin.mint(&accounts.deployer, &TOTAL_VESTED);
    let holders = [
        accounts.deployer.clone(),
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.user3.clone(),
        accounts.validator.clone(),
        accounts.arbiter.clone(),
        token_admin_id,
        token_id.clone(),
        vesting_id.clone(),
        subscription_id,
    ];
    let funder_has_supply = [TOTAL_VESTED, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_balances(&token, TOTAL_VESTED, &holders, &funder_has_supply);

    token.transfer(&accounts.deployer, &vesting_id, &TOTAL_VESTED);
    let vesting_funded = [0, 0, 0, 0, 0, 0, 0, 0, TOTAL_VESTED, 0];
    assert_balances(&token, TOTAL_VESTED, &holders, &vesting_funded);

    let schedule_id = vesting.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token_id,
        &TOTAL_VESTED,
        &0,
        &BILLING_PERIOD,
    );
    assert_eq!(
        vesting.get_schedule(&schedule_id).status,
        VestingStatus::Vesting
    );
    assert_balances(&token, TOTAL_VESTED, &holders, &vesting_funded);

    env.ledger().set_timestamp(START + BILLING_PERIOD);
    assert_eq!(vesting.claim(&schedule_id), TOTAL_VESTED);
    let beneficiary_claimed = [0, TOTAL_VESTED, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        vesting.get_schedule(&schedule_id).status,
        VestingStatus::Completed
    );
    assert_balances(&token, TOTAL_VESTED, &holders, &beneficiary_claimed);

    let subscription = subscriptions.subscribe(
        &accounts.user1,
        &accounts.validator,
        &token_id,
        &CHARGE,
        &BILLING_PERIOD,
    );
    assert_eq!(
        subscriptions.get_subscription(&subscription).status,
        SubscriptionStatus::Active
    );
    assert_balances(&token, TOTAL_VESTED, &holders, &beneficiary_claimed);

    env.ledger().set_timestamp(START + BILLING_PERIOD * 2);
    assert_eq!(subscriptions.charge(&subscription), CHARGE);
    let first_charge = [0, TOTAL_VESTED - CHARGE, 0, 0, CHARGE, 0, 0, 0, 0, 0];
    assert_balances(&token, TOTAL_VESTED, &holders, &first_charge);

    env.ledger().set_timestamp(START + BILLING_PERIOD * 3);
    assert_eq!(subscriptions.charge(&subscription), CHARGE);
    let fully_spent = [0, 0, 0, 0, TOTAL_VESTED, 0, 0, 0, 0, 0];
    assert_balances(&token, TOTAL_VESTED, &holders, &fully_spent);

    env.ledger().set_timestamp(START + BILLING_PERIOD * 4);
    assert_eq!(subscriptions.charge(&subscription), 0);
    let past_due = subscriptions.get_subscription(&subscription);
    assert_eq!(past_due.status, SubscriptionStatus::PastDue);
    assert_eq!(past_due.failed_attempts, 1);
    assert_balances(&token, TOTAL_VESTED, &holders, &fully_spent);
}
