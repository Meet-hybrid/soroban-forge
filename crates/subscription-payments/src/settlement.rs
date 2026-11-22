use super::*;
use crate::prepaid::{data_field, events_named, sc_i128};
use proptest::{prelude::*, test_runner::RngSeed};
use soroban_sdk::{
    testutils::{storage::Persistent as _, Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    token::StellarAssetClient,
    IntoVal,
};

struct Case {
    env: Env,
    contract: Address,
    token: Address,
    payer: Address,
    provider: Address,
    id: u64,
}

impl Case {
    fn new(amount: i128, period: u64, funds: i128, prepaid: bool) -> Self {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_timestamp(1000);
        let payer = Address::generate(&env);
        let provider = Address::generate(&env);
        let token = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();
        let contract = env.register(SubscriptionPayments, ());
        StellarAssetClient::new(&env, &token).mint(&payer, &funds);
        let c = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract);
        let id = c.subscribe(&payer, &provider, &token, &amount, &period);
        if prepaid {
            c.deposit(&id, &funds);
        }
        Self {
            env,
            contract,
            token,
            payer,
            provider,
            id,
        }
    }

    fn client(&self) -> SorobanForgeSubscriptionPaymentsClient<'_> {
        SorobanForgeSubscriptionPaymentsClient::new(&self.env, &self.contract)
    }

    fn balances(&self) -> [i128; 3] {
        let token = StellarAssetClient::new(&self.env, &self.token);
        [&self.payer, &self.provider, &self.contract].map(|a| token.balance(a))
    }

    fn elapsed(&self, seconds: u64) {
        self.env.ledger().set_timestamp(1000 + seconds);
    }
}

#[test]
fn plan_proration_and_carry_across_billing_periods() {
    for prepaid in [false, true] {
        for (elapsed, price, credit) in [(0, 30, 100), (51, 200, 49), (100, 50, 0)] {
            let w = Case::new(100, 100, 1000, prepaid);
            let c = w.client();
            w.elapsed(elapsed);
            assert_eq!(c.change_plan(&w.id, &price), credit);
            let events = events_named(&w.env, &w.contract, "plan_changed");
            assert_eq!(events.len(), 1);
            for (key, expected) in [
                ("old_amount", 100),
                ("new_amount", price),
                ("credit", credit),
            ] {
                assert_eq!(data_field(&events[0].1, key), sc_i128(expected));
            }
            let mut carry = credit;
            for period in 1..=5 {
                w.elapsed(period * 100);
                let due = price - price.min(carry);
                assert_eq!(c.quote_period(&w.id), due);
                let before = w.balances();
                assert_eq!(c.charge(&w.id), due);
                assert_eq!(w.balances()[1], before[1] + due);
                assert_eq!(c.get_subscription(&w.id).last_charged, 1000 + period * 100);
                carry -= price.min(carry);
            }
        }
    }
}

#[test]
fn changing_an_unbilled_price_twice_does_not_credit_the_same_period_twice() {
    let w = Case::new(100, 100, 1000, false);
    let c = w.client();
    assert_eq!(c.change_plan(&w.id, &200), 100);
    assert_eq!(c.change_plan(&w.id, &150), 0);
    w.elapsed(100);
    assert_eq!(c.charge(&w.id), 50);
}

#[test]
fn plan_change_requires_both_signers_positive_new_price_and_active_state() {
    let w = Case::new(100, 10, 1, false);
    let c = w.client();
    for signer in [&w.payer, &w.provider] {
        w.env.mock_auths(&[MockAuth {
            address: signer,
            invoke: &MockAuthInvoke {
                contract: &w.contract,
                fn_name: "change_plan",
                args: (w.id, 50_i128).into_val(&w.env),
                sub_invokes: &[],
            },
        }]);
        assert!(c.try_change_plan(&w.id, &50).is_err());
        assert_eq!(c.get_subscription(&w.id).amount, 100);
    }
    w.env.mock_all_auths_allowing_non_root_auth();
    for price in [0, -1, 100] {
        assert_eq!(
            c.try_change_plan(&w.id, &price),
            Err(Ok(ForgeError::InvalidInput))
        );
    }
    c.pause(&w.id);
    assert_eq!(
        c.try_change_plan(&w.id, &50),
        Err(Ok(ForgeError::InvalidInput))
    );
    c.resume(&w.id);
    w.elapsed(10);
    c.charge(&w.id);
    assert_eq!(
        c.get_subscription(&w.id).status,
        SubscriptionStatus::PastDue
    );
    assert_eq!(
        c.try_change_plan(&w.id, &50),
        Err(Ok(ForgeError::InvalidInput))
    );
    c.cancel(&w.id);
    assert_eq!(
        c.try_change_plan(&w.id, &50),
        Err(Ok(ForgeError::InvalidInput))
    );
}

#[test]
fn preview_matches_settlement_without_renewing_records_or_changing_balances() {
    let w = Case::new(90, 10, 1000, false);
    let c = w.client();
    c.change_plan(&w.id, &30);
    assert_eq!(c.preview_catchup(&w.id, &3).chargeable_amount, 0);
    w.elapsed(50);
    w.env.ledger().set_sequence_number(5);
    let keys = [
        DataKey::Subscription(w.id),
        DataKey::PlanCredit(w.id),
        DataKey::SubscriberSubscriptions(w.payer.clone()),
        DataKey::ProviderSubscriptions(w.provider.clone()),
    ];
    let ttls = || {
        w.env.as_contract(&w.contract, || {
            keys.each_ref()
                .map(|key| w.env.storage().persistent().get_ttl(key))
        })
    };
    let before = (ttls(), w.balances(), c.get_subscription(&w.id));
    let none = c.preview_catchup(&w.id, &0);
    assert_eq!((none.chargeable_amount, none.arrears_remaining), (0, 60));
    let preview = c.preview_catchup(&w.id, &4);
    assert_eq!(
        (
            preview.chargeable_amount,
            preview.prepaid_consumed,
            preview.arrears_remaining
        ),
        (30, 0, 30)
    );
    assert_eq!(before, (ttls(), w.balances(), c.get_subscription(&w.id)));
    assert_eq!(c.charge_catchup(&w.id, &4), preview.chargeable_amount);
    assert_eq!(c.get_subscription(&w.id).status, preview.resulting_status);
    assert_eq!(c.charge_catchup(&w.id, &4), preview.arrears_remaining);
    assert_eq!(c.preview_catchup(&w.id, &4).arrears_remaining, 0);
}

#[test]
fn preview_prices_open_usage_once_and_preserves_rounding() {
    let w = Case::new(100, 10, 1000, false);
    let c = w.client();
    let metric = Symbol::new(&w.env, "calls");
    c.set_quotas(
        &w.id,
        &Vec::from_array(
            &w.env,
            [MetricQuota {
                metric: metric.clone(),
                included_units: 1,
                bucket_units: 3,
                overage_price: 7,
                max_overage_units: None,
            }],
        ),
    );
    c.record_usage(&w.id, &metric, &5);
    w.elapsed(30);
    assert_eq!(c.preview_catchup(&w.id, &0).arrears_remaining, 314);
    let p = c.preview_catchup(&w.id, &2);
    assert_eq!((p.chargeable_amount, p.arrears_remaining), (214, 100));
    assert_eq!(c.get_usage(&w.id, &metric).units, 5);
    assert_eq!(c.charge_catchup(&w.id, &2), p.chargeable_amount);
    assert_eq!(c.charge_catchup(&w.id, &1), p.arrears_remaining);
}

#[test]
fn fractional_credit_is_floored_even_for_large_prices() {
    let w = Case::new(i128::MAX, 3, 1, false);
    w.elapsed(2);
    assert_eq!(w.client().change_plan(&w.id, &1), i128::MAX / 3);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, rng_seed: RngSeed::Fixed(709), ..ProptestConfig::default() })]
    #[test]
    fn compare_catchup_and_single_charge_policy(
        amount in 2i128..10000, period in 1u64..100, elapsed_periods in 0u32..8, offset in 0u64..100,
        limit in 0u32..8, funded_periods in 0u32..8,
    ) {
        for prepaid in [false, true] {
            compare_settlement(amount, period, elapsed_periods, offset, limit, funded_periods, prepaid);
        }
    }
}

fn compare_settlement(
    amount: i128,
    period: u64,
    elapsed_periods: u32,
    offset: u64,
    limit: u32,
    funded_periods: u32,
    prepaid: bool,
) {
    let funds = amount * i128::from(funded_periods) + 1;
    let single = Case::new(amount, period, funds, prepaid);
    let batch = Case::new(amount, period, funds, prepaid);
    let elapsed = u64::from(elapsed_periods) * period + offset % period;
    single.elapsed(elapsed);
    batch.elapsed(elapsed);
    let n = elapsed_periods.min(limit);
    let mut events = std::vec::Vec::new();
    let mut paid = 0;
    for _ in 0..n {
        let debit = single.client().charge(&single.id);
        events.extend(events_named(&single.env, &single.contract, "charged"));
        paid += debit;
        if debit == 0 {
            break;
        }
    }
    let before = batch.balances();
    let preview = batch.client().try_preview_catchup(&batch.id, &limit);
    let result = batch.client().try_charge_catchup(&batch.id, &limit);
    let batch_events = events_named(&batch.env, &batch.contract, "charged");
    let sub = single.client().get_subscription(&single.id);
    let caught = batch.client().get_subscription(&batch.id);
    assert_eq!(paid, amount * i128::from(n.min(funded_periods)));
    assert_eq!(events.len(), n.min(funded_periods) as usize);
    let remaining = funds - paid;
    assert_eq!(
        single.balances(),
        if prepaid {
            [0, paid, remaining]
        } else {
            [remaining, paid, 0]
        }
    );
    assert_eq!(
        sub.last_charged,
        1000 + u64::from(n.min(funded_periods)) * period
    );
    if prepaid || n > funded_periods {
        let error = if prepaid {
            ForgeError::InvalidInput
        } else {
            ForgeError::TokenTransferFailed
        };
        assert_eq!(result, Err(Ok(error)));
        if prepaid {
            assert_eq!(preview, Err(Ok(ForgeError::InvalidInput)));
        } else {
            assert_eq!(
                preview.unwrap().unwrap().chargeable_amount,
                i128::from(n) * amount
            );
        }
        assert_eq!(batch.balances(), before);
        assert_eq!(caught.last_charged, 1000);
        assert_eq!(caught.status, SubscriptionStatus::Active);
        assert!(batch_events.is_empty());
        if n > funded_periods {
            assert_eq!(sub.status, SubscriptionStatus::PastDue);
        }
    } else {
        assert_eq!(result, Ok(Ok(paid)));
        assert_eq!(preview.unwrap().unwrap().chargeable_amount, paid);
        assert_eq!(batch.balances(), single.balances());
        assert_eq!(caught.last_charged, sub.last_charged);
        assert_eq!(caught.status, sub.status);
        assert_eq!(batch_events, events);
    }
}
