use crate::{SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::xdr::{ContractEventBody, Int128Parts, ScSymbol, ScVal};
use soroban_sdk::{Address, Env};

const START: u64 = 1_000_000;
const PERIOD: u64 = 1_000;
const AMOUNT: i128 = 250;

fn setup() -> (Env, Address, Address, Address, Address, u64) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(START);
    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let subscriber = Address::generate(&env);
    let provider = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&subscriber, &10_000);
    let contract_id = env.register(SubscriptionPayments, ());
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let id = client.subscribe(&subscriber, &provider, &token, &AMOUNT, &PERIOD);
    (env, token, subscriber, provider, contract_id, id)
}

#[test]
fn deposit_failure_leaves_balance_and_tokens_unchanged() {
    let (env, token, subscriber, _provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let token_client = TokenClient::new(&env, &token);
    let before = client.get_subscription(&id);
    let user_before = token_client.balance(&subscriber);
    let result = client.try_deposit(&id, &(user_before + 1));
    assert_eq!(
        result.unwrap_err().unwrap(),
        ForgeError::TokenTransferFailed
    );
    assert_eq!(client.get_subscription(&id), before);
    assert_eq!(token_client.balance(&subscriber), user_before);
    assert_eq!(token_client.balance(&contract_id), 0);
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

fn assert_total_supply_conserved(
    token_client: &TokenClient<'_>,
    subscriber: &Address,
    contract_id: &Address,
    provider: &Address,
    expected_total: i128,
    expected_prepaid: Option<i128>,
) {
    let sub_bal = token_client.balance(subscriber);
    let contract_bal = token_client.balance(contract_id);
    let provider_bal = token_client.balance(provider);
    assert!(sub_bal >= 0);
    assert!(contract_bal >= 0);
    assert!(provider_bal >= 0);
    assert_eq!(contract_bal, expected_prepaid.unwrap_or(0));
    assert_eq!(sub_bal + contract_bal + provider_bal, expected_total);
}

#[test]
fn deposit_followed_by_charge_proves_exact_balance_and_token_deltas() {
    let (env, token, subscriber, provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let token_client = TokenClient::new(&env, &token);
    let initial_supply = token_client.balance(&subscriber);

    let api_metric = soroban_sdk::Symbol::new(&env, "api_calls");
    let quotas = soroban_sdk::vec![
        &env,
        crate::MetricQuota {
            metric: api_metric.clone(),
            included_units: 100,
            overage_price: 35,
            bucket_units: 50,
            max_overage_units: Some(200),
        },
    ];
    client.set_quotas(&id, &quotas);

    // First deposit: +400 into prepaid custody.
    assert_eq!(client.deposit(&id, &400), 400);
    assert_eq!(token_client.balance(&subscriber), initial_supply - 400);
    assert_eq!(token_client.balance(&contract_id), 400);
    assert_eq!(token_client.balance(&provider), 0);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(400),
    );

    // Second deposit: +250 -> total 650 in prepaid custody.
    assert_eq!(client.deposit(&id, &250), 650);
    assert_eq!(token_client.balance(&subscriber), initial_supply - 650);
    assert_eq!(token_client.balance(&contract_id), 650);
    assert_eq!(token_client.balance(&provider), 0);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(650),
    );

    // Period 1 with metered overage: 100 included + 55 overage = 2 buckets * 35 = 70 overage.
    // Total charge = 250 + 70 = 320. Remaining prepaid balance = 650 - 320 = 330.
    client.record_usage(&id, &api_metric, &155);
    assert_eq!(client.quote_period(&id), AMOUNT + 70);
    env.ledger().set_timestamp(START + PERIOD);
    let charged_p1 = client.charge(&id);
    assert_eq!(charged_p1, AMOUNT + 70);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(330));
    assert_eq!(token_client.balance(&subscriber), initial_supply - 650);
    assert_eq!(token_client.balance(&contract_id), 330);
    assert_eq!(token_client.balance(&provider), 320);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(330),
    );

    // Period 2 with no overage: flat charge = 250. Remaining prepaid balance = 330 - 250 = 80.
    env.ledger().set_timestamp(START + PERIOD * 2);
    let charged_p2 = client.charge(&id);
    assert_eq!(charged_p2, AMOUNT);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(80));
    assert_eq!(token_client.balance(&subscriber), initial_supply - 650);
    assert_eq!(token_client.balance(&contract_id), 80);
    assert_eq!(token_client.balance(&provider), 320 + AMOUNT);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(80),
    );
}

#[test]
fn withdrawal_and_refund_boundaries_prevent_negative_balances_and_conserve_tokens() {
    let (env, token, subscriber, provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let token_client = TokenClient::new(&env, &token);
    let initial_supply = token_client.balance(&subscriber);

    // Pull-mode subscription (prepaid_balance == None) rejects withdrawal.
    let before_pull = client.get_subscription(&id);
    assert_eq!(
        client.try_withdraw_balance(&id, &1).unwrap_err().unwrap(),
        ForgeError::InsufficientFunds
    );
    assert_eq!(client.get_subscription(&id), before_pull);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        None,
    );

    // Opt into prepaid mode with 400 tokens.
    assert_eq!(client.deposit(&id, &400), 400);
    let funded_state = client.get_subscription(&id);

    // Non-positive and over-balance withdrawals fail without mutating state or tokens.
    for invalid_amount in [0, -1, -400, i128::MIN] {
        assert_eq!(
            client
                .try_withdraw_balance(&id, &invalid_amount)
                .unwrap_err()
                .unwrap(),
            ForgeError::InvalidInput
        );
        assert_eq!(client.get_subscription(&id), funded_state);
    }
    for excessive_amount in [401, 10_000, i128::MAX] {
        assert_eq!(
            client
                .try_withdraw_balance(&id, &excessive_amount)
                .unwrap_err()
                .unwrap(),
            ForgeError::InsufficientFunds
        );
        assert_eq!(client.get_subscription(&id), funded_state);
    }
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(400),
    );

    // Partial withdrawal followed by exact boundary withdrawal to 0.
    assert_eq!(client.withdraw_balance(&id, &150), 250);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(250));
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(250),
    );

    assert_eq!(client.withdraw_balance(&id, &250), 0);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(0));
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(0),
    );

    // Withdrawing 1 from Some(0) fails with InsufficientFunds and never goes negative.
    assert_eq!(
        client.try_withdraw_balance(&id, &1).unwrap_err().unwrap(),
        ForgeError::InsufficientFunds
    );
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(0));

    // Top-up from Some(0), charge 1 period, then cancel refunds exact remainder.
    assert_eq!(client.deposit(&id, &(AMOUNT + 95)), AMOUNT + 95);
    env.ledger().set_timestamp(START + PERIOD);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(95));

    client.cancel(&id);
    let cancelled = client.get_subscription(&id);
    assert_eq!(cancelled.status, SubscriptionStatus::Cancelled);
    assert_eq!(cancelled.prepaid_balance, Some(0));
    assert_eq!(token_client.balance(&subscriber), initial_supply - AMOUNT);
    assert_eq!(token_client.balance(&provider), AMOUNT);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(0),
    );

    // Post-cancel withdrawal and duplicate cancel both reject with InvalidInput.
    assert_eq!(
        client.try_withdraw_balance(&id, &1).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_cancel(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(client.get_subscription(&id), cancelled);

    // Pooled custody isolation: even when another subscription holds contract custody,
    // over-withdrawing on one subscription is rejected with InsufficientFunds and never
    // creates a negative prepaid_balance or drains sibling custody.
    let sub_a = client.subscribe(&subscriber, &provider, &token, &AMOUNT, &PERIOD);
    let sub_b = client.subscribe(&subscriber, &provider, &token, &AMOUNT, &PERIOD);
    assert_eq!(client.deposit(&sub_a, &200), 200);
    assert_eq!(client.deposit(&sub_b, &500), 500);
    assert_eq!(token_client.balance(&contract_id), 700);
    assert_eq!(
        client
            .try_withdraw_balance(&sub_a, &201)
            .unwrap_err()
            .unwrap(),
        ForgeError::InsufficientFunds
    );
    assert_eq!(client.get_subscription(&sub_a).prepaid_balance, Some(200));
    assert_eq!(client.get_subscription(&sub_b).prepaid_balance, Some(500));
    assert_eq!(token_client.balance(&contract_id), 700);
}

#[test]
fn catchup_vs_prepaid_interaction_and_overdue_recovery_conserve_balances() {
    let (env, token, subscriber, provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let token_client = TokenClient::new(&env, &token);
    let initial_supply = token_client.balance(&subscriber);

    // 1. In pull mode, catch-up bills bounded periods and conserves tokens.
    env.ledger().set_timestamp(START + PERIOD * 3);
    assert_eq!(client.charge_catchup(&id, &2), AMOUNT * 2);
    assert_eq!(
        client.get_subscription(&id).last_charged,
        START + PERIOD * 2
    );
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        None,
    );

    // 2. Once opted into prepaid mode, catch-up is rejected in Active, PastDue, Paused, and Cancelled.
    // Deposit 200 (less than AMOUNT = 250) so period 3 charge will enter PastDue.
    assert_eq!(client.deposit(&id, &200), 200);
    let active_prepaid = client.get_subscription(&id);
    assert_eq!(
        client.try_charge_catchup(&id, &1).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(client.get_subscription(&id), active_prepaid);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(200),
    );

    // 3. Overdue attempt at START + PERIOD * 3 fails (200 < 250) and enters PastDue without debiting.
    assert_eq!(client.charge(&id), 0);
    let past_due_1 = client.get_subscription(&id);
    assert_eq!(past_due_1.status, SubscriptionStatus::PastDue);
    assert_eq!(past_due_1.failed_attempts, 1);
    assert_eq!(past_due_1.prepaid_balance, Some(200));
    assert_eq!(past_due_1.last_charged, START + PERIOD * 2);
    assert_eq!(
        client.try_charge_catchup(&id, &1).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(client.get_subscription(&id), past_due_1);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(200),
    );

    // 4. Top-up while PastDue (+300 -> 500) and recover via `charge` across two overdue periods.
    env.ledger().set_timestamp(START + PERIOD * 4);
    assert_eq!(client.deposit(&id, &300), 500);
    assert_eq!(client.charge(&id), AMOUNT);
    let recovered = client.get_subscription(&id);
    assert_eq!(recovered.status, SubscriptionStatus::Active);
    assert_eq!(recovered.failed_attempts, 0);
    assert_eq!(recovered.last_charged, START + PERIOD * 3);
    assert_eq!(recovered.prepaid_balance, Some(250));
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(250),
    );

    // Second overdue period (period 4) can now be settled sequentially via `charge`.
    assert_eq!(client.charge(&id), AMOUNT);
    let current = client.get_subscription(&id);
    assert_eq!(current.status, SubscriptionStatus::Active);
    assert_eq!(current.last_charged, START + PERIOD * 4);
    assert_eq!(current.prepaid_balance, Some(0));
    assert_eq!(token_client.balance(&provider), AMOUNT * 4);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(0),
    );
}

#[test]
fn invalid_transitions_leave_balances_and_subscription_status_unchanged() {
    let (env, token, subscriber, provider, contract_id, id) = setup();
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let token_client = TokenClient::new(&env, &token);
    let initial_supply = token_client.balance(&subscriber);

    client.deposit(&id, &300);
    let snapshot_active = client.get_subscription(&id);
    let sub_bal_active = token_client.balance(&subscriber);
    let contract_bal_active = token_client.balance(&contract_id);
    let provider_bal_active = token_client.balance(&provider);

    // Invalid deposits: 0, negative, overflow, and exceeding subscriber balance.
    for invalid_deposit in [0, -1, -300] {
        assert_eq!(
            client
                .try_deposit(&id, &invalid_deposit)
                .unwrap_err()
                .unwrap(),
            ForgeError::InvalidInput
        );
        assert_eq!(client.get_subscription(&id), snapshot_active);
    }
    assert_eq!(
        client.try_deposit(&id, &i128::MAX).unwrap_err().unwrap(),
        ForgeError::ArithmeticOverflow
    );
    assert_eq!(
        client
            .try_deposit(&id, &(sub_bal_active + 1))
            .unwrap_err()
            .unwrap(),
        ForgeError::TokenTransferFailed
    );
    // Resume while Active is rejected.
    assert_eq!(
        client.try_resume(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(client.get_subscription(&id), snapshot_active);
    assert_eq!(token_client.balance(&subscriber), sub_bal_active);
    assert_eq!(token_client.balance(&contract_id), contract_bal_active);
    assert_eq!(token_client.balance(&provider), provider_bal_active);

    // Pause subscription and verify invalid actions while Paused leave state and balances untouched.
    client.pause(&id);
    env.ledger().set_timestamp(START + PERIOD * 2);
    let snapshot_paused = client.get_subscription(&id);
    assert_eq!(
        client.try_pause(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_charge(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_charge_catchup(&id, &1).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client
            .try_set_quotas(&id, &soroban_sdk::Vec::new(&env))
            .unwrap_err()
            .unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(client.get_subscription(&id), snapshot_paused);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(300),
    );

    // Resume, drain balance to 100, advance to due date, and trigger PastDue.
    client.resume(&id);
    assert_eq!(client.withdraw_balance(&id, &200), 100);
    let next_due = client.get_subscription(&id).last_charged + PERIOD;
    env.ledger().set_timestamp(next_due);
    assert_eq!(client.charge(&id), 0);
    let snapshot_past_due = client.get_subscription(&id);
    assert_eq!(snapshot_past_due.status, SubscriptionStatus::PastDue);
    assert_eq!(snapshot_past_due.prepaid_balance, Some(100));

    // Invalid transitions while PastDue (pause, resume, charge_catchup, set_quotas) preserve state and balances.
    assert_eq!(
        client.try_pause(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_resume(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_charge_catchup(&id, &1).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client
            .try_set_quotas(&id, &soroban_sdk::Vec::new(&env))
            .unwrap_err()
            .unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(client.get_subscription(&id), snapshot_past_due);
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(100),
    );

    // Cancel from PastDue refunds the remaining 100 and locks out further mutations.
    client.cancel(&id);
    let snapshot_cancelled = client.get_subscription(&id);
    assert_eq!(snapshot_cancelled.status, SubscriptionStatus::Cancelled);
    assert_eq!(snapshot_cancelled.prepaid_balance, Some(0));
    for res in [
        client.try_deposit(&id, &100).err(),
        client.try_withdraw_balance(&id, &1).err(),
        client.try_charge(&id).err(),
        client.try_charge_catchup(&id, &1).err(),
        client.try_pause(&id).err(),
        client.try_resume(&id).err(),
        client.try_cancel(&id).err(),
    ] {
        assert_eq!(res.unwrap().unwrap(), ForgeError::InvalidInput);
        assert_eq!(client.get_subscription(&id), snapshot_cancelled);
    }
    assert_total_supply_conserved(
        &token_client,
        &subscriber,
        &contract_id,
        &provider,
        initial_supply,
        Some(0),
    );
}
