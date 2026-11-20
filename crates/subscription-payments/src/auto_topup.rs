//! Prepaid auto-topup: configuration, authorization, the top-up during
//! `charge`, and the safe fallback when the top-up transfer is rejected.

use crate::{
    AutoTopup, SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus,
};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{
    Address as _, Events as _, IssuerFlags, Ledger as _, MockAuth, MockAuthInvoke,
};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::xdr::{ContractEventBody, Int128Parts, ScSymbol, ScVal};
use soroban_sdk::{Address, Env, IntoVal, InvokeError};

const START: u64 = 1_000_000;
const PERIOD: u64 = 1_000;
const AMOUNT: i128 = 250;
const MINTED: i128 = 10_000;

struct World {
    env: Env,
    token: Address,
    subscriber: Address,
    provider: Address,
    contract_id: Address,
    id: u64,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(START);
    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    // Revocable issuer, so tests can deauthorize the subscriber's account.
    sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    let token = sac.address();
    let subscriber = Address::generate(&env);
    let provider = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&subscriber, &MINTED);
    let contract_id = env.register(SubscriptionPayments, ());
    let id = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id).subscribe(
        &subscriber,
        &provider,
        &token,
        &AMOUNT,
        &PERIOD,
    );
    World {
        env,
        token,
        subscriber,
        provider,
        contract_id,
        id,
    }
}

impl World {
    fn client(&self) -> SorobanForgeSubscriptionPaymentsClient<'_> {
        SorobanForgeSubscriptionPaymentsClient::new(&self.env, &self.contract_id)
    }

    fn balance(&self, address: &Address) -> i128 {
        TokenClient::new(&self.env, &self.token).balance(address)
    }

    /// (subscriber, provider, contract) token balances.
    fn balances(&self) -> (i128, i128, i128) {
        (
            self.balance(&self.subscriber),
            self.balance(&self.provider),
            self.balance(&self.contract_id),
        )
    }

    fn advance_periods(&self, periods: u64) {
        self.env.ledger().set_timestamp(START + PERIOD * periods);
    }

    /// `AutoTopupTriggered` events of the most recent invocation, as
    /// `(amount, balance_after)` pairs.
    fn topup_events(&self) -> std::vec::Vec<(ScVal, ScVal)> {
        let name = ScVal::Symbol(ScSymbol::try_from("auto_topup_triggered").unwrap());
        let id = ScVal::U64(self.id);
        self.env
            .events()
            .all()
            .filter_by_contract(&self.contract_id)
            .events()
            .iter()
            .filter_map(|event| {
                let ContractEventBody::V0(body) = &event.body;
                if body.topics.first() != Some(&name) {
                    return None;
                }
                assert_eq!(body.topics.get(1), Some(&id), "subscription_id topic");
                Some((
                    data_field(&body.data, "amount"),
                    data_field(&body.data, "balance_after"),
                ))
            })
            .collect()
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

fn sc_i128(value: i128) -> ScVal {
    ScVal::I128(Int128Parts {
        hi: (value >> 64) as i64,
        lo: value as u64,
    })
}

// -----------------------------------------------------------------------
// Configuration and authorization
// -----------------------------------------------------------------------

#[test]
fn configure_stores_replaces_and_disable_removes_the_setting() {
    let w = setup();
    let client = w.client();
    assert_eq!(client.get_auto_topup(&w.id), None);

    client.configure_auto_topup(&w.id, &0, &500);
    assert_eq!(
        client.get_auto_topup(&w.id),
        Some(AutoTopup {
            trigger_threshold: 0,
            topup_amount: 500,
        })
    );

    client.configure_auto_topup(&w.id, &1_000, &750);
    assert_eq!(
        client.get_auto_topup(&w.id),
        Some(AutoTopup {
            trigger_threshold: 1_000,
            topup_amount: 750,
        })
    );

    client.disable_auto_topup(&w.id);
    assert_eq!(client.get_auto_topup(&w.id), None);
    // Disabling again is a no-op.
    client.disable_auto_topup(&w.id);
    assert_eq!(client.get_auto_topup(&w.id), None);
}

#[test]
fn configure_rejects_invalid_amounts_missing_and_cancelled_subscriptions() {
    let w = setup();
    let client = w.client();

    for (threshold, topup) in [(0, 0), (0, -1), (-1, 500), (i128::MIN, 500), (0, i128::MIN)] {
        assert_eq!(
            client
                .try_configure_auto_topup(&w.id, &threshold, &topup)
                .unwrap_err()
                .unwrap(),
            ForgeError::InvalidInput,
            "threshold {threshold}, topup {topup}"
        );
    }
    assert_eq!(client.get_auto_topup(&w.id), None);

    assert_eq!(
        client
            .try_configure_auto_topup(&(w.id + 1), &0, &500)
            .unwrap_err()
            .unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client
            .try_disable_auto_topup(&(w.id + 1))
            .unwrap_err()
            .unwrap(),
        ForgeError::NotFound
    );

    client.cancel(&w.id);
    assert_eq!(
        client
            .try_configure_auto_topup(&w.id, &0, &500)
            .unwrap_err()
            .unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(client.get_auto_topup(&w.id), None);
}

#[test]
fn configure_and_disable_require_the_subscriber_signature() {
    let w = setup();
    let client = w.client();
    let stranger = Address::generate(&w.env);

    for signer in [&w.provider, &stranger] {
        w.env.mock_auths(&[MockAuth {
            address: signer,
            invoke: &MockAuthInvoke {
                contract: &w.contract_id,
                fn_name: "configure_auto_topup",
                args: (w.id, 0_i128, 500_i128).into_val(&w.env),
                sub_invokes: &[],
            },
        }]);
        assert!(matches!(
            client.try_configure_auto_topup(&w.id, &0, &500),
            Err(Err(InvokeError::Abort))
        ));
        assert_eq!(client.get_auto_topup(&w.id), None);
    }

    w.env.mock_auths(&[MockAuth {
        address: &w.subscriber,
        invoke: &MockAuthInvoke {
            contract: &w.contract_id,
            fn_name: "configure_auto_topup",
            args: (w.id, 0_i128, 500_i128).into_val(&w.env),
            sub_invokes: &[],
        },
    }]);
    client.configure_auto_topup(&w.id, &0, &500);
    assert!(client.get_auto_topup(&w.id).is_some());

    for signer in [&w.provider, &stranger] {
        w.env.mock_auths(&[MockAuth {
            address: signer,
            invoke: &MockAuthInvoke {
                contract: &w.contract_id,
                fn_name: "disable_auto_topup",
                args: (w.id,).into_val(&w.env),
                sub_invokes: &[],
            },
        }]);
        assert!(matches!(
            client.try_disable_auto_topup(&w.id),
            Err(Err(InvokeError::Abort))
        ));
        assert!(client.get_auto_topup(&w.id).is_some());
    }

    w.env.mock_auths(&[MockAuth {
        address: &w.subscriber,
        invoke: &MockAuthInvoke {
            contract: &w.contract_id,
            fn_name: "disable_auto_topup",
            args: (w.id,).into_val(&w.env),
            sub_invokes: &[],
        },
    }]);
    client.disable_auto_topup(&w.id);
    assert_eq!(client.get_auto_topup(&w.id), None);
}

// -----------------------------------------------------------------------
// Top-up during charge
// -----------------------------------------------------------------------

#[test]
fn shortfall_is_topped_up_before_the_debit() {
    let w = setup();
    let client = w.client();
    client.deposit(&w.id, &100);
    client.configure_auto_topup(&w.id, &0, &500);

    w.advance_periods(1);
    assert_eq!(client.charge(&w.id), AMOUNT);
    assert_eq!(
        w.topup_events(),
        std::vec![(sc_i128(500), sc_i128(600))],
        "the event reports the top-up and the balance before the debit"
    );

    let subscription = client.get_subscription(&w.id);
    assert_eq!(subscription.status, SubscriptionStatus::Active);
    assert_eq!(subscription.last_charged, START + PERIOD);
    assert_eq!(subscription.failed_attempts, 0);
    assert_eq!(subscription.prepaid_balance, Some(100 + 500 - AMOUNT));
    assert_eq!(
        w.balances(),
        (MINTED - 100 - 500, AMOUNT, 100 + 500 - AMOUNT),
        "subscriber funds custody, custody pays the provider"
    );

    // The next period is covered by the remaining balance: no top-up.
    w.advance_periods(2);
    assert_eq!(client.charge(&w.id), AMOUNT);
    assert!(w.topup_events().is_empty());
    assert_eq!(client.get_subscription(&w.id).prepaid_balance, Some(100));
}

#[test]
fn balance_below_the_safety_threshold_is_topped_up_even_when_it_covers_the_period() {
    let w = setup();
    let client = w.client();
    client.deposit(&w.id, &300);
    client.configure_auto_topup(&w.id, &1_000, &800);

    // 300 covers the 250 period but sits below the 1,000 floor.
    w.advance_periods(1);
    assert_eq!(client.charge(&w.id), AMOUNT);
    assert_eq!(w.topup_events(), std::vec![(sc_i128(800), sc_i128(1_100))]);
    assert_eq!(
        client.get_subscription(&w.id).prepaid_balance,
        Some(1_100 - AMOUNT)
    );

    // 850 is still below the floor: one more top-up, never more than one
    // per charge.
    w.advance_periods(2);
    assert_eq!(client.charge(&w.id), AMOUNT);
    assert_eq!(w.topup_events(), std::vec![(sc_i128(800), sc_i128(1_650))]);
    assert_eq!(
        client.get_subscription(&w.id).prepaid_balance,
        Some(1_650 - AMOUNT)
    );

    // 1,400 is at or above the floor: plain debit.
    w.advance_periods(3);
    assert_eq!(client.charge(&w.id), AMOUNT);
    assert!(w.topup_events().is_empty());
    assert_eq!(
        client.get_subscription(&w.id).prepaid_balance,
        Some(1_400 - AMOUNT)
    );
    assert_eq!(
        w.balances(),
        (MINTED - 300 - 1_600, AMOUNT * 3, 1_150),
        "custody plus payouts conserve every token pulled from the subscriber"
    );
}

#[test]
fn charge_before_the_period_never_tops_up() {
    let w = setup();
    let client = w.client();
    client.configure_auto_topup(&w.id, &1_000, &500);
    client.deposit(&w.id, &1);

    w.env.ledger().set_timestamp(START + PERIOD - 1);
    assert_eq!(client.charge(&w.id), 0);
    assert!(w.topup_events().is_empty());
    assert_eq!(client.get_subscription(&w.id).prepaid_balance, Some(1));
    assert_eq!(w.balance(&w.subscriber), MINTED - 1);
}

#[test]
fn without_auto_topup_a_shortfall_still_lapses_to_past_due() {
    for disabled_after_configuring in [false, true] {
        let w = setup();
        let client = w.client();
        client.deposit(&w.id, &100);
        if disabled_after_configuring {
            client.configure_auto_topup(&w.id, &0, &500);
            client.disable_auto_topup(&w.id);
        }
        let before = w.balances();

        w.advance_periods(1);
        assert_eq!(client.charge(&w.id), 0);
        assert!(w.topup_events().is_empty());
        let subscription = client.get_subscription(&w.id);
        assert_eq!(subscription.status, SubscriptionStatus::PastDue);
        assert_eq!(subscription.failed_attempts, 1);
        assert_eq!(subscription.last_charged, START);
        assert_eq!(subscription.prepaid_balance, Some(100));
        assert_eq!(w.balances(), before, "no token moves without auto-topup");
    }
}

#[test]
fn pull_mode_billing_never_tops_up() {
    let w = setup();
    let client = w.client();
    client.configure_auto_topup(&w.id, &1_000, &500);

    w.advance_periods(1);
    assert_eq!(client.charge(&w.id), AMOUNT);
    assert!(w.topup_events().is_empty());
    assert_eq!(client.get_subscription(&w.id).prepaid_balance, None);
    assert_eq!(w.balances(), (MINTED - AMOUNT, AMOUNT, 0));
}

// -----------------------------------------------------------------------
// Fallback when the top-up transfer fails
// -----------------------------------------------------------------------

/// Assert a charge whose top-up was rejected left the subscription in
/// `PastDue` with nothing moved and nothing but the arrears bookkeeping
/// written.
fn assert_rejected_topup_falls_back_to_past_due(w: &World, balance: i128) {
    let client = w.client();
    let before_balances = w.balances();
    let before = client.get_subscription(&w.id);

    w.advance_periods(1);
    assert_eq!(client.charge(&w.id), 0);
    assert!(
        w.topup_events().is_empty(),
        "a rejected top-up emits nothing"
    );

    let after = client.get_subscription(&w.id);
    assert_eq!(after.status, SubscriptionStatus::PastDue);
    assert_eq!(after.failed_attempts, before.failed_attempts + 1);
    assert_eq!(after.last_charged, before.last_charged);
    assert_eq!(after.prepaid_balance, Some(balance));
    assert_eq!(w.balances(), before_balances, "no token moves");
    assert!(
        client.get_auto_topup(&w.id).is_some(),
        "the setting survives a failed top-up"
    );
}

#[test]
fn topup_larger_than_the_subscriber_balance_falls_back_to_past_due() {
    let w = setup();
    let client = w.client();
    client.deposit(&w.id, &100);
    client.configure_auto_topup(&w.id, &0, &(MINTED * 10));
    assert_rejected_topup_falls_back_to_past_due(&w, 100);
}

#[test]
fn deauthorized_subscriber_falls_back_to_past_due_and_recovers_once_reauthorized() {
    let w = setup();
    let client = w.client();
    client.deposit(&w.id, &100);
    client.configure_auto_topup(&w.id, &0, &500);
    let sac = StellarAssetClient::new(&w.env, &w.token);
    sac.set_authorized(&w.subscriber, &false);
    assert_rejected_topup_falls_back_to_past_due(&w, 100);

    // Once the account is authorized again, the retry tops up and settles.
    sac.set_authorized(&w.subscriber, &true);
    assert_eq!(client.charge(&w.id), AMOUNT);
    assert_eq!(w.topup_events(), std::vec![(sc_i128(500), sc_i128(600))]);
    let subscription = client.get_subscription(&w.id);
    assert_eq!(subscription.status, SubscriptionStatus::Active);
    assert_eq!(subscription.failed_attempts, 0);
    assert_eq!(subscription.prepaid_balance, Some(600 - AMOUNT));
}

#[test]
fn rejected_threshold_topup_still_debits_a_covering_balance() {
    let w = setup();
    let client = w.client();
    client.deposit(&w.id, &300);
    client.configure_auto_topup(&w.id, &1_000, &500);
    StellarAssetClient::new(&w.env, &w.token).set_authorized(&w.subscriber, &false);

    w.advance_periods(1);
    assert_eq!(client.charge(&w.id), AMOUNT);
    assert!(w.topup_events().is_empty());
    let subscription = client.get_subscription(&w.id);
    assert_eq!(subscription.status, SubscriptionStatus::Active);
    assert_eq!(subscription.prepaid_balance, Some(300 - AMOUNT));
    assert_eq!(w.balances(), (MINTED - 300, AMOUNT, 300 - AMOUNT));
}

#[test]
fn topup_that_cannot_cover_the_period_is_credited_and_refunded_on_auto_cancel() {
    let w = setup();
    let client = w.client();
    client.configure_auto_topup(&w.id, &0, &50);
    client.deposit(&w.id, &1);

    // Each retry tops up 50 but never reaches 250: PastDue, then Cancelled
    // after the third failure, refunding every topped-up token.
    w.advance_periods(1);
    for attempt in 1..=3_u32 {
        assert_eq!(client.charge(&w.id), 0);
        assert_eq!(
            w.topup_events(),
            std::vec![(sc_i128(50), sc_i128(1 + 50 * i128::from(attempt)))]
        );
        let subscription = client.get_subscription(&w.id);
        assert_eq!(subscription.failed_attempts, attempt);
        if attempt < 3 {
            assert_eq!(subscription.status, SubscriptionStatus::PastDue);
            assert_eq!(
                subscription.prepaid_balance,
                Some(1 + 50 * i128::from(attempt))
            );
        } else {
            assert_eq!(subscription.status, SubscriptionStatus::Cancelled);
            assert_eq!(subscription.prepaid_balance, Some(0));
        }
    }
    assert_eq!(
        w.balances(),
        (MINTED, 0, 0),
        "auto-cancel returns the deposit and every top-up"
    );
}
