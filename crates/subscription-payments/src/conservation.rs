//! Explicit deterministic conservation equations for prepaid subscription transitions.

use crate::{SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};

const START: u64 = 2_000_000;
const PERIOD: u64 = 1_000;
const AMOUNT: i128 = 250;

fn fixture() -> (
    Env,
    Address,
    Address,
    Address,
    Address,
    u64,
    SorobanForgeSubscriptionPaymentsClient<'static>,
) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(START);
    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let subscriber = Address::generate(&env);
    let provider = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&subscriber, &10_000);
    let contract = env.register(SubscriptionPayments, ());
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract);
    let id = client.subscribe(&subscriber, &provider, &token, &AMOUNT, &PERIOD);
    (env, token, subscriber, provider, contract, id, client)
}

#[test]
fn deposit_then_charge_balance_delta_is_exact() {
    let (env, token, subscriber, provider, contract, id, client) = fixture();
    let token_client = TokenClient::new(&env, &token);
    let balance_before = client.get_subscription(&id).prepaid_balance.unwrap_or(0);

    client.deposit(&id, &(AMOUNT * 2));
    env.ledger().set_timestamp(START + PERIOD);
    let charged = client.charge(&id);
    let balance_after = client.get_subscription(&id).prepaid_balance.unwrap_or(0);

    assert_eq!(
        balance_before + (AMOUNT * 2) - charged,
        balance_after,
        "deposit then charge equation: balance_before + deposit - charge == balance_after"
    );
    assert_eq!(token_client.balance(&provider), charged);
    assert_eq!(token_client.balance(&subscriber), 10_000 - AMOUNT * 2);
    assert_eq!(token_client.balance(&contract), AMOUNT);
}

#[test]
fn withdrawal_and_refund_never_allow_negative_prepaid_balance() {
    let (env, token, subscriber, provider, contract, id, client) = fixture();
    let token_client = TokenClient::new(&env, &token);
    client.deposit(&id, &500);

    assert_eq!(
        client.withdraw_balance(&id, &200),
        300,
        "withdrawal equation: balance_before - amount == balance_after"
    );
    let err = client.try_withdraw_balance(&id, &301).unwrap_err().unwrap();
    assert_eq!(
        err,
        soroban_forge_shared_utils::ForgeError::InsufficientFunds
    );
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(300));

    client.cancel(&id);
    assert_eq!(
        client.get_subscription(&id).prepaid_balance,
        Some(0),
        "refund equation: paid refund equals the remaining prepaid balance"
    );
    assert_eq!(token_client.balance(&subscriber), 10_000);
    assert_eq!(token_client.balance(&provider), 0);
    assert_eq!(token_client.balance(&contract), 0);
}

#[test]
fn rejected_transition_preserves_balance_status_and_tokens() {
    let (env, token, subscriber, _provider, contract, id, client) = fixture();
    let token_client = TokenClient::new(&env, &token);
    client.deposit(&id, &500);
    let before = client.get_subscription(&id);
    let subscriber_before = token_client.balance(&subscriber);
    let contract_before = token_client.balance(&contract);

    assert_eq!(
        client.try_deposit(&id, &0).unwrap_err().unwrap(),
        soroban_forge_shared_utils::ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_withdraw_balance(&id, &501).unwrap_err().unwrap(),
        soroban_forge_shared_utils::ForgeError::InsufficientFunds
    );

    assert_eq!(
        client.get_subscription(&id),
        before,
        "rejected transitions preserve subscription status and prepaid balance"
    );
    assert_eq!(token_client.balance(&subscriber), subscriber_before);
    assert_eq!(token_client.balance(&contract), contract_before);
}

#[test]
fn repeated_deposit_and_charge_preserve_total_value_with_rounding() {
    let (env, token, subscriber, provider, contract, id, client) = fixture();
    let token_client = TokenClient::new(&env, &token);
    for _ in 0..3 {
        client.deposit(&id, &250);
    }
    env.ledger().set_timestamp(START + PERIOD);
    for _ in 0..2 {
        assert_eq!(client.charge(&id), AMOUNT);
        env.ledger()
            .set_timestamp(env.ledger().timestamp() + PERIOD);
    }
    let balance_after = client.get_subscription(&id).prepaid_balance.unwrap_or(0);
    assert_eq!(
        750 - 500,
        balance_after,
        "repeated deposits and charges: deposits - charges == balance_after"
    );
    assert_eq!(token_client.balance(&provider), 500);
    assert_eq!(token_client.balance(&contract), balance_after);
    assert_eq!(
        token_client.balance(&subscriber)
            + token_client.balance(&contract)
            + token_client.balance(&provider),
        10_000,
        "repeated operations conserve subscriber, contract, and provider token value"
    );
}

#[test]
fn catchup_consumes_one_period_and_residual_arrears_remain() {
    let (env, token, subscriber, provider, contract, id, client) = fixture();
    let token_client = TokenClient::new(&env, &token);
    client.deposit(&id, &(AMOUNT * 2));
    env.ledger().set_timestamp(START + PERIOD * 3);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(AMOUNT));
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(0));
    assert_eq!(token_client.balance(&provider), AMOUNT * 2);
    assert_eq!(token_client.balance(&subscriber), 10_000 - AMOUNT * 2);
    assert_eq!(token_client.balance(&contract), 0);
}

#[test]
fn over_withdrawal_cannot_spend_another_subscriptions_escrow() {
    let (env, token, _subscriber, provider, contract, id, client) = fixture();
    let token_client = TokenClient::new(&env, &token);
    let other_subscriber = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&other_subscriber, &10_000);
    let other_id = client.subscribe(&other_subscriber, &provider, &token, &AMOUNT, &PERIOD);
    client.deposit(&id, &300);
    client.deposit(&other_id, &500);

    assert_eq!(
        client.try_withdraw_balance(&id, &301).unwrap_err().unwrap(),
        soroban_forge_shared_utils::ForgeError::InsufficientFunds,
        "a withdrawal larger than this subscription's balance must be rejected          even when the contract holds enough tokens from other subscriptions"
    );
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(300));
    assert_eq!(
        client.get_subscription(&other_id).prepaid_balance,
        Some(500)
    );
    assert_eq!(token_client.balance(&contract), 800);
}
