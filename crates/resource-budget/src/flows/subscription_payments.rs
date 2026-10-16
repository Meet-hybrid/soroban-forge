//! Subscription payments flow: a subscriber opens a prepaid subscription and
//! the provider charges the first period — the contract's primary billing
//! path through SEP-41 settlement.

use soroban_forge_subscription_payments::{
    SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments,
};
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env};

use crate::metrics::ResourceSample;

const START: u64 = 1_000_000;
const PERIOD: u64 = 1_000;
const AMOUNT: i128 = 250;
const FUNDS: i128 = 10_000_000;

/// Runs the flow and captures the resources of the `charge` invocation.
pub(crate) fn run() -> ResourceSample {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let token_admin = StellarAssetClient::new(&env, &token);

    let contract_id = env.register(SubscriptionPayments, ());
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let accounts = TestAccounts::generate(&env);
    token_admin.mint(&accounts.user1, &FUNDS);

    let subscription_id = client.subscribe(
        &accounts.user1,
        &accounts.validator,
        &token,
        &AMOUNT,
        &PERIOD,
    );
    // Charges settle from the prepaid balance: fund one period up front so
    // the measured charge actually bills instead of marking the
    // subscription PastDue with a zero transfer.
    client.deposit(&subscription_id, &AMOUNT);

    // One full period elapsed: the charge is due.
    env.ledger().set_timestamp(START + PERIOD);

    // The measured entrypoint.
    let charged = client.charge(&subscription_id);
    let sample = ResourceSample::capture(&env);

    assert!(charged > 0, "first charge must bill a positive amount");
    sample
}
