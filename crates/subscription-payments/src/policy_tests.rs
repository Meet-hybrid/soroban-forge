use super::*;
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{vec, IntoVal, Symbol};

struct World {
    env: Env,
    contract: Address,
    token: Address,
    subscriber: Address,
    provider: Address,
    id: u64,
}

impl World {
    fn new(funded: bool) -> Self {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_timestamp(1000);
        let token = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();
        let subscriber = Address::generate(&env);
        let provider = Address::generate(&env);
        if funded {
            StellarAssetClient::new(&env, &token).mint(&subscriber, &10000);
        }
        let contract = env.register(SubscriptionPayments, ());
        let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract);
        let id = client.subscribe(&subscriber, &provider, &token, &100, &100);
        Self {
            env,
            contract,
            token,
            subscriber,
            provider,
            id,
        }
    }
    fn client(&self) -> SorobanForgeSubscriptionPaymentsClient<'_> {
        SorobanForgeSubscriptionPaymentsClient::new(&self.env, &self.contract)
    }
    fn at(&self, timestamp: u64) {
        self.env.ledger().set_timestamp(timestamp);
    }
}

#[test]
fn grace_retries_do_not_cancel_or_advance_until_recovery() {
    let w = World::new(false);
    let client = w.client();
    client.set_grace_period(&w.id, &50);
    w.at(1100);
    for _ in 0..5 {
        assert_eq!(client.charge(&w.id), 0);
    }
    let sub = client.get_subscription(&w.id);
    assert_eq!(sub.status, SubscriptionStatus::PastDue);
    assert_eq!(sub.failed_attempts, 5);
    assert_eq!(sub.last_charged, 1000);
    assert_eq!(client.grace_deadline(&w.id), Some(1150));
    StellarAssetClient::new(&w.env, &w.token).mint(&w.subscriber, &100);
    w.at(1149);
    assert_eq!(client.charge(&w.id), 100);
    assert_eq!(client.charge(&w.id), 0);
    assert_eq!(TokenClient::new(&w.env, &w.token).balance(&w.provider), 100);
    let recovered = client.get_subscription(&w.id);
    assert_eq!(recovered.status, SubscriptionStatus::Active);
    assert_eq!(recovered.failed_attempts, 0);
    assert_eq!(recovered.past_due_since, None);
    assert_eq!(client.grace_deadline(&w.id), None);
}

#[test]
fn grace_exact_expiry_cancels_and_refunds_prepaid() {
    let w = World::new(true);
    let client = w.client();
    client.deposit(&w.id, &40);
    client.set_grace_period(&w.id, &50);
    w.at(1100);
    assert_eq!(client.charge(&w.id), 0);
    w.at(1150);
    assert_eq!(client.charge(&w.id), 0);
    let sub = client.get_subscription(&w.id);
    assert_eq!(sub.status, SubscriptionStatus::Cancelled);
    assert_eq!(sub.prepaid_balance, Some(0));
    assert_eq!(TokenClient::new(&w.env, &w.token).balance(&w.contract), 0);
    assert_eq!(
        TokenClient::new(&w.env, &w.token).balance(&w.subscriber),
        10000
    );
}

#[test]
fn grace_policy_cap_and_subscriber_authorization() {
    let w = World::new(false);
    let client = w.client();
    assert!(client
        .try_set_grace_period(&w.id, &(MAX_GRACE_SECONDS + 1))
        .is_err());
    w.env.mock_auths(&[]);
    assert!(client.try_set_grace_period(&w.id, &10).is_err());
    w.env.mock_all_auths();
    client.set_grace_period(&w.id, &0);
    w.at(1100);
    for _ in 0..3 {
        client.charge(&w.id);
    }
    assert_eq!(
        client.get_subscription(&w.id).status,
        SubscriptionStatus::Cancelled
    );
}

#[test]
fn provider_rotation_preserves_terms_balances_and_sorted_indexes() {
    let w = World::new(true);
    let client = w.client();
    client.deposit(&w.id, &500);
    let new = Address::generate(&w.env);
    let replacement = Address::generate(&w.env);
    client.propose_provider(&w.id, &new);
    client.propose_provider(&w.id, &replacement);
    assert_eq!(client.pending_provider(&w.id), Some(replacement));
    assert!(client.try_accept_provider(&w.id, &new).is_err());
    client.cancel_provider_change(&w.id);
    assert_eq!(client.pending_provider(&w.id), None);
    let later = client.subscribe(&w.subscriber, &new, &w.token, &100, &100);
    client.propose_provider(&w.id, &new);
    let before = client.get_subscription(&w.id);
    w.env.mock_auths(&[]);
    assert!(client.try_accept_provider(&w.id, &new).is_err());
    w.env.mock_all_auths();
    client.accept_provider(&w.id, &new);
    let mut expected = before;
    expected.provider = new.clone();
    assert_eq!(client.get_subscription(&w.id), expected);
    assert_eq!(client.pending_provider(&w.id), None);
    assert!(client
        .subscriptions_for_provider(&w.provider, &0, &10)
        .is_empty());
    let indexed = client.subscriptions_for_provider(&new, &0, &10);
    assert_eq!(indexed.get(0).unwrap().subscription_id, w.id);
    assert_eq!(indexed.get(1).unwrap().subscription_id, later);
    w.at(1100);
    w.env.mock_auths(&[MockAuth {
        address: &w.provider,
        invoke: &MockAuthInvoke {
            contract: &w.contract,
            fn_name: "charge",
            args: (w.id,).into_val(&w.env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_charge(&w.id).is_err());
    w.env.mock_all_auths();
    assert_eq!(client.charge(&w.id), 100);
    assert_eq!(TokenClient::new(&w.env, &w.token).balance(&new), 100);
    assert_eq!(TokenClient::new(&w.env, &w.token).balance(&w.provider), 0);
}

fn quota(env: &Env) -> Vec<MetricQuota> {
    vec![
        env,
        MetricQuota {
            metric: Symbol::new(env, "api"),
            included_units: 10,
            overage_price: 2,
            bucket_units: 1,
            max_overage_units: None,
        },
    ]
}

#[test]
fn rollover_consumes_carried_units_before_base_and_caps_multiple_periods() {
    let w = World::new(true);
    let client = w.client();
    let metric = Symbol::new(&w.env, "api");
    client.set_quotas(&w.id, &quota(&w.env));
    assert!(client
        .try_set_rollover_cap(&w.id, &metric, &u64::MAX)
        .is_err());
    client.set_rollover_cap(&w.id, &metric, &10);
    w.at(1100);
    assert_eq!(client.charge(&w.id), 100);
    assert_eq!(client.rollover_remaining(&w.id, &metric), 10);
    client.record_usage(&w.id, &metric, &6);
    assert_eq!(client.rollover_remaining(&w.id, &metric), 4);
    client.record_usage(&w.id, &metric, &15);
    assert_eq!(client.rollover_remaining(&w.id, &metric), 0);
    assert_eq!(client.quote_period(&w.id), 102);
    assert!(client.try_set_rollover_cap(&w.id, &metric, &5).is_err());
    w.at(1200);
    assert_eq!(client.charge(&w.id), 102);
    assert_eq!(client.rollover_remaining(&w.id, &metric), 0);
    client.set_rollover_cap(&w.id, &metric, &25);
    w.at(1500);
    assert_eq!(client.charge_catchup(&w.id, &3), 300);
    assert_eq!(client.rollover_remaining(&w.id, &metric), 25);
    assert_eq!(client.get_subscription(&w.id).amount, 100);
    assert_eq!(client.get_subscription(&w.id).period, 100);
    client.set_rollover_cap(&w.id, &metric, &0);
    assert_eq!(client.rollover_remaining(&w.id, &metric), 0);
    client.record_usage(&w.id, &metric, &11);
    assert_eq!(client.quote_period(&w.id), 102);
}

#[test]
fn failed_charge_preserves_rollover_and_meter_for_retry() {
    let w = World::new(true);
    let client = w.client();
    let metric = Symbol::new(&w.env, "api");
    client.set_quotas(&w.id, &quota(&w.env));
    client.set_rollover_cap(&w.id, &metric, &10);
    client.deposit(&w.id, &100);
    w.at(1100);
    client.charge(&w.id);
    client.record_usage(&w.id, &metric, &21);
    w.at(1200);
    client.charge(&w.id);
    assert_eq!(client.quote_period(&w.id), 102);
    assert_eq!(client.get_usage(&w.id, &metric).units, 21);
    client.deposit(&w.id, &102);
    assert_eq!(client.charge(&w.id), 102);
    assert_eq!(client.rollover_remaining(&w.id, &metric), 0);
}

#[test]
fn provider_proposal_requires_subscriber_and_cancellation_invalidates_acceptance() {
    let w = World::new(true);
    let client = w.client();
    let new = Address::generate(&w.env);
    w.env.mock_auths(&[]);
    assert!(client.try_propose_provider(&w.id, &new).is_err());
    assert_eq!(client.pending_provider(&w.id), None);
    w.env.mock_all_auths();
    client.propose_provider(&w.id, &new);
    w.env.mock_auths(&[]);
    assert!(client.try_cancel_provider_change(&w.id).is_err());
    w.env.mock_all_auths();
    client.cancel(&w.id);
    assert_eq!(client.pending_provider(&w.id), None);
    assert!(client.try_accept_provider(&w.id, &new).is_err());
}

#[test]
fn funded_at_exact_grace_deadline_still_cancels_without_billing() {
    let w = World::new(false);
    let client = w.client();
    client.set_grace_period(&w.id, &1);
    w.at(1100);
    client.charge(&w.id);
    StellarAssetClient::new(&w.env, &w.token).mint(&w.subscriber, &100);
    w.at(1101);
    client.charge(&w.id);
    assert_eq!(
        client.get_subscription(&w.id).status,
        SubscriptionStatus::Cancelled
    );
    assert_eq!(
        TokenClient::new(&w.env, &w.token).balance(&w.subscriber),
        100
    );
    assert_eq!(TokenClient::new(&w.env, &w.token).balance(&w.provider), 0);
}
