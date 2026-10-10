use super::*;
use soroban_forge_subscription_payments::{
    MetricQuota, SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus,
};
use soroban_forge_vesting::{SorobanForgeVestingClient, Vesting, VestingStatus};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, Symbol, Vec};

const START: u64 = 1_000_000;
const CLIFF: u64 = 0;
const DURATION: u64 = 6_000;
const PERIOD: u64 = 1_000;
const AMOUNT: i128 = 500;

fn setup_vesting_subscription() -> (
    Env,
    Address,
    StellarAssetClient<'static>,
    SorobanForgeVestingClient<'static>,
    SorobanForgeSubscriptionPaymentsClient<'static>,
    soroban_forge_test_utils::TestAccounts,
) {
    let (env, accounts) = setup_env();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(START);

    let token_admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(token_admin);
    let token = sac.address();
    let token_client = StellarAssetClient::new(&env, &token);

    let vesting_id = env.register(Vesting, ());
    let vesting_client = SorobanForgeVestingClient::new(&env, &vesting_id);

    let sub_id = env.register(SubscriptionPayments, ());
    let sub_client = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_id);

    (
        env,
        token,
        token_client,
        vesting_client,
        sub_client,
        accounts,
    )
}

#[test]
fn vesting_claims_stream_into_pull_subscription_charges() {
    let (env, token, token_client, vesting_client, sub_client, accounts) =
        setup_vesting_subscription();

    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    // 1. Create a linear vesting schedule funded with 12_000 tokens for subscriber.
    let total_vesting = 12_000_i128;
    token_client.mint(&vesting_client.address, &total_vesting);
    let schedule_id = vesting_client.create_schedule(
        &accounts.deployer,
        subscriber,
        &token,
        &total_vesting,
        &CLIFF,
        &DURATION,
    );
    assert_eq!(
        vesting_client.get_status(&schedule_id),
        VestingStatus::Vesting
    );

    // 2. Subscriber subscribes to provider service (500 tokens per 1_000 seconds).
    let subscription_id = sub_client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);
    assert_eq!(
        sub_client.get_subscription(&subscription_id).status,
        SubscriptionStatus::Active
    );
    assert_eq!(token_client.balance(subscriber), 0);

    // 3. Advance to 2_000s past start (1_000s past cliff). 1/3 of the schedule is vested (4_000 tokens).
    env.ledger().set_timestamp(START + 2_000);
    assert_eq!(
        vesting_client.get_status(&schedule_id),
        VestingStatus::Vesting
    );
    let claimable = vesting_client.claimable(&schedule_id);
    assert_eq!(claimable, 4_000);

    // 4. Beneficiary claims vested tokens.
    let claimed = vesting_client.claim(&schedule_id);
    assert_eq!(claimed, 4_000);
    assert_eq!(token_client.balance(subscriber), 4_000);

    // 5. Provider bills due periods using pull charges.
    let billed = sub_client.charge(&subscription_id);
    assert_eq!(billed, AMOUNT);
    assert_eq!(token_client.balance(subscriber), 3_500);
    assert_eq!(token_client.balance(provider), 500);

    // 6. Advance another period (to t = 3_000s). Claim next vested slice (halfway vested = 6_000 total).
    env.ledger().set_timestamp(START + 3_000);
    let second_claim = vesting_client.claim(&schedule_id);
    assert_eq!(second_claim, 2_000);
    assert_eq!(token_client.balance(subscriber), 5_500);

    // 7. Provider charges second period.
    let billed_2 = sub_client.charge(&subscription_id);
    assert_eq!(billed_2, AMOUNT);
    assert_eq!(token_client.balance(subscriber), 5_000);
    assert_eq!(token_client.balance(provider), 1_000);

    // 8. Advance to full duration (t = 6_000s) and claim final vesting remainder.
    env.ledger().set_timestamp(START + DURATION);
    let final_claim = vesting_client.claim(&schedule_id);
    assert_eq!(final_claim, 6_000);
    assert_eq!(
        vesting_client.get_status(&schedule_id),
        VestingStatus::Completed
    );

    // 9. Verify full system token conservation:
    // 12_000 initial tokens = Vesting contract (0) + Subscriber (11_000) + Provider (1_000).
    assert_eq!(token_client.balance(&vesting_client.address), 0);
    assert_eq!(token_client.balance(subscriber), 11_000);
    assert_eq!(token_client.balance(provider), 1_000);
}

#[test]
fn vesting_claims_fund_prepaid_subscription_with_usage_overage_and_refund() {
    let (env, token, token_client, vesting_client, sub_client, accounts) =
        setup_vesting_subscription();

    let subscriber = &accounts.user1;
    let provider = &accounts.user2;

    // 1. Set up vesting schedule for 10_000 tokens.
    let total_vesting = 10_000_i128;
    token_client.mint(&vesting_client.address, &total_vesting);
    let schedule_id = vesting_client.create_schedule(
        &accounts.deployer,
        subscriber,
        &token,
        &total_vesting,
        &CLIFF,
        &DURATION,
    );

    // 2. Set up subscription with metered usage quota.
    let subscription_id = sub_client.subscribe(subscriber, provider, &token, &AMOUNT, &PERIOD);
    let metric = Symbol::new(&env, "api_calls");
    let mut quotas = Vec::new(&env);
    quotas.push_back(MetricQuota {
        metric: metric.clone(),
        included_units: 100,
        overage_price: 5,
        bucket_units: 10,
        max_overage_units: None,
    });
    sub_client.set_quotas(&subscription_id, &quotas);

    // 3. Advance to 3_000s (halfway) and claim 5_000 tokens.
    env.ledger().set_timestamp(START + 3_000);
    let claimed = vesting_client.claim(&schedule_id);
    assert_eq!(claimed, 5_000);
    assert_eq!(token_client.balance(subscriber), 5_000);

    // 4. Subscriber deposits 3_000 tokens into prepaid subscription balance.
    let prepaid_deposit = 3_000_i128;
    let new_balance = sub_client.deposit(&subscription_id, &prepaid_deposit);
    assert_eq!(new_balance, prepaid_deposit);
    assert_eq!(token_client.balance(subscriber), 2_000);
    assert_eq!(token_client.balance(&sub_client.address), 3_000);

    // 5. Provider records 150 units of usage (50 overage units = 5 buckets * 5 = 25 overage tokens).
    sub_client.record_usage(&subscription_id, &metric, &150);
    assert_eq!(sub_client.get_usage(&subscription_id, &metric).units, 150);
    assert_eq!(sub_client.quote_period(&subscription_id), AMOUNT + 25);

    // 6. Provider charges the open period. Billed amount = 525 debited from prepaid custody.
    let billed = sub_client.charge(&subscription_id);
    assert_eq!(billed, 525);
    assert_eq!(
        sub_client
            .get_subscription(&subscription_id)
            .prepaid_balance,
        Some(2_475)
    );
    assert_eq!(token_client.balance(provider), 525);

    // 7. Subscriber cancels the subscription; remaining prepaid balance (2_475) is refunded.
    sub_client.cancel(&subscription_id);
    assert_eq!(
        sub_client.get_subscription(&subscription_id).status,
        SubscriptionStatus::Cancelled
    );
    assert_eq!(
        sub_client
            .get_subscription(&subscription_id)
            .prepaid_balance,
        Some(0)
    );
    assert_eq!(token_client.balance(&sub_client.address), 0);

    // 8. Verify subscriber received exact refund (2_000 uncommitted + 2_475 refund = 4_475).
    assert_eq!(token_client.balance(subscriber), 4_475);
    assert_eq!(token_client.balance(provider), 525);
    assert_eq!(token_client.balance(&vesting_client.address), 5_000);

    // Total conservation: 4_475 + 525 + 5_000 = 10_000 tokens!
    let total_held = token_client.balance(subscriber)
        + token_client.balance(provider)
        + token_client.balance(&vesting_client.address)
        + token_client.balance(&sub_client.address);
    assert_eq!(total_held, total_vesting);
}
