//! Vesting + Subscription payments composition settlement scenarios.
//!
//! Exercises cross-contract flows between [`Vesting`] and
//! [`SubscriptionPayments`] backed by a real SEP-41 [`TokenFixture`] and
//! verified against an independent [`BalanceMirror`]:
//! - Linear vesting schedules (`create_schedule`, `claim`) funding both direct
//!   recurring charges (`charge`, `charge_catchup`) and prepaid balance
//!   subscriptions (`deposit`, `charge`).
//! - Discrete tranche unlock schedules (`create_tranche_schedule`) funding
//!   metered usage subscriptions (`set_quotas`, `record_usage`, `charge`).
//! - Mid-stream beneficiary reassignment (`reassign_beneficiary`, `claim_for`)
//!   splitting vested payouts across former and current subscribers.
//! - Cross-contract authorization enforcement (`env.auths()` and negative
//!   `mock_auths` / `set_auths`).
//! - Failure isolation and rollback when vesting custody is underfunded or when
//!   subscription charges run before vesting claims settle.
//! - Deterministic multi-seed conservation runs across [`DEFAULT_COMPOSITION_SEEDS`].

use super::*;
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_subscription_payments::{
    MetricQuota, SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus,
};
use soroban_forge_vesting::{SorobanForgeVestingClient, Tranche, Vesting, VestingStatus};
use soroban_sdk::testutils::{Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, InvokeError, Symbol, Vec};

/// Reusable setup fixture for Vesting + Subscription Payments composition tests.
pub struct VestingSubscriptionFixture {
    pub env: Env,
    pub accounts: TestAccounts,
    pub token: TokenFixture,
    pub vesting_id: Address,
    pub subscriptions_id: Address,
    pub mirror: BalanceMirror,
}

impl VestingSubscriptionFixture {
    /// Deploy and initialize the vesting + subscription composition environment.
    pub fn new() -> Self {
        let (env, accounts, token) = setup_env();
        let vesting_id = env.register(Vesting, ());
        let subscriptions_id = env.register(SubscriptionPayments, ());

        let participants = [
            accounts.deployer.clone(),
            accounts.user1.clone(),
            accounts.user2.clone(),
            accounts.user3.clone(),
            accounts.validator.clone(),
            accounts.arbiter.clone(),
            vesting_id.clone(),
            subscriptions_id.clone(),
        ];
        let mirror = BalanceMirror::new(&env, &token.address, &participants);

        Self {
            env,
            accounts,
            token,
            vesting_id,
            subscriptions_id,
            mirror,
        }
    }

    pub fn vesting(&self) -> SorobanForgeVestingClient<'_> {
        SorobanForgeVestingClient::new(&self.env, &self.vesting_id)
    }

    pub fn subscriptions(&self) -> SorobanForgeSubscriptionPaymentsClient<'_> {
        SorobanForgeSubscriptionPaymentsClient::new(&self.env, &self.subscriptions_id)
    }
}

#[test]
fn linear_vesting_claims_fund_direct_and_prepaid_subscriptions() {
    let mut fx = VestingSubscriptionFixture::new();
    let vesting = fx.vesting();
    let subscriptions = fx.subscriptions();

    let funder = fx.accounts.user1.clone();
    let beneficiary = fx.accounts.user2.clone();
    let provider = fx.accounts.user3.clone();

    // 1. Funder mints and transfers 10,000 tokens into Vesting custody.
    fx.token.mint(&fx.env, &funder, 10_000);
    fx.mirror.record_mint(&funder, 10_000);
    fx.token.transfer(&fx.env, &funder, &fx.vesting_id, 10_000);
    fx.mirror.record_transfer(&funder, &fx.vesting_id, 10_000);
    fx.mirror.assert_conserved(&fx.env);

    // 2. Create a linear vesting schedule: 10,000 total, 1,000s cliff, 4,000s duration.
    let schedule_id = vesting.create_schedule(
        &funder,
        &beneficiary,
        &fx.token.address,
        &10_000,
        &1_000,
        &4_000,
    );
    assert_auth_invocation(&fx.env, &beneficiary, &fx.vesting_id, "create_schedule");
    assert_eq!(vesting.claimable(&schedule_id), 0);

    // 3. Beneficiary creates two subscriptions with `provider`:
    //    - `direct_sub`: 1,500 per 1,000s period (direct pull from subscriber)
    //    - `prepaid_sub`: 1,000 per 1,000s period (funded via prepaid deposit)
    let direct_sub =
        subscriptions.subscribe(&beneficiary, &provider, &fx.token.address, &1_500, &1_000);
    assert_auth_invocation(&fx.env, &beneficiary, &fx.subscriptions_id, "subscribe");
    let prepaid_sub =
        subscriptions.subscribe(&beneficiary, &provider, &fx.token.address, &1_000, &1_000);

    // 4. Advance 2,000s (50% vested = 5,000 tokens) and claim from Vesting.
    let start = fx.env.ledger().timestamp();
    fx.env.ledger().set_timestamp(start + 2_000);
    assert_eq!(vesting.claimable(&schedule_id), 5_000);

    let claimed_1 = vesting.claim(&schedule_id);
    assert_eq!(claimed_1, 5_000);
    assert_sac_transfer_event(
        &fx.env,
        &fx.token.address,
        &fx.vesting_id,
        &beneficiary,
        5_000,
    );
    assert_auth_invocation(&fx.env, &beneficiary, &fx.vesting_id, "claim");
    assert_eq!(
        vesting.get_schedule(&schedule_id).status,
        VestingStatus::Vesting
    );
    fx.mirror
        .record_transfer(&fx.vesting_id, &beneficiary, 5_000);
    fx.mirror.assert_conserved(&fx.env);

    // 5. Beneficiary deposits 2,000 of the claimed tokens into `prepaid_sub`.
    let prepaid_bal = subscriptions.deposit(&prepaid_sub, &2_000);
    assert_eq!(prepaid_bal, 2_000);
    assert_contract_event(&fx.env, &fx.subscriptions_id, "deposited");
    assert_sac_transfer_event(
        &fx.env,
        &fx.token.address,
        &beneficiary,
        &fx.subscriptions_id,
        2_000,
    );
    fx.mirror
        .record_transfer(&beneficiary, &fx.subscriptions_id, 2_000);
    fx.mirror.assert_conserved(&fx.env);

    // 6. Provider charges one period on `prepaid_sub` (1,000 debited from prepaid custody)
    //    and catches up 2 elapsed periods on `direct_sub` (3,000 pulled from beneficiary).
    let billed_prepaid = subscriptions.charge(&prepaid_sub);
    assert_eq!(billed_prepaid, 1_000);
    assert_contract_event(&fx.env, &fx.subscriptions_id, "balance_debited");
    assert_contract_event(&fx.env, &fx.subscriptions_id, "charged");
    assert_sac_transfer_event(
        &fx.env,
        &fx.token.address,
        &fx.subscriptions_id,
        &provider,
        1_000,
    );
    fx.mirror
        .record_transfer(&fx.subscriptions_id, &provider, 1_000);
    fx.mirror.assert_conserved(&fx.env);

    let billed_catchup = subscriptions.charge_catchup(&direct_sub, &2);
    assert_eq!(billed_catchup, 3_000);
    assert_eq!(
        contract_event_count(&fx.env, &fx.subscriptions_id, "charged"),
        2
    );
    fx.mirror.record_transfer(&beneficiary, &provider, 3_000);
    fx.mirror.assert_conserved(&fx.env);

    // 7. Advance to full vesting (start + 4,000s), claim final 5,000, and charge remaining periods.
    fx.env.ledger().set_timestamp(start + 4_000);
    let claimed_2 = vesting.claim(&schedule_id);
    assert_eq!(claimed_2, 5_000);
    assert_eq!(
        vesting.get_schedule(&schedule_id).status,
        VestingStatus::Completed
    );
    fx.mirror
        .record_transfer(&fx.vesting_id, &beneficiary, 5_000);
    fx.mirror.assert_conserved(&fx.env);

    let billed_final = subscriptions.charge_catchup(&direct_sub, &2);
    assert_eq!(billed_final, 3_000);
    fx.mirror.record_transfer(&beneficiary, &provider, 3_000);
    fx.mirror.assert_conserved(&fx.env);
}

#[test]
fn tranche_vesting_unlocks_fund_metered_usage_subscription() {
    let mut fx = VestingSubscriptionFixture::new();
    let vesting = fx.vesting();
    let subscriptions = fx.subscriptions();

    let funder = fx.accounts.user1.clone();
    let beneficiary = fx.accounts.user2.clone();
    let provider = fx.accounts.user3.clone();

    fx.token.mint(&fx.env, &funder, 6_000);
    fx.mirror.record_mint(&funder, 6_000);
    fx.token.transfer(&fx.env, &funder, &fx.vesting_id, 6_000);
    fx.mirror.record_transfer(&funder, &fx.vesting_id, 6_000);

    // Tranche schedule: 2,500 unlocks at +1,000s, 3,500 unlocks at +2,000s.
    let mut tranches = Vec::new(&fx.env);
    tranches.push_back(Tranche {
        unlock_at: 1_000,
        amount: 2_500,
    });
    tranches.push_back(Tranche {
        unlock_at: 2_000,
        amount: 3_500,
    });
    let tranche_id = vesting.create_tranche_schedule(&beneficiary, &fx.token.address, &tranches);

    // Metered subscription: base 1,000 per 1,000s + compute quota (100 included,
    // 250 per 50-unit overage bucket, capped at 200 overage units).
    let sub_id =
        subscriptions.subscribe(&beneficiary, &provider, &fx.token.address, &1_000, &1_000);
    let metric = Symbol::new(&fx.env, "compute");
    let mut quotas = Vec::new(&fx.env);
    quotas.push_back(MetricQuota {
        metric: metric.clone(),
        included_units: 100,
        overage_price: 250,
        bucket_units: 50,
        max_overage_units: Some(200),
    });
    subscriptions.set_quotas(&sub_id, &quotas);

    // Record 175 units in period 1 (75 overage units -> 2 buckets * 250 = 500 overage).
    let usage = subscriptions.record_usage(&sub_id, &metric, &175);
    assert_eq!(usage.units, 175);
    assert_contract_event(&fx.env, &fx.subscriptions_id, "usage_recorded");

    // Advance +1,000s so tranche 1 unlocks and period 1 becomes due.
    let start = fx.env.ledger().timestamp();
    fx.env.ledger().set_timestamp(start + 1_000);

    let claimed = vesting.claim(&tranche_id);
    assert_eq!(claimed, 2_500);
    fx.mirror
        .record_transfer(&fx.vesting_id, &beneficiary, 2_500);
    fx.mirror.assert_conserved(&fx.env);

    // Charge period 1: base 1,000 + overage 500 = 1,500.
    let billed = subscriptions.charge(&sub_id);
    assert_eq!(billed, 1_500);
    assert_contract_event(&fx.env, &fx.subscriptions_id, "charged");
    assert_sac_transfer_event(&fx.env, &fx.token.address, &beneficiary, &provider, 1_500);
    fx.mirror.record_transfer(&beneficiary, &provider, 1_500);
    fx.mirror.assert_conserved(&fx.env);
}

#[test]
fn beneficiary_reassignment_splits_vested_claims_across_two_subscribers() {
    let mut fx = VestingSubscriptionFixture::new();
    let vesting = fx.vesting();
    let subscriptions = fx.subscriptions();

    let funder = fx.accounts.user1.clone();
    let old_beneficiary = fx.accounts.user2.clone();
    let new_beneficiary = fx.accounts.validator.clone();
    let provider = fx.accounts.user3.clone();

    fx.token.mint(&fx.env, &funder, 8_000);
    fx.mirror.record_mint(&funder, 8_000);
    fx.token.transfer(&fx.env, &funder, &fx.vesting_id, 8_000);
    fx.mirror.record_transfer(&funder, &fx.vesting_id, 8_000);

    let schedule_id = vesting.create_schedule(
        &funder,
        &old_beneficiary,
        &fx.token.address,
        &8_000,
        &1_000,
        &4_000,
    );

    let sub_old = subscriptions.subscribe(
        &old_beneficiary,
        &provider,
        &fx.token.address,
        &1_500,
        &1_000,
    );
    let sub_new = subscriptions.subscribe(
        &new_beneficiary,
        &provider,
        &fx.token.address,
        &2_000,
        &1_000,
    );

    // Advance 1,000s (25% = 2,000 vested for `old_beneficiary`), then reassign to `new_beneficiary`.
    let start = fx.env.ledger().timestamp();
    fx.env.ledger().set_timestamp(start + 1_000);

    vesting.reassign_beneficiary(&schedule_id, &new_beneficiary);
    assert_contract_event(&fx.env, &fx.vesting_id, "beneficiary_reassigned_from");
    assert_contract_event(&fx.env, &fx.vesting_id, "beneficiary_reassigned_to");
    assert_auth_invocation(&fx.env, &funder, &fx.vesting_id, "reassign_beneficiary");

    // Advance to full completion (start + 4,000s).
    fx.env.ledger().set_timestamp(start + 4_000);

    // Former beneficiary claims their frozen 2,000 via `claim_for` and pays `sub_old`.
    let old_claimed = vesting.claim_for(&schedule_id, &old_beneficiary);
    assert_eq!(old_claimed, 2_000);
    fx.mirror
        .record_transfer(&fx.vesting_id, &old_beneficiary, 2_000);
    let old_billed = subscriptions.charge(&sub_old);
    assert_eq!(old_billed, 1_500);
    fx.mirror
        .record_transfer(&old_beneficiary, &provider, 1_500);

    // New beneficiary claims the remaining 6,000 via `claim` and pays `sub_new`.
    let new_claimed = vesting.claim(&schedule_id);
    assert_eq!(new_claimed, 6_000);
    fx.mirror
        .record_transfer(&fx.vesting_id, &new_beneficiary, 6_000);
    let new_billed = subscriptions.charge(&sub_new);
    assert_eq!(new_billed, 2_000);
    fx.mirror
        .record_transfer(&new_beneficiary, &provider, 2_000);

    fx.mirror.assert_conserved(&fx.env);
}

#[test]
fn cross_contract_authorization_enforced_across_vesting_and_subscription() {
    let mut fx = VestingSubscriptionFixture::new();
    let vesting = fx.vesting();
    let subscriptions = fx.subscriptions();

    let funder = fx.accounts.user1.clone();
    let beneficiary = fx.accounts.user2.clone();
    let provider = fx.accounts.user3.clone();

    fx.token.mint(&fx.env, &funder, 4_000);
    fx.mirror.record_mint(&funder, 4_000);
    fx.token.transfer(&fx.env, &funder, &fx.vesting_id, 4_000);
    fx.mirror.record_transfer(&funder, &fx.vesting_id, 4_000);

    let schedule_id = vesting.create_schedule(
        &funder,
        &beneficiary,
        &fx.token.address,
        &4_000,
        &500,
        &2_000,
    );
    let sub_id = subscriptions.subscribe(&beneficiary, &provider, &fx.token.address, &500, &500);

    let start = fx.env.ledger().timestamp();
    fx.env.ledger().set_timestamp(start + 1_000);

    // 1. Non-beneficiary (`provider`) cannot authorize `vesting.claim`.
    let snap = fx.mirror.snapshot();
    fx.env.mock_auths(&[MockAuth {
        address: &provider,
        invoke: &MockAuthInvoke {
            contract: &fx.vesting_id,
            fn_name: "claim",
            args: (schedule_id,).into_val(&fx.env),
            sub_invokes: &[],
        },
    }]);
    let res = vesting.try_claim(&schedule_id);
    assert!(matches!(res, Err(Err(InvokeError::Abort))));
    fx.env.mock_all_auths();
    assert_eq!(vesting.get_schedule(&schedule_id).claimed, 0);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // 2. Non-provider (`beneficiary`) cannot authorize `subscriptions.charge`.
    fx.env.mock_auths(&[MockAuth {
        address: &beneficiary,
        invoke: &MockAuthInvoke {
            contract: &fx.subscriptions_id,
            fn_name: "charge",
            args: (sub_id,).into_val(&fx.env),
            sub_invokes: &[],
        },
    }]);
    let res = subscriptions.try_charge(&sub_id);
    assert!(matches!(res, Err(Err(InvokeError::Abort))));
    fx.env.mock_all_auths();
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // 3. Provider cannot call `subscribe_on_behalf_of` without subscriber opt-in.
    let unauth_err = subscriptions
        .try_subscribe_on_behalf_of(&provider, &beneficiary, &fx.token.address, &500, &500)
        .unwrap_err()
        .unwrap();
    assert_eq!(unauth_err, ForgeError::Unauthorized);
    fx.mirror.assert_unchanged(&fx.env, &snap);
}

#[test]
fn underfunded_vesting_and_unclaimed_schedule_isolate_downstream_subscription_failures() {
    let mut fx = VestingSubscriptionFixture::new();
    let vesting = fx.vesting();
    let subscriptions = fx.subscriptions();

    let funder = fx.accounts.user1.clone();
    let beneficiary = fx.accounts.user2.clone();
    let provider = fx.accounts.user3.clone();

    // Create a 3,000-token schedule BEFORE funding `vesting_id`.
    let schedule_id = vesting.create_schedule(
        &funder,
        &beneficiary,
        &fx.token.address,
        &3_000,
        &500,
        &1_500,
    );
    let sub_id = subscriptions.subscribe(&beneficiary, &provider, &fx.token.address, &1_000, &500);

    // Before cliff (+200s): `claim` returns 0 and moves no tokens.
    let start = fx.env.ledger().timestamp();
    fx.env.ledger().set_timestamp(start + 200);
    assert_eq!(vesting.claim(&schedule_id), 0);

    // Advance past full vesting (+1,500s) while `vesting_id` still has 0 tokens.
    fx.env.ledger().set_timestamp(start + 1_500);
    let snap = fx.mirror.snapshot();
    let claim_err = vesting.try_claim(&schedule_id).unwrap_err().unwrap();
    assert_eq!(claim_err, ForgeError::TokenTransferFailed);
    assert_eq!(vesting.get_schedule(&schedule_id).claimed, 0);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // Because `vesting.claim` failed, `beneficiary` still has 0 tokens:
    // 1) `charge_catchup` fails with `TokenTransferFailed` and leaves `last_charged` unchanged.
    let before_sub = subscriptions.get_subscription(&sub_id);
    let catchup_err = subscriptions
        .try_charge_catchup(&sub_id, &2)
        .unwrap_err()
        .unwrap();
    assert_eq!(catchup_err, ForgeError::TokenTransferFailed);
    assert_eq!(subscriptions.get_subscription(&sub_id), before_sub);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // 2) Single-period `charge` transitions subscription to `PastDue` (`failed_attempts == 1`)
    //    without transferring any tokens.
    let billed_zero = subscriptions.charge(&sub_id);
    assert_eq!(billed_zero, 0);
    let past_due_sub = subscriptions.get_subscription(&sub_id);
    assert_eq!(past_due_sub.status, SubscriptionStatus::PastDue);
    assert_eq!(past_due_sub.failed_attempts, 1);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // Now fund `vesting_id`, claim the 3,000 tokens, resume the subscription, and settle!
    fx.token.mint(&fx.env, &funder, 3_000);
    fx.mirror.record_mint(&funder, 3_000);
    fx.token.transfer(&fx.env, &funder, &fx.vesting_id, 3_000);
    fx.mirror.record_transfer(&funder, &fx.vesting_id, 3_000);

    let claimed = vesting.claim(&schedule_id);
    assert_eq!(claimed, 3_000);
    fx.mirror
        .record_transfer(&fx.vesting_id, &beneficiary, 3_000);
    fx.mirror.assert_conserved(&fx.env);

    subscriptions.resume(&sub_id);
    fx.env.ledger().set_timestamp(start + 2_000);
    let billed_after_resume = subscriptions.charge(&sub_id);
    assert_eq!(billed_after_resume, 1_000);
    fx.mirror.record_transfer(&beneficiary, &provider, 1_000);
    fx.mirror.assert_conserved(&fx.env);
}

#[test]
fn vesting_subscription_conservation_holds_across_deterministic_seeds() {
    for seed in DEFAULT_COMPOSITION_SEEDS {
        let mut rng = SeededRng::new(seed);
        let mut fx = VestingSubscriptionFixture::new();
        let vesting = fx.vesting();
        let subscriptions = fx.subscriptions();

        let funder = fx.accounts.user1.clone();
        let beneficiary = fx.accounts.user2.clone();
        let provider = fx.accounts.user3.clone();

        let period = rng.range_u64(300, 1_200);
        let periods_count = u64::from(rng.range_u32(2, 5));
        let duration = period * periods_count;
        let cliff = period;

        let period_price = rng.range_i128(200, 2_000);
        let total_vesting = period_price * i128::from(periods_count) + rng.range_i128(500, 5_000);
        let use_prepaid = rng.choose_bool();

        fx.token.mint(&fx.env, &funder, total_vesting);
        fx.mirror.record_mint(&funder, total_vesting);
        fx.token
            .transfer(&fx.env, &funder, &fx.vesting_id, total_vesting);
        fx.mirror
            .record_transfer(&funder, &fx.vesting_id, total_vesting);
        fx.mirror.assert_conserved(&fx.env);

        let schedule_id = vesting.create_schedule(
            &funder,
            &beneficiary,
            &fx.token.address,
            &total_vesting,
            &cliff,
            &duration,
        );
        let sub_id = subscriptions.subscribe(
            &beneficiary,
            &provider,
            &fx.token.address,
            &period_price,
            &period,
        );

        let start = fx.env.ledger().timestamp();
        for step in 1..=periods_count {
            fx.env.ledger().set_timestamp(start + step * period);

            let newly_claimed = vesting.claim(&schedule_id);
            assert!(newly_claimed > 0);
            fx.mirror
                .record_transfer(&fx.vesting_id, &beneficiary, newly_claimed);
            fx.mirror.assert_conserved(&fx.env);

            if use_prepaid {
                subscriptions.deposit(&sub_id, &period_price);
                fx.mirror
                    .record_transfer(&beneficiary, &fx.subscriptions_id, period_price);
                fx.mirror.assert_conserved(&fx.env);

                let billed = subscriptions.charge(&sub_id);
                assert_eq!(billed, period_price);
                fx.mirror
                    .record_transfer(&fx.subscriptions_id, &provider, period_price);
            } else {
                let billed = subscriptions.charge(&sub_id);
                assert_eq!(billed, period_price);
                fx.mirror
                    .record_transfer(&beneficiary, &provider, period_price);
            }

            fx.mirror.assert_conserved(&fx.env);
        }

        assert_eq!(
            vesting.get_schedule(&schedule_id).status,
            VestingStatus::Completed
        );
    }
}
