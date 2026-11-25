//! Tests for free-trial subscriptions (`subscribe_with_trial`, `get_trial_end`,
//! trial conversion on `charge` / `charge_catchup`, pause/resume extension,
//! cancellation rules, prepaid mode, and events).

use crate::{
    DataKey, SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus,
    TRIAL_MAX_SECONDS,
};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{
    storage::Persistent, Address as _, Events as _, Ledger as _, MockAuth, MockAuthInvoke,
};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::xdr::{ContractEventBody, ScSymbol, ScVal};
use soroban_sdk::{Address, Env, IntoVal, InvokeError};

const START: u64 = 1_000_000;
const PERIOD: u64 = 86_400; // 1 day
const AMOUNT: i128 = 500;
const TRIAL: u64 = 7 * PERIOD; // 7-day trial
const MINTED: i128 = 1_000_000;

struct World {
    env: Env,
    token: Address,
    contract_id: Address,
    subscriber: Address,
    provider: Address,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();

    let subscriber = Address::generate(&env);
    let provider = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&subscriber, &MINTED);

    let contract_id = env.register(SubscriptionPayments, ());
    World {
        env,
        token,
        contract_id,
        subscriber,
        provider,
    }
}

impl World {
    fn client(&self) -> SorobanForgeSubscriptionPaymentsClient<'_> {
        SorobanForgeSubscriptionPaymentsClient::new(&self.env, &self.contract_id)
    }

    fn balance(&self, address: &Address) -> i128 {
        TokenClient::new(&self.env, &self.token).balance(address)
    }

    fn subscribe_trial(&self, trial_seconds: u64) -> u64 {
        self.client().subscribe_with_trial(
            &self.subscriber,
            &self.provider,
            &self.token,
            &AMOUNT,
            &PERIOD,
            &trial_seconds,
        )
    }

    /// Collect `(subscription_id, trial_end)` pairs for events with `event_name`
    /// emitted by this contract in the most recent invocation.
    fn trial_events(&self, event_name: &str) -> std::vec::Vec<(u64, u64)> {
        let topic_name = ScVal::Symbol(ScSymbol::try_from(event_name).unwrap());
        self.env
            .events()
            .all()
            .filter_by_contract(&self.contract_id)
            .events()
            .iter()
            .filter_map(|event| {
                let ContractEventBody::V0(body) = &event.body;
                if body.topics.first() != Some(&topic_name) {
                    return None;
                }
                let Some(ScVal::U64(sub_id)) = body.topics.get(1) else {
                    panic!("missing u64 subscription_id topic: {:?}", body.topics);
                };
                let ScVal::U64(trial_end) = data_field(&body.data, "trial_end") else {
                    panic!("trial_end field is not u64: {:?}", body.data);
                };
                Some((*sub_id, trial_end))
            })
            .collect()
    }

    /// Count events with `event_name` emitted by this contract in the last call.
    fn count_events(&self, event_name: &str) -> usize {
        let topic_name = ScVal::Symbol(ScSymbol::try_from(event_name).unwrap());
        self.env
            .events()
            .all()
            .filter_by_contract(&self.contract_id)
            .events()
            .iter()
            .filter(|event| {
                let ContractEventBody::V0(body) = &event.body;
                body.topics.first() == Some(&topic_name)
            })
            .count()
    }
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

// ── validation, creation, and authorization ──────────────────────────────────

#[test]
fn subscribe_with_trial_rejects_zero_and_above_cap_trial_seconds() {
    let w = setup();
    let client = w.client();

    let err = client
        .try_subscribe_with_trial(&w.subscriber, &w.provider, &w.token, &AMOUNT, &PERIOD, &0)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);

    let err = client
        .try_subscribe_with_trial(
            &w.subscriber,
            &w.provider,
            &w.token,
            &AMOUNT,
            &PERIOD,
            &(TRIAL_MAX_SECONDS + 1),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(client.get_subscription_count(), 0);
}

#[test]
fn subscribe_with_trial_accepts_exact_cap_and_rejects_invalid_terms_and_overflow() {
    let w = setup();
    let client = w.client();

    let err = client
        .try_subscribe_with_trial(&w.subscriber, &w.provider, &w.token, &0, &PERIOD, &TRIAL)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);

    let err = client
        .try_subscribe_with_trial(&w.subscriber, &w.provider, &w.token, &AMOUNT, &0, &TRIAL)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);

    let id = w.subscribe_trial(TRIAL_MAX_SECONDS);
    assert_eq!(client.get_trial_end(&id), Some(START + TRIAL_MAX_SECONDS));

    // Timestamp overflow does not create a partial subscription.
    w.env.ledger().set_timestamp(u64::MAX - 10);
    let err = client
        .try_subscribe_with_trial(&w.subscriber, &w.provider, &w.token, &AMOUNT, &PERIOD, &11)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::ArithmeticOverflow);
    assert_eq!(client.get_subscription_count(), 1);
}

#[test]
fn subscribe_with_trial_emits_trial_started_and_populates_indexes() {
    let w = setup();
    let client = w.client();

    let id = w.subscribe_trial(TRIAL);
    let expected_end = START + TRIAL;

    assert_eq!(
        w.trial_events("trial_started"),
        std::vec![(id, expected_end)]
    );
    assert_eq!(client.get_trial_end(&id), Some(expected_end));

    let sub = client.get_subscription(&id);
    assert_eq!(sub.status, SubscriptionStatus::Active);
    assert_eq!(sub.last_charged, START);
    assert_eq!(sub.subscriber, w.subscriber);
    assert_eq!(sub.provider, w.provider);
    assert_eq!(sub.amount, AMOUNT);
    assert_eq!(sub.period, PERIOD);
    assert_eq!(sub.prepaid_balance, None);

    assert_eq!(
        client
            .subscriptions_for_subscriber(&w.subscriber, &0, &10)
            .len(),
        1
    );
    assert_eq!(
        client
            .subscriptions_for_provider(&w.provider, &0, &10)
            .len(),
        1
    );
}

#[test]
fn get_trial_end_is_none_for_regular_and_missing_subscriptions() {
    let w = setup();
    let client = w.client();
    let id = client.subscribe(&w.subscriber, &w.provider, &w.token, &AMOUNT, &PERIOD);
    assert_eq!(client.get_trial_end(&id), None);
    assert_eq!(client.get_trial_end(&999), None);
}

#[test]
fn subscribe_with_trial_requires_subscriber_auth() {
    let w = setup();
    let client = w.client();

    // Provider auth must be rejected.
    w.env.mock_auths(&[MockAuth {
        address: &w.provider,
        invoke: &MockAuthInvoke {
            contract: &w.contract_id,
            fn_name: "subscribe_with_trial",
            args: (&w.subscriber, &w.provider, &w.token, AMOUNT, PERIOD, TRIAL).into_val(&w.env),
            sub_invokes: &[],
        },
    }]);
    let res = client.try_subscribe_with_trial(
        &w.subscriber,
        &w.provider,
        &w.token,
        &AMOUNT,
        &PERIOD,
        &TRIAL,
    );
    assert!(matches!(res, Err(Err(InvokeError::Abort))));

    // Subscriber auth succeeds.
    w.env.mock_auths(&[MockAuth {
        address: &w.subscriber,
        invoke: &MockAuthInvoke {
            contract: &w.contract_id,
            fn_name: "subscribe_with_trial",
            args: (&w.subscriber, &w.provider, &w.token, AMOUNT, PERIOD, TRIAL).into_val(&w.env),
            sub_invokes: &[],
        },
    }]);
    let id = client
        .try_subscribe_with_trial(
            &w.subscriber,
            &w.provider,
            &w.token,
            &AMOUNT,
            &PERIOD,
            &TRIAL,
        )
        .expect("outer ok")
        .expect("contract ok");
    assert_eq!(client.get_trial_end(&id), Some(START + TRIAL));
}

// ── charging at trial end ± 1 second and post-trial billing ──────────────────

#[test]
fn charge_at_trial_end_minus_one_bills_nothing_and_emits_no_events() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    w.env.ledger().set_timestamp(trial_end - 1);
    assert_eq!(client.charge(&id), 0);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(w.count_events("charged"), 0);
    assert_eq!(w.balance(&w.provider), 0);
    assert_eq!(w.balance(&w.subscriber), MINTED);
    assert_eq!(client.get_subscription(&id).last_charged, START);
    assert_eq!(client.get_trial_end(&id), Some(trial_end));
}

#[test]
fn charge_at_exact_trial_end_converts_and_bills_first_period() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    w.env.ledger().set_timestamp(trial_end);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id, trial_end)]
    );
    assert_eq!(w.count_events("charged"), 1);
    assert_eq!(w.balance(&w.provider), AMOUNT);
    assert_eq!(w.balance(&w.subscriber), MINTED - AMOUNT);
    assert_eq!(client.get_subscription(&id).last_charged, trial_end);
    assert_eq!(client.get_trial_end(&id), Some(trial_end));

    // One second later: already billed the first period, next due is trial_end + PERIOD.
    w.env.ledger().set_timestamp(trial_end + 1);
    assert_eq!(client.charge(&id), 0);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(w.count_events("charged"), 0);

    // One second before second period: still 0.
    w.env.ledger().set_timestamp(trial_end + PERIOD - 1);
    assert_eq!(client.charge(&id), 0);

    // At trial_end + PERIOD: second period billed; TrialConverted NOT emitted again.
    w.env.ledger().set_timestamp(trial_end + PERIOD);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(w.count_events("charged"), 1);
    assert_eq!(w.balance(&w.provider), AMOUNT * 2);
    assert_eq!(
        client.get_subscription(&id).last_charged,
        trial_end + PERIOD
    );
}

#[test]
fn charge_at_trial_end_plus_one_converts_and_anchors_cadence_to_trial_end() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    w.env.ledger().set_timestamp(trial_end + 1);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id, trial_end)]
    );
    assert_eq!(w.count_events("charged"), 1);
    assert_eq!(w.balance(&w.provider), AMOUNT);
    // Cadence anchors to trial_end, not trial_end + 1.
    assert_eq!(client.get_subscription(&id).last_charged, trial_end);

    // Next charge is due at trial_end + PERIOD.
    w.env.ledger().set_timestamp(trial_end + PERIOD);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(w.balance(&w.provider), AMOUNT * 2);
}

#[test]
fn charge_does_not_retroactively_bill_trial_time() {
    let w = setup();
    let client = w.client();
    // TRIAL is 7 * PERIOD, so 7 periods elapse during the trial alone.
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    // Advance 2 periods past trial_end (9 periods since creation).
    w.env.ledger().set_timestamp(trial_end + PERIOD * 2);

    // Exactly 3 charges are due: trial_end, trial_end + PERIOD, trial_end + 2*PERIOD.
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id, trial_end)]
    );
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(client.charge(&id), 0);

    assert_eq!(w.balance(&w.provider), AMOUNT * 3);
    assert_eq!(
        client.get_subscription(&id).last_charged,
        trial_end + PERIOD * 2
    );
}

#[test]
fn trial_shorter_than_period_and_small_timestamp_works_without_underflow() {
    let w = setup();
    let client = w.client();
    w.env.ledger().set_timestamp(100);
    // trial_end = 150 < PERIOD (86_400).
    let id = w.subscribe_trial(50);
    assert_eq!(client.get_trial_end(&id), Some(150));

    w.env.ledger().set_timestamp(149);
    assert_eq!(client.charge(&id), 0);

    w.env.ledger().set_timestamp(150);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(w.trial_events("trial_converted"), std::vec![(id, 150)]);
    assert_eq!(client.get_subscription(&id).last_charged, 150);
}

#[test]
fn failed_charge_at_trial_end_does_not_convert_until_retry_succeeds() {
    let w = setup();
    let client = w.client();
    let broke_subscriber = Address::generate(&w.env);
    StellarAssetClient::new(&w.env, &w.token).mint(&broke_subscriber, &(AMOUNT - 1));

    let id = client.subscribe_with_trial(
        &broke_subscriber,
        &w.provider,
        &w.token,
        &AMOUNT,
        &PERIOD,
        &TRIAL,
    );
    let trial_end = START + TRIAL;

    w.env.ledger().set_timestamp(trial_end);
    assert_eq!(client.charge(&id), 0);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    let sub = client.get_subscription(&id);
    assert_eq!(sub.status, SubscriptionStatus::PastDue);
    assert_eq!(sub.failed_attempts, 1);
    assert_eq!(sub.last_charged, START);

    // Fund subscriber and retry.
    StellarAssetClient::new(&w.env, &w.token).mint(&broke_subscriber, &AMOUNT);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id, trial_end)]
    );
    assert_eq!(w.count_events("charged"), 1);
    let sub = client.get_subscription(&id);
    assert_eq!(sub.status, SubscriptionStatus::Active);
    assert_eq!(sub.failed_attempts, 0);
    assert_eq!(sub.last_charged, trial_end);
}

// ── charge_catchup across the trial boundary ─────────────────────────────────

#[test]
fn charge_catchup_bills_nothing_during_trial_and_zero_max_periods_does_not_convert() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    w.env.ledger().set_timestamp(trial_end - 1);
    assert_eq!(client.charge_catchup(&id, &10), 0);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(w.count_events("charged"), 0);
    assert_eq!(w.balance(&w.provider), 0);

    // max_periods == 0 at trial_end must be a pure no-op.
    w.env.ledger().set_timestamp(trial_end);
    assert_eq!(client.charge_catchup(&id, &0), 0);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(client.get_subscription(&id).last_charged, START);
}

#[test]
fn charge_catchup_at_trial_end_and_across_boundary_bills_post_trial_periods_only() {
    let w = setup();
    let client = w.client();
    let id_at_end = w.subscribe_trial(TRIAL);
    let id_plus_one = w.subscribe_trial(TRIAL);
    let id_multi = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    // Exactly at trial_end: bills 1 period.
    w.env.ledger().set_timestamp(trial_end);
    assert_eq!(client.charge_catchup(&id_at_end, &10), AMOUNT);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id_at_end, trial_end)]
    );
    assert_eq!(w.count_events("charged"), 1);
    assert_eq!(client.get_subscription(&id_at_end).last_charged, trial_end);

    // At trial_end + 1: bills 1 period anchored to trial_end.
    w.env.ledger().set_timestamp(trial_end + 1);
    assert_eq!(client.charge_catchup(&id_plus_one, &10), AMOUNT);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id_plus_one, trial_end)]
    );
    assert_eq!(
        client.get_subscription(&id_plus_one).last_charged,
        trial_end
    );

    // At trial_end + 2.5 * PERIOD (with 7-period trial): bills 3 post-trial periods
    // (trial_end, trial_end + PERIOD, trial_end + 2 * PERIOD), never the 7 trial periods.
    w.env
        .ledger()
        .set_timestamp(trial_end + PERIOD * 2 + PERIOD / 2);
    assert_eq!(client.charge_catchup(&id_multi, &10), AMOUNT * 3);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id_multi, trial_end)]
    );
    assert_eq!(w.count_events("charged"), 3);
    assert_eq!(
        client.get_subscription(&id_multi).last_charged,
        trial_end + PERIOD * 2
    );
    assert_eq!(w.balance(&w.provider), AMOUNT * 5);
}

#[test]
fn charge_catchup_respects_max_periods_and_rolls_back_on_transfer_failure() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    // 4 periods due (trial_end + 0..=3), cap first catchup to 2.
    w.env.ledger().set_timestamp(trial_end + PERIOD * 3);
    assert_eq!(client.charge_catchup(&id, &2), AMOUNT * 2);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id, trial_end)]
    );
    assert_eq!(w.count_events("charged"), 2);
    assert_eq!(
        client.get_subscription(&id).last_charged,
        trial_end + PERIOD
    );

    // Second catchup bills remaining 2 periods without emitting TrialConverted again.
    assert_eq!(client.charge_catchup(&id, &10), AMOUNT * 2);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(w.count_events("charged"), 2);
    assert_eq!(
        client.get_subscription(&id).last_charged,
        trial_end + PERIOD * 3
    );

    // Rollback check: subscriber can only cover 1 of 2 due periods.
    let low_user = Address::generate(&w.env);
    StellarAssetClient::new(&w.env, &w.token).mint(&low_user, &(AMOUNT + 1));
    w.env.ledger().set_timestamp(START);
    let id_fail =
        client.subscribe_with_trial(&low_user, &w.provider, &w.token, &AMOUNT, &PERIOD, &TRIAL);
    w.env.ledger().set_timestamp(trial_end + PERIOD);
    let err = client
        .try_charge_catchup(&id_fail, &2)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::TokenTransferFailed);
    assert_eq!(w.balance(&low_user), AMOUNT + 1);
    assert_eq!(client.get_subscription(&id_fail).last_charged, START);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(w.count_events("charged"), 0);
}

// ── cancellation inside the trial ────────────────────────────────────────────

#[test]
fn cancel_during_trial_costs_nothing_in_pull_mode() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    w.env.ledger().set_timestamp(trial_end - 1);
    client.cancel(&id);

    assert_eq!(w.count_events("cancelled"), 1);
    assert_eq!(w.balance(&w.subscriber), MINTED);
    assert_eq!(w.balance(&w.provider), 0);
    assert_eq!(
        client.get_subscription(&id).status,
        SubscriptionStatus::Cancelled
    );

    // Cannot be charged after trial end once cancelled.
    w.env.ledger().set_timestamp(trial_end + PERIOD);
    let err = client.try_charge(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn cancel_during_trial_refunds_prepaid_balance_in_full() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    let deposit_amount = AMOUNT * 3 + 77;
    client.deposit(&id, &deposit_amount);
    assert_eq!(w.balance(&w.contract_id), deposit_amount);
    assert_eq!(w.balance(&w.subscriber), MINTED - deposit_amount);

    w.env.ledger().set_timestamp(trial_end - 1);
    client.cancel(&id);

    assert_eq!(w.count_events("balance_refunded"), 1);
    assert_eq!(w.count_events("cancelled"), 1);
    assert_eq!(w.balance(&w.subscriber), MINTED);
    assert_eq!(w.balance(&w.provider), 0);
    assert_eq!(w.balance(&w.contract_id), 0);
    let sub = client.get_subscription(&id);
    assert_eq!(sub.status, SubscriptionStatus::Cancelled);
    assert_eq!(sub.prepaid_balance, Some(0));
}

#[test]
fn cancel_by_provider_and_cancel_while_paused_during_trial_refund_in_full() {
    let w = setup();
    let client = w.client();

    let id1 = w.subscribe_trial(TRIAL);
    client.deposit(&id1, &(AMOUNT * 2));
    w.env.ledger().set_timestamp(START + PERIOD);
    client.cancel_by_provider(&id1);
    assert_eq!(client.get_subscription(&id1).prepaid_balance, Some(0));
    assert_eq!(w.balance(&w.subscriber), MINTED);
    assert_eq!(w.balance(&w.provider), 0);

    let id2 = w.subscribe_trial(TRIAL);
    client.deposit(&id2, &(AMOUNT * 2));
    client.pause(&id2);
    w.env.ledger().set_timestamp(START + PERIOD * 2);
    client.cancel(&id2);
    assert_eq!(
        client.get_subscription(&id2).status,
        SubscriptionStatus::Cancelled
    );
    assert_eq!(client.get_subscription(&id2).prepaid_balance, Some(0));
    assert_eq!(w.balance(&w.subscriber), MINTED);
    assert_eq!(w.balance(&w.contract_id), 0);
}

// ── pause and resume inside the trial ────────────────────────────────────────

#[test]
fn pause_and_resume_during_trial_extends_trial_end_by_paused_duration() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    // Pause 2 days into the 7-day trial (5 days left).
    let pause_ts = START + PERIOD * 2;
    let pause_duration = PERIOD * 3;
    w.env.ledger().set_timestamp(pause_ts);
    client.pause(&id);

    // Charging while paused is rejected.
    w.env.ledger().set_timestamp(pause_ts + PERIOD);
    assert_eq!(
        client.try_charge(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_charge_catchup(&id, &5).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );

    // Resume after 3 days -> trial_end extends by 3 days.
    w.env.ledger().set_timestamp(pause_ts + pause_duration);
    client.resume(&id);
    let extended_end = trial_end + pause_duration;
    assert_eq!(client.get_trial_end(&id), Some(extended_end));

    // At original trial_end and extended_end - 1: still in trial.
    w.env.ledger().set_timestamp(trial_end);
    assert_eq!(client.charge(&id), 0);
    w.env.ledger().set_timestamp(extended_end - 1);
    assert_eq!(client.charge(&id), 0);
    assert_eq!(w.balance(&w.provider), 0);

    // At extended_end: converts and bills first period with extended_end.
    w.env.ledger().set_timestamp(extended_end);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id, extended_end)]
    );
    assert_eq!(w.balance(&w.provider), AMOUNT);
    assert_eq!(client.get_subscription(&id).last_charged, extended_end);
}

#[test]
fn pause_across_original_trial_end_preserves_remaining_trial_and_post_conversion_pause_does_not_shift_trial_end(
) {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    // Pause 1 day into a 7-day trial, stay paused for 10 days (past original trial_end).
    let pause_ts = START + PERIOD;
    let pause_duration = PERIOD * 10;
    w.env.ledger().set_timestamp(pause_ts);
    client.pause(&id);
    w.env.ledger().set_timestamp(pause_ts + pause_duration);
    client.resume(&id);

    let extended_end = trial_end + pause_duration;
    assert_eq!(client.get_trial_end(&id), Some(extended_end));

    // Still has 6 days of trial remaining after resume.
    w.env.ledger().set_timestamp(extended_end - 1);
    assert_eq!(client.charge(&id), 0);

    w.env.ledger().set_timestamp(extended_end);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id, extended_end)]
    );

    // Pause and resume AFTER conversion: shifts next billing due date only,
    // leaving historical trial_end untouched.
    let post_pause = extended_end + 100;
    let post_duration = 500;
    w.env.ledger().set_timestamp(post_pause);
    client.pause(&id);
    w.env.ledger().set_timestamp(post_pause + post_duration);
    client.resume(&id);

    assert_eq!(client.get_trial_end(&id), Some(extended_end));
    assert_eq!(
        client.get_subscription(&id).last_charged,
        extended_end + post_duration
    );
    w.env
        .ledger()
        .set_timestamp(extended_end + PERIOD + post_duration - 1);
    assert_eq!(client.charge(&id), 0);
    w.env
        .ledger()
        .set_timestamp(extended_end + PERIOD + post_duration);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
}

// ── prepaid mode across the trial boundary ───────────────────────────────────

#[test]
fn prepaid_mode_across_trial_boundary_debits_at_trial_end_and_recovers_from_insufficient_balance() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    // Deposit less than one period during the trial.
    client.deposit(&id, &(AMOUNT - 100));
    w.env.ledger().set_timestamp(trial_end - 1);
    assert_eq!(client.charge(&id), 0);
    assert_eq!(w.balance(&w.provider), 0);
    assert_eq!(
        client.get_subscription(&id).prepaid_balance,
        Some(AMOUNT - 100)
    );

    // At trial_end: insufficient prepaid balance lapses to PastDue without converting.
    w.env.ledger().set_timestamp(trial_end);
    assert_eq!(client.charge(&id), 0);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(
        client.get_subscription(&id).status,
        SubscriptionStatus::PastDue
    );
    assert_eq!(client.get_subscription(&id).last_charged, START);

    // Top up prepaid balance and retry: converts and debits first period.
    client.deposit(&id, &(AMOUNT + 100));
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id, trial_end)]
    );
    assert_eq!(w.count_events("charged"), 1);
    assert_eq!(w.count_events("balance_debited"), 1);
    assert_eq!(w.balance(&w.provider), AMOUNT);
    let sub = client.get_subscription(&id);
    assert_eq!(sub.status, SubscriptionStatus::Active);
    assert_eq!(sub.last_charged, trial_end);
    assert_eq!(sub.prepaid_balance, Some(AMOUNT));

    // Second period debits remaining prepaid balance without re-emitting TrialConverted.
    w.env.ledger().set_timestamp(trial_end + PERIOD);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(w.trial_events("trial_converted"), std::vec![]);
    assert_eq!(w.balance(&w.provider), AMOUNT * 2);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(0));
}

#[test]
fn prepaid_auto_topup_runs_on_trial_conversion() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);
    let trial_end = START + TRIAL;

    client.deposit(&id, &100);
    client.configure_auto_topup(&id, &AMOUNT, &AMOUNT);

    // During trial: no top-up, no charge.
    w.env.ledger().set_timestamp(trial_end - 1);
    assert_eq!(client.charge(&id), 0);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(100));

    // At trial_end: auto-topup pulls AMOUNT, then period debit settles AMOUNT.
    w.env.ledger().set_timestamp(trial_end);
    assert_eq!(client.charge(&id), AMOUNT);
    assert_eq!(w.count_events("auto_topup_triggered"), 1);
    assert_eq!(
        w.trial_events("trial_converted"),
        std::vec![(id, trial_end)]
    );
    assert_eq!(w.balance(&w.provider), AMOUNT);
    assert_eq!(client.get_subscription(&id).prepaid_balance, Some(100));
}

#[test]
fn touch_ttl_refreshes_trial_end_entry() {
    let w = setup();
    let client = w.client();
    let id = w.subscribe_trial(TRIAL);

    let before = w.env.as_contract(&w.contract_id, || {
        w.env.storage().persistent().get_ttl(&DataKey::TrialEnd(id))
    });
    client.touch_ttl(&id);
    let after = w.env.as_contract(&w.contract_id, || {
        w.env.storage().persistent().get_ttl(&DataKey::TrialEnd(id))
    });
    assert!(after >= before);
    assert_eq!(client.get_trial_end(&id), Some(START + TRIAL));
}
