use super::*;
use crate::{SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus};
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::xdr::{ContractEventBody, Int128Parts, ScSymbol, ScVal};
use soroban_sdk::{Address, Env};

const START: u64 = 1_000_000;
const PERIOD: u64 = 1_000;
const AMOUNT: i128 = 250;

macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_timestamp(START);

        let admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let token_admin = StellarAssetClient::new(&env, &token);
        let token_client = TokenClient::new(&env, &token);

        let contract_id = env.register(SubscriptionPayments, ());
        let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
        let accounts = TestAccounts::generate(&env);
        token_admin.mint(&accounts.user1, &10_000_i128);

        (env, token, contract_id, client, accounts, token_client)
    }};
}

#[test]
fn deposit_happy_path() {
    let (_env, token, contract_id, client, accounts, token_client) = setup!();
    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    let id = client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);
    assert_eq!(client.get_prepaid_balance(&id), 0);

    let sub_initial = token_client.balance(subscriber);
    assert_eq!(token_client.balance(&contract_id), 0);

    // Deposit 1000 tokens
    client.deposit(&id, &1000);

    assert_eq!(client.get_prepaid_balance(&id), 1000);
    assert_eq!(token_client.balance(subscriber), sub_initial - 1000);
    assert_eq!(token_client.balance(&contract_id), 1000);

    // Deposit another 500 tokens
    client.deposit(&id, &500);
    assert_eq!(client.get_prepaid_balance(&id), 1500);
    assert_eq!(token_client.balance(subscriber), sub_initial - 1500);
    assert_eq!(token_client.balance(&contract_id), 1500);
}

#[test]
fn deposit_invalid_amount_rejected() {
    let (_env, token, _contract_id, client, accounts, _token_client) = setup!();
    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    let id = client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);

    assert_eq!(
        client.try_deposit(&id, &0).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_deposit(&id, &-100).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
}

#[test]
fn deposit_missing_subscription_is_not_found() {
    let (_env, _token, _contract_id, client, _accounts, _token_client) = setup!();

    assert_eq!(
        client.try_deposit(&999, &500).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_get_prepaid_balance(&999).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
}

#[test]
fn deposit_cancelled_subscription_is_rejected() {
    let (_env, token, _contract_id, client, accounts, _token_client) = setup!();
    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    let id = client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);
    client.cancel(&id);

    assert_eq!(
        client.try_deposit(&id, &500).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
}

#[test]
fn deposit_failure_leaves_balance_and_tokens_unchanged() {
    let (_env, token, contract_id, client, accounts, token_client) = setup!();
    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    let id = client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);
    let before = client.get_subscription(&id);
    let user_before = token_client.balance(subscriber);
    let result = client.try_deposit(&id, &(user_before + 1));
    assert_eq!(
        result.unwrap_err().unwrap(),
        ForgeError::TokenTransferFailed
    );
    assert_eq!(client.get_subscription(&id), before);
    assert_eq!(token_client.balance(subscriber), user_before);
    assert_eq!(token_client.balance(&contract_id), 0);
}

#[test]
fn charge_prioritizes_prepaid_balance_exact() {
    let (env, token, contract_id, client, accounts, token_client) = setup!();
    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    let id = client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);

    // Deposit 500 (enough for 2 periods of 250)
    client.deposit(&id, &500);
    assert_eq!(client.get_prepaid_balance(&id), 500);
    assert_eq!(token_client.balance(&contract_id), 500);

    let sub_bal_after_deposit = token_client.balance(subscriber);
    let prov_initial = token_client.balance(provider);

    // Period 1 elapses
    env.ledger().set_timestamp(START + PERIOD);

    let billed = client.charge(&id);
    assert_eq!(billed, AMOUNT);

    // Prepaid balance was debited by 250
    assert_eq!(client.get_prepaid_balance(&id), 250);
    assert_eq!(token_client.balance(&contract_id), 250);
    assert_eq!(token_client.balance(provider), prov_initial + AMOUNT);
    // Subscriber's wallet balance did not decrease during charge
    assert_eq!(token_client.balance(subscriber), sub_bal_after_deposit);

    // Period 2 elapses
    env.ledger().set_timestamp(START + 2 * PERIOD);
    let billed2 = client.charge(&id);
    assert_eq!(billed2, AMOUNT);

    // Prepaid balance is now completely depleted
    assert_eq!(client.get_prepaid_balance(&id), 0);
    assert_eq!(token_client.balance(&contract_id), 0);
    assert_eq!(token_client.balance(provider), prov_initial + 2 * AMOUNT);
    assert_eq!(token_client.balance(subscriber), sub_bal_after_deposit);

    // Period 3 elapses: prepaid balance is 0, so falls back to subscriber wallet
    env.ledger().set_timestamp(START + 3 * PERIOD);
    let billed3 = client.charge(&id);
    assert_eq!(billed3, AMOUNT);

    assert_eq!(client.get_prepaid_balance(&id), 0);
    assert_eq!(
        token_client.balance(subscriber),
        sub_bal_after_deposit - AMOUNT
    );
    assert_eq!(token_client.balance(provider), prov_initial + 3 * AMOUNT);
}

#[test]
fn charge_insufficient_prepaid_and_wallet_fails_and_lapses() {
    let (env, token, contract_id, client, accounts, token_client) = setup!();
    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    let id = client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);

    // Deposit 100 (< AMOUNT 250)
    client.deposit(&id, &100);
    assert_eq!(client.get_prepaid_balance(&id), 100);

    // Drain subscriber's remaining wallet
    let remaining_wallet = token_client.balance(subscriber);
    token_client.transfer(subscriber, provider, &remaining_wallet);
    assert_eq!(token_client.balance(subscriber), 0);

    // Period elapses
    env.ledger().set_timestamp(START + PERIOD);

    // Charge: prepaid balance is 100 (< 250), so attempts wallet pull which fails
    let billed = client.charge(&id);
    assert_eq!(billed, 0);

    let sub = client.get_subscription(&id);
    assert_eq!(sub.status, SubscriptionStatus::PastDue);
    assert_eq!(sub.failed_attempts, 1);
    // Prepaid balance remains untouched
    assert_eq!(client.get_prepaid_balance(&id), 100);
    assert_eq!(token_client.balance(&contract_id), 100);

    // Subscriber tops up prepaid balance by 200 (now 300 >= 250)
    // First mint to subscriber to allow deposit
    let token_admin = StellarAssetClient::new(&env, &token);
    token_admin.mint(subscriber, &200);
    client.deposit(&id, &200);
    assert_eq!(client.get_prepaid_balance(&id), 300);

    // Retry charge: now succeeds from prepaid balance and restores Active!
    let billed2 = client.charge(&id);
    assert_eq!(billed2, AMOUNT);

    let sub2 = client.get_subscription(&id);
    assert_eq!(sub2.status, SubscriptionStatus::Active);
    assert_eq!(sub2.failed_attempts, 0);
    assert_eq!(client.get_prepaid_balance(&id), 50);
}

#[test]
fn cancel_refunds_exact_remaining_prepaid_balance() {
    let (env, token, contract_id, client, accounts, token_client) = setup!();
    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    let id = client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);
    let sub_initial = token_client.balance(subscriber);

    // Deposit 1000
    client.deposit(&id, &1000);
    assert_eq!(token_client.balance(subscriber), sub_initial - 1000);
    assert_eq!(token_client.balance(&contract_id), 1000);

    // Charge 1 period (250)
    env.ledger().set_timestamp(START + PERIOD);
    client.charge(&id);

    assert_eq!(client.get_prepaid_balance(&id), 750);
    assert_eq!(token_client.balance(&contract_id), 750);

    // Cancel subscription: exact refund of 750
    client.cancel(&id);

    let sub = client.get_subscription(&id);
    assert_eq!(sub.status, SubscriptionStatus::Cancelled);
    assert_eq!(client.get_prepaid_balance(&id), 0);
    assert_eq!(token_client.balance(&contract_id), 0);
    assert_eq!(token_client.balance(subscriber), sub_initial - 250);
}

#[test]
fn cancel_with_zero_prepaid_balance_succeeds_without_transfer() {
    let (_env, token, contract_id, client, accounts, token_client) = setup!();
    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    let id = client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);
    let sub_initial = token_client.balance(subscriber);

    client.cancel(&id);

    let sub = client.get_subscription(&id);
    assert_eq!(sub.status, SubscriptionStatus::Cancelled);
    assert_eq!(client.get_prepaid_balance(&id), 0);
    assert_eq!(token_client.balance(subscriber), sub_initial);
    assert_eq!(token_client.balance(&contract_id), 0);
}

#[test]
fn paused_and_resumed_subscription_conserves_prepaid_balance() {
    let (env, token, contract_id, client, accounts, _token_client) = setup!();
    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    let id = client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);
    client.deposit(&id, &1000);

    // Pause
    client.pause(&id);
    let sub = client.get_subscription(&id);
    assert_eq!(sub.status, SubscriptionStatus::Paused);

    // Charge fails while paused
    env.ledger().set_timestamp(START + PERIOD);
    assert_eq!(
        client.try_charge(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(client.get_prepaid_balance(&id), 1000);
    assert_eq!(_token_client.balance(&contract_id), 1000);

    // Resume after 500 seconds
    env.ledger().set_timestamp(START + PERIOD + 500);
    client.resume(&id);

    // Not yet due because due date was shifted
    assert_eq!(client.charge(&id), 0);
    assert_eq!(client.get_prepaid_balance(&id), 1000);

    // Advance to shifted due date
    env.ledger().set_timestamp(START + PERIOD + 500 + PERIOD);
    let billed = client.charge(&id);
    assert_eq!(billed, AMOUNT);
    assert_eq!(client.get_prepaid_balance(&id), 750);
}

#[test]
fn charge_catchup_with_prepaid_balance() {
    let (env, token, contract_id, client, accounts, token_client) = setup!();
    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    let id = client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);

    // Deposit 500 (covers 2 periods)
    client.deposit(&id, &500);
    assert_eq!(client.get_prepaid_balance(&id), 500);
    let sub_wallet_before = token_client.balance(subscriber);

    // Advance 3 full periods
    env.ledger().set_timestamp(START + 3 * PERIOD);

    // Catch up 3 periods: 2 from prepaid balance, 1 from wallet
    let billed = client.charge_catchup(&id, &3);
    assert_eq!(billed, 3 * AMOUNT);

    assert_eq!(client.get_prepaid_balance(&id), 0);
    assert_eq!(token_client.balance(&contract_id), 0);
    assert_eq!(token_client.balance(provider), 3 * AMOUNT);
    assert_eq!(token_client.balance(subscriber), sub_wallet_before - AMOUNT);
}

#[test]
fn exact_period_balance_debits_then_lapses_and_top_up_recovers() {
    let (env, token, subscriber, provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let token_client = TokenClient::new(&env, &token);
    client.deposit(&id, &(AMOUNT * 2));
    assert_eq!(
        client.get_subscription(&id).prepaid_balance,
        Some(AMOUNT * 2)
    );

    env.ledger().set_timestamp(START + PERIOD);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(AMOUNT));

    env.ledger().set_timestamp(START + PERIOD * 2);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(0));
    assert_eq!(token_client.balance(&provider), AMOUNT * 2);

    env.ledger().set_timestamp(START + PERIOD * 3);
    assert_eq!(client.charge(&id), 0);
    assert_eq!(
        client.get_subscription(&id).status,
        SubscriptionStatus::PastDue
    );
    assert_eq!(
        client.get_subscription(&id).last_charged,
        START + PERIOD * 2
    );

    client.deposit(&id, &AMOUNT);
    assert_eq!(client.charge(&id), AMOUNT);
    let recovered = client.get_subscription(&id);
    assert_eq!(recovered.status, SubscriptionStatus::Active);
    assert_eq!(recovered.last_charged, START + PERIOD * 3);
    assert_eq!(recovered.prepaid_balance, Some(0));
    assert_eq!(token_client.balance(&provider), AMOUNT * 3);
    assert_eq!(token_client.balance(&contract_id), 0);
    assert_eq!(token_client.balance(&subscriber), 10_000 - AMOUNT * 3);
}

#[test]
fn cancellation_refunds_exact_remainder_and_withdrawal_preserves_mode() {
    let (env, token, subscriber, provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let token_client = TokenClient::new(&env, &token);
    client.deposit(&id, &(AMOUNT + 83));
    env.ledger().set_timestamp(START + PERIOD);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(83));
    client.cancel(&id);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(0));
    assert_eq!(token_client.balance(&subscriber), 10_000 - AMOUNT);
    assert_eq!(token_client.balance(&provider), AMOUNT);
    assert_eq!(token_client.balance(&contract_id), 0);

    let second = client.subscribe(&subscriber, &provider, &token, &AMOUNT, &PERIOD);
    client.deposit(&second, &500);
    assert_eq!(client.withdraw_balance(&second, &200), 300);
    assert_eq!(client.get_subscription(&second).prepaid_balance, Some(300));
}

#[test]
fn retry_limit_auto_cancellation_refunds_the_exact_remainder() {
    let (env, token, subscriber, provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let token_client = TokenClient::new(&env, &token);
    client.deposit(&id, &(AMOUNT + 37));
    env.ledger().set_timestamp(START + PERIOD);
    assert_eq!(client.charge(&id), AMOUNT);
    for attempt in 2..=4 {
        env.ledger().set_timestamp(START + PERIOD * attempt);
        assert_eq!(client.charge(&id), 0);
    }
    let sub = client.get_subscription(&id);
    assert_eq!(sub.status, SubscriptionStatus::Cancelled);
    assert_eq!(sub.prepaid_balance, Some(0));
    assert_eq!(token_client.balance(&subscriber), 10_000 - AMOUNT);
    assert_eq!(token_client.balance(&provider), AMOUNT);
    assert_eq!(token_client.balance(&contract_id), 0);
}

#[test]
fn catchup_cannot_bypass_prepaid_one_period_lapse_policy() {
    let (env, _token, _subscriber, _provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    client.deposit(&id, &(AMOUNT * 2));
    env.ledger().set_timestamp(START + PERIOD * 2);
    let err = client.try_charge_catchup(&id, &2).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(
        client.get_subscription(&id).prepaid_balance,
        Some(AMOUNT * 2)
    );
}

#[test]
fn prepaid_events_report_exact_amounts_and_resulting_balances() {
    let (env, _token, _subscriber, _provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    client.deposit(&id, &300);
    let deposits = events_named(&env, &contract_id, "deposited");
    assert_eq!(deposits.len(), 1);
    assert_eq!(data_field(&deposits[0].1, "amount"), sc_i128(300));
    assert_eq!(data_field(&deposits[0].1, "balance_after"), sc_i128(300));

    env.ledger().set_timestamp(START + PERIOD);
    assert_eq!(client.charge(&id), AMOUNT);
    let debits = events_named(&env, &contract_id, "balance_debited");
    assert_eq!(debits.len(), 1);
    assert_eq!(data_field(&debits[0].1, "amount"), sc_i128(AMOUNT));
    assert_eq!(data_field(&debits[0].1, "balance_after"), sc_i128(50));

    client.cancel(&id);
    let refunds = events_named(&env, &contract_id, "balance_refunded");
    assert_eq!(refunds.len(), 1);
    assert_eq!(data_field(&refunds[0].1, "amount"), sc_i128(50));
    assert_eq!(data_field(&refunds[0].1, "balance_after"), sc_i128(0));
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(0));
}

fn events_named(
    env: &Env,
    contract: &Address,
    name: &str,
) -> std::vec::Vec<(std::vec::Vec<ScVal>, ScVal)> {
    let name = ScVal::Symbol(ScSymbol::try_from(name).unwrap());
    env.events()
        .all()
        .filter_by_contract(contract)
        .events()
        .iter()
        .filter_map(|event| {
            let ContractEventBody::V0(body) = &event.body;
            if body.topics.first() != Some(&name) {
                return None;
            }
            Some((body.topics[1..].to_vec(), body.data.clone()))
        })
        .collect()
}

fn data_field(data: &ScVal, field: &str) -> ScVal {
    let ScVal::Map(Some(entries)) = data else {
        panic!("event data is not a map: {data:?}");
    };
    let key = ScVal::Symbol(ScSymbol::try_from(field).unwrap());
    entries
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.val.clone())
        .unwrap_or_else(|| panic!("event data has no `{field}` field: {data:?}"))
}

fn sc_i128(value: i128) -> ScVal {
    ScVal::I128(Int128Parts {
        hi: (value >> 64) as i64,
        lo: value as u64,
    })
}

#[test]
fn prepaid_pause_and_resume_shift_due_date_without_debit() {
    let (env, token, subscriber, provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let token_client = TokenClient::new(&env, &token);
    client.deposit(&id, &AMOUNT);
    client.pause(&id);
    env.ledger().set_timestamp(START + PERIOD * 5);
    let paused = client.try_charge(&id).unwrap_err().unwrap();
    assert_eq!(paused, ForgeError::InvalidInput);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(AMOUNT));
    assert_eq!(token_client.balance(&provider), 0);

    client.resume(&id);
    let due = client.get_subscription(&id).last_charged + PERIOD;
    env.ledger().set_timestamp(due);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(0));
    assert_eq!(token_client.balance(&subscriber), 10_000 - AMOUNT);
}
